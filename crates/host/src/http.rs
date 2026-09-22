use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::Router;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use domain::{ContentHash, FunctionId, InvokeRequest, VersionLabel, MAX_INVOKE_BODY_BYTES};
use runtime::{decode_response, encode_request, Request as FnRequest};
use tower_http::trace::TraceLayer;

use crate::error::HttpError;
use crate::state::AppState;

pub const SHASUM_HEADER: &str = "x-nitrum-fn-shasum";
pub const NONCE_HEADER: &str = "x-nitrum-fn-nonce";
pub const ATTESTATION_HEADER: &str = "x-nitrum-fn-attestation";

pub fn router(state: AppState) -> Router {
    telemetry::http::instrument_router(
        Router::new()
            .route("/healthz", get(|| async { StatusCode::OK }))
            .route("/invoke/{name}", post(invoke))
            .layer(DefaultBodyLimit::max(MAX_INVOKE_BODY_BYTES))
            .layer(TraceLayer::new_for_http())
            .with_state(state),
    )
}

async fn invoke(
    State(state): State<AppState>,
    Path(name): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<impl IntoResponse, HttpError> {
    let function = FunctionId::new(&name).map_err(application::AppError::from)?;
    let version = match headers
        .get("x-nitrum-fn-version")
        .and_then(|v| v.to_str().ok())
    {
        Some(raw) => VersionLabel::new(raw).map_err(application::AppError::from)?,
        None => VersionLabel::latest(),
    };

    let nonce = parse_nonce(&headers)?;

    let path = format!("/invoke/{name}");
    let fn_headers = headers
        .iter()
        .filter_map(|(k, v)| {
            let value = v.to_str().ok()?.to_string();
            Some((k.as_str().to_string(), value))
        })
        .collect();

    if body.len() > MAX_INVOKE_BODY_BYTES {
        return Err(application::AppError::PayloadTooLarge(format!(
            "invoke body {} bytes exceeds max {MAX_INVOKE_BODY_BYTES}",
            body.len()
        ))
        .into());
    }

    let fn_req = FnRequest::new("POST", path, fn_headers, body.to_vec());
    let payload = encode_request(&fn_req)
        .map_err(|e| application::AppError::Invoke(format!("encode request: {e}")))?;

    let bearer = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim);
    let response = state
        .invoke
        .execute(
            bearer,
            InvokeRequest {
                function,
                version,
                payload,
            },
        )
        .await?;

    let fn_res = decode_response(&response.output)
        .map_err(|e| application::AppError::Invoke(format!("decode response: {e}")))?;

    let status = StatusCode::from_u16(fn_res.status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let response_body = fn_res.body().to_vec();

    let mut out_headers = HeaderMap::new();
    for (name, value) in fn_res.headers() {
        let Ok(header_name) = HeaderName::try_from(name.as_str()) else {
            continue;
        };
        let Ok(header_value) = HeaderValue::try_from(value.as_str()) else {
            continue;
        };
        out_headers.insert(header_name, header_value);
    }

    let hash_hex = response.content_hash.to_hex();
    out_headers.insert(
        HeaderName::from_static(SHASUM_HEADER),
        HeaderValue::from_str(&hash_hex)
            .map_err(|e| application::AppError::Invoke(format!("hash header: {e}")))?,
    );

    if let Some(nonce) = nonce {
        if let Some(doc) = state
            .attest_invoke(&response.content_hash, &response_body, &nonce)
            .await?
        {
            let b64 = BASE64.encode(&doc);
            out_headers.insert(
                HeaderName::from_static(ATTESTATION_HEADER),
                HeaderValue::from_str(&b64).map_err(|e| {
                    application::AppError::Invoke(format!("attestation header: {e}"))
                })?,
            );
        }
    }

    Ok((status, out_headers, response_body))
}

fn parse_nonce(headers: &HeaderMap) -> Result<Option<Vec<u8>>, HttpError> {
    let Some(raw) = headers.get(NONCE_HEADER).and_then(|v| v.to_str().ok()) else {
        return Ok(None);
    };
    let bytes = BASE64.decode(raw.trim()).map_err(|_| {
        application::AppError::BadRequest("invalid x-nitrum-fn-nonce (base64)".into())
    })?;
    if !(16..=32).contains(&bytes.len()) {
        return Err(application::AppError::BadRequest(format!(
            "x-nitrum-fn-nonce must decode to 16..=32 bytes, got {}",
            bytes.len()
        ))
        .into());
    }
    Ok(Some(bytes))
}

/// Build NSM `user_data`: content hash (32) || sha256(HTTP body) (32).
pub fn invoke_user_data(hash: &ContentHash, body: &[u8]) -> [u8; 64] {
    let body_hash = ContentHash::from_bytes(body);
    let mut out = [0u8; 64];
    out[..32].copy_from_slice(hash.as_bytes());
    out[32..].copy_from_slice(body_hash.as_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_data_is_hash_then_body_digest() {
        let hash = ContentHash::from_bytes(b"\0asm");
        let body = br#"{"ok":true}"#;
        let ud = invoke_user_data(&hash, body);
        assert_eq!(&ud[..32], hash.as_bytes());
        assert_eq!(&ud[32..], ContentHash::from_bytes(body).as_bytes());
    }

    #[test]
    fn rejects_invalid_nonce_base64() {
        let mut headers = HeaderMap::new();
        headers.insert(NONCE_HEADER, "%%%".parse().unwrap());
        let err = parse_nonce(&headers).expect_err("base64");
        assert!(
            matches!(err.0, application::AppError::BadRequest(_)),
            "{:?}",
            err.0
        );
    }

    #[test]
    fn rejects_short_nonce() {
        let mut headers = HeaderMap::new();
        headers.insert(NONCE_HEADER, BASE64.encode([0u8; 8]).parse().unwrap());
        let err = parse_nonce(&headers).expect_err("length");
        assert!(
            matches!(err.0, application::AppError::BadRequest(_)),
            "{:?}",
            err.0
        );
    }

    #[test]
    fn host_crate_does_not_depend_on_payments() {
        let manifest = include_str!("../Cargo.toml");
        assert!(
            !manifest.lines().any(|line| {
                let trimmed = line.trim();
                trimmed.starts_with("payments") || trimmed.contains("crates/payments")
            }),
            "{manifest}"
        );
    }

    mod router {
        use super::super::*;
        use application::ports::{
            AccountStore, ArtifactStore, CreditOutcome, DebitReceipt, FunctionCatalog,
            FunctionRunner, RunOutcome, StoreError,
        };
        use application::{InvokeFunction, PaidInvoke};
        use async_trait::async_trait;
        use axum::body::{to_bytes, Body};
        use axum::http::Request;
        use domain::{Account, AccountId, FunctionVersion, KeyId, KeyMeta, KeyRecord, NewKey};
        use std::sync::Arc;
        use std::sync::Mutex;
        use tower::ServiceExt;

        struct Ledger {
            account: Mutex<Account>,
            key: Mutex<KeyRecord>,
            hash: domain::SecretHash,
        }

        impl Ledger {
            fn open(balance: u64) -> (Self, String) {
                let account = Account {
                    id: AccountId::generate().unwrap(),
                    balance,
                };
                let issued = NewKey::issue(account.id.clone(), None).unwrap();
                let secret = issued.secret.as_str().to_string();
                (
                    Self {
                        hash: issued.record.secret_hash.clone(),
                        account: Mutex::new(account),
                        key: Mutex::new(issued.record),
                    },
                    secret,
                )
            }
        }

        #[async_trait]
        impl AccountStore for Ledger {
            async fn create(&self, _: &Account, _: &KeyRecord) -> Result<(), StoreError> {
                unimplemented!()
            }
            async fn put_key(&self, _: &KeyRecord) -> Result<(), StoreError> {
                unimplemented!()
            }
            async fn revoke(&self, _: &AccountId, _: &KeyId) -> Result<(), StoreError> {
                unimplemented!()
            }
            async fn key_by_hash(
                &self,
                _: &domain::SecretHash,
            ) -> Result<Option<KeyRecord>, StoreError> {
                Ok(Some(self.key.lock().unwrap().clone()))
            }
            async fn account(&self, _: &AccountId) -> Result<Option<Account>, StoreError> {
                Ok(Some(self.account.lock().unwrap().clone()))
            }
            async fn list_keys(&self, _: &AccountId) -> Result<Vec<KeyMeta>, StoreError> {
                Ok(vec![])
            }
            async fn debit(
                &self,
                hash: &domain::SecretHash,
                cost: u64,
            ) -> Result<DebitReceipt, StoreError> {
                if hash != &self.hash {
                    return Err(StoreError::Unauthorized);
                }
                let mut account = self.account.lock().unwrap();
                if account.balance < cost {
                    return Err(StoreError::Insufficient {
                        account_id: account.id.clone(),
                        balance: account.balance,
                        cost,
                        spend_cap_remaining: None,
                    });
                }
                account.balance -= cost;
                Ok(DebitReceipt {
                    secret_hash: hash.clone(),
                    account_id: account.id.clone(),
                    cost,
                    capped: false,
                })
            }
            async fn refund(&self, receipt: &DebitReceipt) -> Result<(), StoreError> {
                self.account.lock().unwrap().balance += receipt.cost;
                Ok(())
            }
            async fn credit(
                &self,
                _: &AccountId,
                _: u64,
                _: &str,
            ) -> Result<CreditOutcome, StoreError> {
                unimplemented!()
            }
            async fn admin_credit(&self, _: &AccountId, _: u64) -> Result<u64, StoreError> {
                unimplemented!()
            }
        }

        struct FixedCatalog {
            version: FunctionVersion,
        }

        #[async_trait]
        impl FunctionCatalog for FixedCatalog {
            async fn upsert(
                &self,
                _: &FunctionId,
                _: &VersionLabel,
                _: ContentHash,
                _: u64,
                _: &[domain::EgressOrigin],
            ) -> Result<bool, application::AppError> {
                Ok(true)
            }
            async fn resolve(
                &self,
                _: &FunctionId,
                _: &VersionLabel,
            ) -> Result<FunctionVersion, application::AppError> {
                Ok(self.version.clone())
            }
            async fn list(&self) -> Result<Vec<FunctionVersion>, application::AppError> {
                Ok(vec![])
            }
        }

        struct Wasm(Vec<u8>);

        #[async_trait]
        impl ArtifactStore for Wasm {
            async fn put(&self, wasm: &[u8]) -> Result<ContentHash, application::AppError> {
                Ok(ContentHash::from_bytes(wasm))
            }
            async fn get(&self, hash: &ContentHash) -> Result<Vec<u8>, application::AppError> {
                let actual = ContentHash::from_bytes(&self.0);
                if &actual != hash {
                    return Err(application::AppError::HashMismatch {
                        expected: hash.to_hex(),
                        actual: actual.to_hex(),
                    });
                }
                Ok(self.0.clone())
            }
        }

        struct Runner {
            body: Vec<u8>,
        }

        #[async_trait]
        impl FunctionRunner for Runner {
            async fn validate(&self, _: &[u8]) -> Result<(), application::AppError> {
                Ok(())
            }
            async fn run(
                &self,
                _: &ContentHash,
                _: &[u8],
                _: &[u8],
                _: &[domain::EgressOrigin],
            ) -> Result<RunOutcome, application::AppError> {
                let b64 = BASE64.encode(&self.body);
                let wire = format!(r#"{{"status":200,"headers":[],"body_base64":"{b64}"}}"#);
                Ok(RunOutcome {
                    output: wire.into_bytes(),
                })
            }
        }

        struct RecAttestor {
            user_data: Mutex<Option<Vec<u8>>>,
        }

        #[async_trait]
        impl application::ports::FunctionAttestor for RecAttestor {
            async fn attest(
                &self,
                user_data: &[u8],
                _: &[u8],
            ) -> Result<Option<Vec<u8>>, application::AppError> {
                *self.user_data.lock().unwrap() = Some(user_data.to_vec());
                Ok(Some(b"doc".to_vec()))
            }
        }

        fn app(balance: u64) -> (axum::Router, Arc<RecAttestor>, String, ContentHash, Vec<u8>) {
            let wasm = b"\0asm billed".to_vec();
            let hash = ContentHash::from_bytes(&wasm);
            let body = br#"{"ok":true}"#.to_vec();
            let (ledger, secret) = Ledger::open(balance);
            let paid = PaidInvoke::new(
                1,
                "http://api.test",
                Arc::new(ledger),
                InvokeFunction::new(
                    Arc::new(FixedCatalog {
                        version: FunctionVersion {
                            id: FunctionId::new("echo").unwrap(),
                            label: VersionLabel::latest(),
                            content_hash: hash.clone(),
                            egress_allow: vec![],
                        },
                    }),
                    Arc::new(Wasm(wasm)),
                    Arc::new(Runner { body: body.clone() }),
                ),
            );
            let attestor = Arc::new(RecAttestor {
                user_data: Mutex::new(None),
            });
            let router = router(crate::state::AppState {
                invoke: Arc::new(paid),
                attestor: attestor.clone(),
            });
            (router, attestor, secret, hash, body)
        }

        #[tokio::test]
        async fn billed_success_attests_wasm_and_body_only() {
            let (app, attestor, secret, hash, body) = app(1);
            let nonce = BASE64.encode([7u8; 16]);
            let res = app
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/invoke/echo")
                        .header("authorization", format!("Bearer {secret}"))
                        .header(NONCE_HEADER, nonce)
                        .body(Body::from("{}"))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(res.status(), StatusCode::OK);
            let seen = attestor
                .user_data
                .lock()
                .unwrap()
                .clone()
                .expect("attested");
            assert_eq!(seen, invoke_user_data(&hash, &body));
            assert_eq!(seen.len(), 64);
        }

        #[tokio::test]
        async fn insufficient_credits_are_json_402_without_x402() {
            let (app, attestor, secret, _, _) = app(0);
            let res = app
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/invoke/echo")
                        .header("authorization", format!("Bearer {secret}"))
                        .body(Body::from("{}"))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(res.status(), StatusCode::PAYMENT_REQUIRED);
            assert!(res.headers().get("payment-required").is_none());
            assert!(res.headers().get("x-payment").is_none());
            assert!(res.headers().get("payment-signature").is_none());
            let bytes = to_bytes(res.into_body(), 4096).await.unwrap();
            let text = String::from_utf8(bytes.to_vec()).unwrap();
            assert!(text.contains("\"credit_cost\":1"), "{text}");
            assert!(text.contains("\"balance\":0"), "{text}");
            assert!(text.contains("/accounts/"), "{text}");
            assert!(text.contains("/credit"), "{text}");
            assert!(!text.contains("spend_cap_remaining"), "{text}");
            assert!(attestor.user_data.lock().unwrap().is_none());
        }

        #[tokio::test]
        async fn missing_bearer_is_401() {
            let (app, _, _, _, _) = app(1);
            let res = app
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/invoke/echo")
                        .body(Body::from("{}"))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        }
    }
}
