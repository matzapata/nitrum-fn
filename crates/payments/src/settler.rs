use std::sync::Arc;
use std::time::Duration;

use application::ports::{CreditSettler, PaymentChallenge};
use application::AppError;
use async_trait::async_trait;
use domain::AccountId;
use serde::Serialize;

use crate::{
    apply_after_settle, check_exact, due_usdc, parse_payment, unix_now, ExactRequirements,
    PaymentError,
};

#[async_trait]
pub trait Facilitator: Send + Sync {
    async fn settle_exact(
        &self,
        payment_header: &str,
        requirements: &ExactRequirements,
        network: &str,
        resource: &str,
    ) -> Result<(), PaymentError>;
}

pub struct HttpFacilitator {
    client: reqwest::Client,
    base_url: String,
}

impl HttpFacilitator {
    pub fn new(base_url: impl Into<String>) -> Result<Self, PaymentError> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .connect_timeout(Duration::from_secs(3))
            .build()
            .map_err(|e| PaymentError::Settle(e.to_string()))?;
        Ok(Self {
            client,
            base_url: base_url.into().trim_end_matches('/').to_string(),
        })
    }
}

#[derive(Serialize)]
struct SettleRequest<'a> {
    #[serde(rename = "x402Version")]
    x402_version: u32,
    #[serde(rename = "paymentHeader")]
    payment_header: &'a str,
    #[serde(rename = "paymentRequirements")]
    payment_requirements: RequirementsBody<'a>,
}

#[derive(Serialize)]
struct RequirementsBody<'a> {
    scheme: &'static str,
    network: &'a str,
    #[serde(rename = "maxAmountRequired")]
    max_amount_required: String,
    #[serde(rename = "payTo")]
    pay_to: &'a str,
    asset: &'a str,
    resource: &'a str,
}

#[async_trait]
impl Facilitator for HttpFacilitator {
    async fn settle_exact(
        &self,
        payment_header: &str,
        requirements: &ExactRequirements,
        network: &str,
        resource: &str,
    ) -> Result<(), PaymentError> {
        let body = SettleRequest {
            x402_version: 1,
            payment_header,
            payment_requirements: RequirementsBody {
                scheme: "exact",
                network,
                max_amount_required: requirements.amount.to_string(),
                pay_to: &requirements.pay_to,
                asset: &requirements.asset,
                resource,
            },
        };
        let url = format!("{}/settle", self.base_url);
        let response = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| PaymentError::Settle(e.to_string()))?;
        if !response.status().is_success() {
            return Err(PaymentError::Settle(format!(
                "facilitator status {}",
                response.status()
            )));
        }
        let parsed: serde_json::Value = response
            .json()
            .await
            .map_err(|e| PaymentError::Settle(e.to_string()))?;
        if parsed.get("success").and_then(|v| v.as_bool()) != Some(true) {
            return Err(PaymentError::Settle(
                parsed
                    .get("errorReason")
                    .and_then(|v| v.as_str())
                    .unwrap_or("facilitator rejected the payment")
                    .to_string(),
            ));
        }
        Ok(())
    }
}

pub struct X402Settler<F> {
    facilitator: F,
    usdc_per_invoke: u64,
    asset: String,
    pay_to: String,
    network: String,
    credit_base: String,
    now: Arc<dyn Fn() -> u64 + Send + Sync>,
}

impl X402Settler<HttpFacilitator> {
    pub fn from_config(
        facilitator_url: &str,
        usdc_per_invoke: u64,
        asset: impl Into<String>,
        pay_to: impl Into<String>,
        network: impl Into<String>,
        credit_base: impl Into<String>,
    ) -> Result<Self, PaymentError> {
        Ok(Self::new(
            HttpFacilitator::new(facilitator_url)?,
            usdc_per_invoke,
            asset,
            pay_to,
            network,
            credit_base,
            Arc::new(unix_now),
        ))
    }
}

