use std::sync::Arc;

use domain::{BearerSecret, InvokeRequest, InvokeResponse, SecretHash};
use tracing::instrument;

use crate::error::AppError;
use crate::ports::{AccountStore, StoreError};
use crate::usecases::invoke::InvokeFunction;

/// Debits the configured invoke-credit cost, then runs [`InvokeFunction`].
///
/// A cost of zero skips the bearer. Artifact load and compile failures refund.
/// Once the guest has been entered the debit stays, including a guest 4xx.
pub struct PaidInvoke {
    cost: u64,
    credit_base: String,
    accounts: Arc<dyn AccountStore>,
    invoke: InvokeFunction,
}

impl PaidInvoke {
    pub fn new(
        cost: u64,
        credit_base: impl Into<String>,
        accounts: Arc<dyn AccountStore>,
        invoke: InvokeFunction,
    ) -> Self {
        Self {
            cost,
            credit_base: credit_base.into().trim_end_matches('/').to_string(),
            accounts,
            invoke,
        }
    }

    pub fn cost(&self) -> u64 {
        self.cost
    }

    #[instrument(skip(self, bearer, req), fields(function = %req.function, cost = self.cost))]
    pub async fn execute(
        &self,
        bearer: Option<&str>,
        req: InvokeRequest,
    ) -> Result<InvokeResponse, AppError> {
        if self.cost == 0 {
            return self.invoke.execute(req).await;
        }
        let Some(raw) = bearer.map(str::trim).filter(|s| !s.is_empty()) else {
            return Err(AppError::Unauthorized);
        };
        let secret = BearerSecret::parse(raw).map_err(|_| AppError::Unauthorized)?;
        let hash = SecretHash::of(&secret);
        let receipt = self
            .accounts
            .debit(&hash, self.cost)
            .await
            .map_err(|err| map_debit(err, &self.credit_base))?;
        match self.invoke.execute(req).await {
            Ok(response) => Ok(response),
            Err(err) if failed_before_guest(&err) => {
                if let Err(refund_err) = self.accounts.refund(&receipt).await {
                    return Err(AppError::Storage(format!(
                        "refund after pre-guest failure ({err}): {refund_err}"
                    )));
                }
                Err(err)
            }
            Err(err) => Err(err),
        }
    }
}

fn failed_before_guest(err: &AppError) -> bool {
    matches!(
        err,
        AppError::NotFound(_)
            | AppError::ArtifactMissing(_)
            | AppError::HashMismatch { .. }
            | AppError::Compile(_)
            | AppError::Domain(_)
            | AppError::Storage(_)
    )
}

