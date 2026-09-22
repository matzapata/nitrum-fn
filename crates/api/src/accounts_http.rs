use std::sync::Arc;

use application::{Accounts, AppError};
use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderName, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use domain::{AccountId, BearerSecret, KeyId};
use serde::{Deserialize, Serialize};

use crate::error::HttpError;

const PAYMENT_SIGNATURE: &str = "payment-signature";
const X_PAYMENT: &str = "x-payment";
pub const PAYMENT_REQUIRED: &str = "payment-required";

pub fn router(accounts: Arc<Accounts>) -> Router {
    Router::new()
        .route("/accounts", post(create_account))
        .route("/accounts/{id}", get(get_account))
        .route("/accounts/{id}/keys", post(issue_key))
        .route("/accounts/{id}/keys/{key_id}/revoke", post(revoke_key))
        .route("/accounts/{id}/credit", post(credit))
        .route("/accounts/{id}/admin-credit", post(admin_credit))
        .with_state(accounts)
}

pub fn bearer_secret(headers: &HeaderMap) -> Option<BearerSecret> {
    let raw = headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?;
    let token = raw.strip_prefix("Bearer ")?.trim();
    BearerSecret::parse(token).ok()
}

fn payment_header(headers: &HeaderMap) -> Option<String> {
    for name in [PAYMENT_SIGNATURE, X_PAYMENT] {
        if let Some(value) = headers.get(name).and_then(|v| v.to_str().ok()) {
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

fn operator_token(headers: &HeaderMap) -> &str {
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("")
        .trim()
}

fn account_id(raw: &str) -> Result<AccountId, HttpError> {
    AccountId::from_hex(raw)
        .map_err(|_| AppError::BadRequest("invalid account id".into()))
        .map_err(Into::into)
}

fn key_id(raw: &str) -> Result<KeyId, HttpError> {
    KeyId::from_hex(raw)
        .map_err(|_| AppError::BadRequest("invalid key id".into()))
        .map_err(Into::into)
}

#[derive(Serialize)]
struct CreatedBody {
    account_id: String,
    key_id: String,
    secret: String,
    balance: u64,
}

#[derive(Serialize)]
struct IssuedBody {
    key_id: String,
    secret: String,
}

#[derive(Serialize)]
struct KeyBody {
    key_id: String,
    revoked: bool,
    capped: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    cap_remaining: Option<u64>,
}

#[derive(Serialize)]
struct AccountBody {
    account_id: String,
    balance: u64,
    keys: Vec<KeyBody>,
}

#[derive(Deserialize)]
struct IssueBody {
    #[serde(default)]
    spend_cap: Option<u64>,
}

#[derive(Deserialize)]
struct InvokesBody {
    invokes: u64,
}

#[derive(Serialize)]
struct BalanceBody {
    account_id: String,
    balance: u64,
    invokes: u64,
}

async fn create_account(
    State(accounts): State<Arc<Accounts>>,
) -> Result<impl IntoResponse, HttpError> {
    let created = accounts.create().await?;
    Ok(Json(CreatedBody {
        account_id: created.account_id.to_string(),
        key_id: created.key_id.to_string(),
        secret: created.secret.as_str().to_string(),
        balance: created.balance,
    }))
}

async fn issue_key(
    State(accounts): State<Arc<Accounts>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<IssueBody>,
) -> Result<impl IntoResponse, HttpError> {
    let account_id = account_id(&id)?;
    let issued = accounts
        .issue_key(
            &account_id,
            bearer_secret(&headers).as_ref(),
            body.spend_cap,
        )
        .await?;
    Ok(Json(IssuedBody {
        key_id: issued.key_id.to_string(),
        secret: issued.secret.as_str().to_string(),
    }))
}

async fn revoke_key(
    State(accounts): State<Arc<Accounts>>,
    Path((id, key)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, HttpError> {
    let account_id = account_id(&id)?;
    let key_id = key_id(&key)?;
    accounts
        .revoke(&account_id, &key_id, bearer_secret(&headers).as_ref())
        .await?;
    Ok(StatusCode::OK)
}

async fn get_account(
    State(accounts): State<Arc<Accounts>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, HttpError> {
    let account_id = account_id(&id)?;
    let secret = bearer_secret(&headers).ok_or(AppError::Unauthorized)?;
    let view = accounts.read(&secret).await?;
    if view.account_id != account_id {
        return Err(AppError::Unauthorized.into());
    }
    Ok(Json(AccountBody {
        account_id: view.account_id.to_string(),
        balance: view.balance,
        keys: view
            .keys
            .into_iter()
            .map(|k| KeyBody {
                key_id: k.key_id.to_string(),
                revoked: k.revoked,
                capped: k.capped,
                cap_remaining: k.cap_remaining,
            })
            .collect(),
    }))
}

async fn credit(
    State(accounts): State<Arc<Accounts>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<InvokesBody>,
) -> Result<impl IntoResponse, HttpError> {
    let account_id = account_id(&id)?;
    let balance = accounts
        .credit(
            &account_id,
            body.invokes,
            payment_header(&headers).as_deref(),
        )
        .await?;
    Ok(Json(BalanceBody {
        account_id: account_id.to_string(),
        balance,
        invokes: body.invokes,
    }))
}

async fn admin_credit(
    State(accounts): State<Arc<Accounts>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<InvokesBody>,
) -> Result<impl IntoResponse, HttpError> {
    let account_id = account_id(&id)?;
    let balance = accounts
        .admin_credit(&account_id, body.invokes, operator_token(&headers))
        .await?;
    Ok(Json(BalanceBody {
        account_id: account_id.to_string(),
        balance,
        invokes: body.invokes,
    }))
}

pub fn payment_required_name() -> HeaderName {
    HeaderName::from_static(PAYMENT_REQUIRED)
}

#[cfg(test)]
mod tests {
    use super::*;
    use application::ports::{AccountStore, CreditSettler, PaymentChallenge};
    use async_trait::async_trait;
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use serde_json::Value;
    use std::sync::Arc;
    use tower::ServiceExt;

    use crate::testutil::MemLedger;

    const PRICE: u64 = 10;
    const PAY_TO: &str = "0xplatform";
    const OPERATOR: &str = "operator-secret";

    struct Settler;

    #[async_trait]
    impl CreditSettler for Settler {
        async fn challenge(
            &self,
            account_id: &AccountId,
            invokes: u64,
        ) -> Result<PaymentChallenge, AppError> {
            Ok(PaymentChallenge {
                invokes,
                amount: invokes * PRICE,
                asset: "0xusdc".into(),
                pay_to: PAY_TO.into(),
                resource: format!("http://api.test/accounts/{account_id}/credit"),
                network: "base".into(),
            })
        }

        async fn settle(
            &self,
            _account_id: &AccountId,
            _invokes: u64,
            header: &str,
        ) -> Result<String, AppError> {
            Ok(header.to_string())
        }
    }

    fn app(ledger: Arc<MemLedger>) -> Router {
        let accounts = Arc::new(Accounts::new(
            ledger,
            Arc::new(Settler),
            OPERATOR,
            "http://api.test",
        ));
        router(accounts)
    }

    async fn json(res: axum::response::Response) -> Value {
        let bytes = to_bytes(res.into_body(), 64 * 1024).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    async fn create(app: &Router) -> Value {
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/accounts")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        json(res).await
    }

    #[tokio::test]
    async fn open_create_returns_secret_once_and_list_omits_it() {
        let ledger = Arc::new(MemLedger::new());
        let created = create(&app(ledger.clone())).await;
        let secret = created["secret"].as_str().unwrap().to_string();
        let account_id = created["account_id"].as_str().unwrap();
        assert!(secret.starts_with("nfk_"));
        assert_eq!(created["balance"], 0);

        let res = app(ledger)
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/accounts/{account_id}"))
                    .header("authorization", format!("Bearer {secret}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = json(res).await;
        let text = body.to_string();
        assert!(!text.contains(&secret));
        assert!(!text.contains("secret_hash"));
        assert_eq!(body["balance"], 0);
        assert_eq!(body["keys"][0]["key_id"], created["key_id"]);
    }

    #[tokio::test]
    async fn issue_and_revoke_require_that_accounts_bearer() {
        let ledger = Arc::new(MemLedger::new());
        let created = create(&app(ledger.clone())).await;
        let account_id = created["account_id"].as_str().unwrap();
        let secret = created["secret"].as_str().unwrap();

        let denied = app(ledger.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/accounts/{account_id}/keys"))
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);

        let issued = app(ledger.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/accounts/{account_id}/keys"))
                    .header("authorization", format!("Bearer {secret}"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"spend_cap":3}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(issued.status(), StatusCode::OK);
        let issued = json(issued).await;
        let new_secret = issued["secret"].as_str().unwrap();
        assert_ne!(new_secret, secret);
        assert!(new_secret.starts_with("nfk_"));

        let key_id = created["key_id"].as_str().unwrap();
        let stranger = app(ledger.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/accounts/{account_id}/keys/{key_id}/revoke"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(stranger.status(), StatusCode::UNAUTHORIZED);

        let revoked = app(ledger)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/accounts/{account_id}/keys/{key_id}/revoke"))
                    .header("authorization", format!("Bearer {secret}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(revoked.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn unpaid_credit_is_a_challenge_and_replay_does_not_credit_again() {
        let ledger = Arc::new(MemLedger::new());
        let created = create(&app(ledger.clone())).await;
        let account_id = created["account_id"].as_str().unwrap();

        let unpaid = app(ledger.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/accounts/{account_id}/credit"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"invokes":4}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unpaid.status(), StatusCode::PAYMENT_REQUIRED);
        assert!(unpaid.headers().get(PAYMENT_REQUIRED).is_some());
        let body = json(unpaid).await;
        assert_eq!(body["accepts"][0]["payTo"], PAY_TO);
        assert_eq!(
            body["accepts"][0]["maxAmountRequired"],
            (4 * PRICE).to_string()
        );
        let balance = ledger
            .account(&AccountId::from_hex(account_id).unwrap())
            .await
            .unwrap()
            .unwrap()
            .balance;
        assert_eq!(balance, 0);

        let pay = |ledger: Arc<MemLedger>| {
            let account_id = account_id.to_string();
            async move {
                app(ledger)
                    .oneshot(
                        Request::builder()
                            .method("POST")
                            .uri(format!("/accounts/{account_id}/credit"))
                            .header("content-type", "application/json")
                            .header(PAYMENT_SIGNATURE, "nonce-once")
                            .body(Body::from(r#"{"invokes":4}"#))
                            .unwrap(),
                    )
                    .await
                    .unwrap()
            }
        };
        let first = pay(ledger.clone()).await;
        assert_eq!(first.status(), StatusCode::OK);
        assert_eq!(json(first).await["balance"], 4);
        let second = pay(ledger.clone()).await;
        assert_eq!(second.status(), StatusCode::OK);
        assert_eq!(json(second).await["balance"], 4);
    }

    #[tokio::test]
    async fn admin_credit_requires_operator_token_and_does_not_change_the_cap() {
        let ledger = Arc::new(MemLedger::new());
        let created = create(&app(ledger.clone())).await;
        let account_id = created["account_id"].as_str().unwrap();
        let secret = created["secret"].as_str().unwrap();
        let issued = app(ledger.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/accounts/{account_id}/keys"))
                    .header("authorization", format!("Bearer {secret}"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"spend_cap":3}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(issued.status(), StatusCode::OK);

        let missing = app(ledger.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/accounts/{account_id}/admin-credit"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"invokes":9}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(missing.status(), StatusCode::UNAUTHORIZED);
        let balance = ledger
            .account(&AccountId::from_hex(account_id).unwrap())
            .await
            .unwrap()
            .unwrap()
            .balance;
        assert_eq!(balance, 0);

        let granted = app(ledger.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/accounts/{account_id}/admin-credit"))
                    .header("authorization", format!("Bearer {OPERATOR}"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"invokes":9}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(granted.status(), StatusCode::OK);
        assert!(granted.headers().get(PAYMENT_REQUIRED).is_none());
        assert_eq!(json(granted).await["balance"], 9);

        let view = app(ledger)
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/accounts/{account_id}"))
                    .header("authorization", format!("Bearer {secret}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = json(view).await;
        let capped = body["keys"]
            .as_array()
            .unwrap()
            .iter()
            .find(|k| k["capped"] == true)
            .unwrap();
        assert_eq!(capped["cap_remaining"], 3);
    }
}