impl<F> X402Settler<F> {
    pub fn new(
        facilitator: F,
        usdc_per_invoke: u64,
        asset: impl Into<String>,
        pay_to: impl Into<String>,
        network: impl Into<String>,
        credit_base: impl Into<String>,
        now: Arc<dyn Fn() -> u64 + Send + Sync>,
    ) -> Self {
        Self {
            facilitator,
            usdc_per_invoke,
            asset: asset.into(),
            pay_to: pay_to.into(),
            network: network.into(),
            credit_base: credit_base.into().trim_end_matches('/').to_string(),
            now,
        }
    }

    fn requirements(&self, invokes: u64) -> Result<ExactRequirements, PaymentError> {
        Ok(ExactRequirements {
            amount: due_usdc(invokes, self.usdc_per_invoke)?,
            asset: self.asset.clone(),
            pay_to: self.pay_to.clone(),
        })
    }

    fn resource(&self, account_id: &AccountId) -> String {
        format!("{}/accounts/{account_id}/credit", self.credit_base)
    }
}

#[async_trait]
impl<F: Facilitator> CreditSettler for X402Settler<F> {
    async fn challenge(
        &self,
        account_id: &AccountId,
        invokes: u64,
    ) -> Result<PaymentChallenge, AppError> {
        let requirements = self.requirements(invokes).map_err(payment_to_app)?;
        Ok(PaymentChallenge {
            invokes,
            amount: requirements.amount,
            asset: requirements.asset,
            pay_to: requirements.pay_to,
            resource: self.resource(account_id),
            network: self.network.clone(),
        })
    }

    async fn settle(
        &self,
        account_id: &AccountId,
        invokes: u64,
        payment_header: &str,
    ) -> Result<String, AppError> {
        let requirements = self.requirements(invokes).map_err(payment_to_app)?;
        let auth = parse_payment(payment_header).map_err(payment_to_app)?;
        check_exact(&auth, &requirements, (self.now)()).map_err(payment_to_app)?;
        let nonce = auth.nonce.clone();
        let resource = self.resource(account_id);
        apply_after_settle(
            self.facilitator
                .settle_exact(payment_header, &requirements, &self.network, &resource),
            || Ok(nonce.clone()),
        )
        .await
        .map_err(payment_to_app)
    }
}

fn payment_to_app(err: PaymentError) -> AppError {
    match err {
        PaymentError::Settle(msg) => AppError::Storage(format!("x402 settle: {msg}")),
        other => AppError::BadRequest(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct SharedFacilitator {
        calls: Arc<AtomicUsize>,
        fail: bool,
    }

    fn settler(fail: bool) -> (X402Settler<SharedFacilitator>, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        let settler = X402Settler::new(
            SharedFacilitator {
                calls: calls.clone(),
                fail,
            },
            25,
            "0xusdc",
            "0xplatform",
            "base",
            "http://api.test",
            Arc::new(|| 10),
        );
        (settler, calls)
    }

    #[async_trait]
    impl Facilitator for SharedFacilitator {
        async fn settle_exact(
            &self,
            _: &str,
            _: &ExactRequirements,
            _: &str,
            _: &str,
        ) -> Result<(), PaymentError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.fail {
                Err(PaymentError::Settle("no".into()))
            } else {
                Ok(())
            }
        }
    }

    fn header(value: u64) -> String {
        format!(
            r#"{{"scheme":"exact","asset":"0xusdc","payload":{{"authorization":{{"to":"0xplatform","value":"{value}","validBefore":"100","nonce":"n-1"}}}}}}"#
        )
    }

    #[tokio::test]
    async fn mismatched_amount_does_not_settle_or_return_a_nonce() {
        let (settler, calls) = settler(false);
        let account = AccountId::generate().unwrap();
        let err = settler.settle(&account, 4, &header(99)).await.unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)), "{err}");
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn failed_settle_does_not_return_a_nonce() {
        let (settler, calls) = settler(true);
        let account = AccountId::generate().unwrap();
        let err = settler.settle(&account, 4, &header(100)).await.unwrap_err();
        assert!(matches!(err, AppError::Storage(_)), "{err}");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