fn map_debit(err: StoreError, credit_base: &str) -> AppError {
    match err {
        StoreError::Unauthorized => AppError::Unauthorized,
        StoreError::NotFound => AppError::Unauthorized,
        StoreError::Insufficient {
            account_id,
            balance,
            cost,
            spend_cap_remaining,
        } => AppError::InsufficientCredits {
            credit_cost: cost,
            balance,
            credit_url: format!("{credit_base}/accounts/{account_id}/credit"),
            spend_cap_remaining,
        },
        StoreError::Storage(msg) => AppError::Storage(msg),
    }
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unauthorized => write!(f, "unauthorized"),
            Self::NotFound => write!(f, "not found"),
            Self::Insufficient { balance, cost, .. } => {
                write!(f, "insufficient credits: balance {balance} cost {cost}")
            }
            Self::Storage(msg) => write!(f, "{msg}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::{ArtifactStore, DebitReceipt, FunctionCatalog, FunctionRunner, RunOutcome};
    use async_trait::async_trait;
    use domain::{
        Account, AccountId, ContentHash, FunctionId, FunctionVersion, KeyRecord, NewKey,
        VersionLabel,
    };
    use std::sync::Mutex;

    struct MemStore {
        account: Mutex<Account>,
        key: Mutex<KeyRecord>,
        secret_hash: SecretHash,
    }

    impl MemStore {
        fn funded(balance: u64, cap: Option<u64>) -> (Self, String) {
            let account = Account {
                id: AccountId::generate().unwrap(),
                balance,
            };
            let issued = NewKey::issue(account.id.clone(), cap).unwrap();
            let secret = issued.secret.as_str().to_string();
            let hash = issued.record.secret_hash.clone();
            (
                Self {
                    account: Mutex::new(account),
                    key: Mutex::new(issued.record),
                    secret_hash: hash,
                },
                secret,
            )
        }
    }

    #[async_trait]
    impl AccountStore for MemStore {
        async fn create(&self, _: &Account, _: &KeyRecord) -> Result<(), StoreError> {
            unimplemented!()
        }
        async fn put_key(&self, _: &KeyRecord) -> Result<(), StoreError> {
            unimplemented!()
        }
        async fn revoke(&self, _: &AccountId, _: &domain::KeyId) -> Result<(), StoreError> {
            unimplemented!()
        }
        async fn key_by_hash(&self, hash: &SecretHash) -> Result<Option<KeyRecord>, StoreError> {
            if hash != &self.secret_hash {
                return Ok(None);
            }
            Ok(Some(self.key.lock().unwrap().clone()))
        }
        async fn account(&self, _: &AccountId) -> Result<Option<Account>, StoreError> {
            Ok(Some(self.account.lock().unwrap().clone()))
        }
        async fn list_keys(&self, _: &AccountId) -> Result<Vec<domain::KeyMeta>, StoreError> {
            Ok(vec![])
        }
        async fn debit(&self, hash: &SecretHash, cost: u64) -> Result<DebitReceipt, StoreError> {
            if hash != &self.secret_hash {
                return Err(StoreError::Unauthorized);
            }
            let key = self.key.lock().unwrap().clone();
            if key.revoked {
                return Err(StoreError::Unauthorized);
            }
            let mut account = self.account.lock().unwrap();
            if account.balance < cost {
                return Err(StoreError::Insufficient {
                    account_id: account.id.clone(),
                    balance: account.balance,
                    cost,
                    spend_cap_remaining: key.cap_remaining,
                });
            }
            if key.capped && key.cap_remaining.unwrap_or(0) < cost {
                return Err(StoreError::Insufficient {
                    account_id: account.id.clone(),
                    balance: account.balance,
                    cost,
                    spend_cap_remaining: key.cap_remaining,
                });
            }
            account.balance -= cost;
            drop(account);
            let mut key_mut = self.key.lock().unwrap();
            if key_mut.capped {
                key_mut.cap_remaining = Some(key_mut.cap_remaining.unwrap_or(0) - cost);
            }
            Ok(DebitReceipt {
                secret_hash: hash.clone(),
                account_id: key_mut.account_id.clone(),
                cost,
                capped: key_mut.capped,
            })
        }
        async fn refund(&self, receipt: &DebitReceipt) -> Result<(), StoreError> {
            self.account.lock().unwrap().balance += receipt.cost;
            if receipt.capped {
                let mut key = self.key.lock().unwrap();
                key.cap_remaining = Some(key.cap_remaining.unwrap_or(0) + receipt.cost);
            }
            Ok(())
        }
        async fn credit(
            &self,
            _: &AccountId,
            _: u64,
            _: &str,
        ) -> Result<crate::ports::CreditOutcome, StoreError> {
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
        ) -> Result<bool, AppError> {
            Ok(true)
        }
        async fn resolve(
            &self,
            id: &FunctionId,
            _: &VersionLabel,
        ) -> Result<FunctionVersion, AppError> {
            if id != &self.version.id {
                return Err(AppError::NotFound(id.to_string()));
            }
            Ok(self.version.clone())
        }
        async fn list(&self) -> Result<Vec<FunctionVersion>, AppError> {
            Ok(vec![])
        }
    }

    struct MemArtifacts {
        wasm: Vec<u8>,
        missing: bool,
    }

    #[async_trait]
    impl ArtifactStore for MemArtifacts {
        async fn put(&self, wasm: &[u8]) -> Result<ContentHash, AppError> {
            Ok(ContentHash::from_bytes(wasm))
        }
        async fn get(&self, hash: &ContentHash) -> Result<Vec<u8>, AppError> {
            if self.missing {
                return Err(AppError::ArtifactMissing(hash.to_hex()));
            }
            Ok(self.wasm.clone())
        }
    }

    struct Runner {
        mode: Mutex<Mode>,
        calls: Mutex<u32>,
    }

    enum Mode {
        Ok,
        Guest4xx,
        Compile,
        Trap,
    }

    #[async_trait]
    impl FunctionRunner for Runner {
        async fn validate(&self, _: &[u8]) -> Result<(), AppError> {
            Ok(())
        }
        async fn run(
            &self,
            _: &ContentHash,
            _: &[u8],
            _: &[u8],
            _: &[domain::EgressOrigin],
        ) -> Result<RunOutcome, AppError> {
            *self.calls.lock().unwrap() += 1;
            match *self.mode.lock().unwrap() {
                Mode::Ok => Ok(RunOutcome {
                    output: b"ok".to_vec(),
                }),
                Mode::Guest4xx => Ok(RunOutcome {
                    output: b"status=400".to_vec(),
                }),
                Mode::Compile => Err(AppError::Compile("bad module".into())),
                Mode::Trap => Err(AppError::Trap("unreachable".into())),
            }
        }
    }

    fn paid(
        cost: u64,
        store: Arc<MemStore>,
        runner: Arc<Runner>,
        missing_wasm: bool,
    ) -> PaidInvoke {
        let wasm = b"\0asm";
        let hash = ContentHash::from_bytes(wasm);
        let catalog = Arc::new(FixedCatalog {
            version: FunctionVersion {
                id: FunctionId::new("echo").unwrap(),
                label: VersionLabel::latest(),
                content_hash: hash.clone(),
                egress_allow: vec![],
            },
        });
        let artifacts = Arc::new(MemArtifacts {
            wasm: wasm.to_vec(),
            missing: missing_wasm,
        });
        PaidInvoke::new(
            cost,
            "http://api.test",
            store,
            InvokeFunction::new(catalog, artifacts, runner),
        )
    }

    fn request() -> InvokeRequest {
        InvokeRequest {
            function: FunctionId::new("echo").unwrap(),
            version: VersionLabel::latest(),
            payload: b"hi".to_vec(),
        }
    }

    #[tokio::test]
    async fn cost_zero_skips_the_bearer() {
        let (store, _secret) = MemStore::funded(0, None);
        let store = Arc::new(store);
        let runner = Arc::new(Runner {
            mode: Mutex::new(Mode::Ok),
            calls: Mutex::new(0),
        });
        let invoke = paid(0, store.clone(), runner.clone(), false);
        let out = invoke.execute(None, request()).await.expect("unbilled");
        assert_eq!(out.output, b"ok");
        assert_eq!(store.account.lock().unwrap().balance, 0);
        assert_eq!(*runner.calls.lock().unwrap(), 1);
    }

    #[tokio::test]
    async fn one_credit_debit_then_runs() {
        let (store, secret) = MemStore::funded(2, Some(2));
        let store = Arc::new(store);
        let runner = Arc::new(Runner {
            mode: Mutex::new(Mode::Ok),
            calls: Mutex::new(0),
        });
        let invoke = paid(1, store.clone(), runner, false);
        invoke
            .execute(Some(&secret), request())
            .await
            .expect("paid");
        assert_eq!(store.account.lock().unwrap().balance, 1);
        assert_eq!(store.key.lock().unwrap().cap_remaining, Some(1));
    }

    #[tokio::test]
    async fn insufficient_balance_does_not_run() {
        let (store, secret) = MemStore::funded(0, None);
        let store = Arc::new(store);
        let runner = Arc::new(Runner {
            mode: Mutex::new(Mode::Ok),
            calls: Mutex::new(0),
        });
        let invoke = paid(1, store.clone(), runner.clone(), false);
        let err = invoke
            .execute(Some(&secret), request())
            .await
            .expect_err("broke");
        match err {
            AppError::InsufficientCredits {
                credit_cost,
                balance,
                credit_url,
                spend_cap_remaining,
            } => {
                assert_eq!(credit_cost, 1);
                assert_eq!(balance, 0);
                assert!(credit_url.contains("/credit"), "{credit_url}");
                assert!(spend_cap_remaining.is_none());
            }
            other => panic!("{other}"),
        }
        assert_eq!(*runner.calls.lock().unwrap(), 0);
        assert_eq!(store.account.lock().unwrap().balance, 0);
    }

    #[tokio::test]
    async fn spend_cap_blocks_a_funded_account() {
        let (store, secret) = MemStore::funded(10, Some(0));
        let store = Arc::new(store);
        let runner = Arc::new(Runner {
            mode: Mutex::new(Mode::Ok),
            calls: Mutex::new(0),
        });
        let invoke = paid(1, store.clone(), runner.clone(), false);
        let err = invoke
            .execute(Some(&secret), request())
            .await
            .expect_err("cap");
        match err {
            AppError::InsufficientCredits {
                balance,
                spend_cap_remaining,
                ..
            } => {
                assert_eq!(balance, 10);
                assert_eq!(spend_cap_remaining, Some(0));
            }
            other => panic!("{other}"),
        }
        assert_eq!(*runner.calls.lock().unwrap(), 0);
        assert_eq!(store.account.lock().unwrap().balance, 10);
        assert_eq!(store.key.lock().unwrap().cap_remaining, Some(0));
    }

    #[tokio::test]
    async fn guest_4xx_stays_paid() {
        let (store, secret) = MemStore::funded(1, Some(1));
        let store = Arc::new(store);
        let runner = Arc::new(Runner {
            mode: Mutex::new(Mode::Guest4xx),
            calls: Mutex::new(0),
        });
        let invoke = paid(1, store.clone(), runner, false);
        let out = invoke
            .execute(Some(&secret), request())
            .await
            .expect("guest 4xx is still Ok");
        assert_eq!(out.output, b"status=400");
        assert_eq!(store.account.lock().unwrap().balance, 0);
        assert_eq!(store.key.lock().unwrap().cap_remaining, Some(0));
    }

    #[tokio::test]
    async fn pre_guest_failure_restores_balance_and_cap() {
        let (store, secret) = MemStore::funded(4, Some(3));
        let store = Arc::new(store);
        let runner = Arc::new(Runner {
            mode: Mutex::new(Mode::Compile),
            calls: Mutex::new(0),
        });
        let invoke = paid(1, store.clone(), runner, false);
        let err = invoke
            .execute(Some(&secret), request())
            .await
            .expect_err("compile");
        assert!(matches!(err, AppError::Compile(_)), "{err}");
        assert_eq!(store.account.lock().unwrap().balance, 4);
        assert_eq!(store.key.lock().unwrap().cap_remaining, Some(3));
    }

    #[tokio::test]
    async fn missing_artifact_restores_balance() {
        let (store, secret) = MemStore::funded(4, Some(3));
        let store = Arc::new(store);
        let runner = Arc::new(Runner {
            mode: Mutex::new(Mode::Ok),
            calls: Mutex::new(0),
        });
        let invoke = paid(1, store.clone(), runner.clone(), true);
        let err = invoke
            .execute(Some(&secret), request())
            .await
            .expect_err("missing");
        assert!(matches!(err, AppError::ArtifactMissing(_)), "{err}");
        assert_eq!(*runner.calls.lock().unwrap(), 0);
        assert_eq!(store.account.lock().unwrap().balance, 4);
        assert_eq!(store.key.lock().unwrap().cap_remaining, Some(3));
    }

    #[tokio::test]
    async fn guest_trap_stays_paid() {
        let (store, secret) = MemStore::funded(1, None);
        let store = Arc::new(store);
        let runner = Arc::new(Runner {
            mode: Mutex::new(Mode::Trap),
            calls: Mutex::new(0),
        });
        let invoke = paid(1, store.clone(), runner, false);
        let err = invoke
            .execute(Some(&secret), request())
            .await
            .expect_err("trap");
        assert!(matches!(err, AppError::Trap(_)), "{err}");
        assert_eq!(store.account.lock().unwrap().balance, 0);
    }
}
