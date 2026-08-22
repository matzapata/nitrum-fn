use std::collections::HashMap;

use application::error::AppError;
use application::ports::{evaluate_claim, IdempotencyClaim, IdempotencyRecord, IdempotencyStatus};
use domain::{FunctionId, IdempotencyKey};

pub(crate) const TTL_SECS: u64 = 24 * 60 * 60;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Stored {
    pub record: IdempotencyRecord,
    pub expires_at: u64,
}

pub(crate) fn storage_key(function: &FunctionId, key: &IdempotencyKey) -> String {
    format!("{}#{}", function.as_str(), key.as_str())
}

pub(crate) fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub(crate) fn is_live(expires_at: u64, now: u64) -> bool {
    expires_at > now
}

pub(crate) fn claim_in_map(
    records: &mut HashMap<String, Stored>,
    now: u64,
    key: &IdempotencyKey,
    proposed: &IdempotencyRecord,
) -> Result<IdempotencyClaim, AppError> {
    let sk = storage_key(&proposed.function, key);
    if let Some(existing) = records.get(&sk) {
        if is_live(existing.expires_at, now) {
            return evaluate_claim(&existing.record, proposed);
        }
    }
    records.insert(
        sk,
        Stored {
            record: IdempotencyRecord {
                status: IdempotencyStatus::Pending,
                ..proposed.clone()
            },
            expires_at: now.saturating_add(TTL_SECS),
        },
    );
    Ok(IdempotencyClaim::Proceed)
}

pub(crate) fn complete_in_map(
    records: &mut HashMap<String, Stored>,
    now: u64,
    key: &IdempotencyKey,
    record: &IdempotencyRecord,
) -> Result<(), AppError> {
    let sk = storage_key(&record.function, key);
    if let Some(existing) = records.get(&sk) {
        if is_live(existing.expires_at, now)
            && (existing.record.function != record.function
                || existing.record.content_hash != record.content_hash)
        {
            return Err(AppError::Storage(
                "idempotency complete payload mismatch".into(),
            ));
        }
    }
    records.insert(
        sk,
        Stored {
            record: IdempotencyRecord {
                status: IdempotencyStatus::Completed,
                ..record.clone()
            },
            expires_at: now.saturating_add(TTL_SECS),
        },
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::ContentHash;

    fn rec(name: &str, wasm: &[u8], status: IdempotencyStatus) -> IdempotencyRecord {
        IdempotencyRecord {
            function: FunctionId::new(name).unwrap(),
            content_hash: ContentHash::from_bytes(wasm),
            wasm_bytes: wasm.len(),
            status,
        }
    }

    fn key(raw: &str) -> IdempotencyKey {
        IdempotencyKey::new(raw).unwrap()
    }

    #[test]
    fn expired_row_can_be_reused_for_a_new_hash() {
        let mut records = HashMap::new();
        let k = key("retry-1");
        let first = rec("echo", b"one", IdempotencyStatus::Completed);
        records.insert(
            storage_key(&first.function, &k),
            Stored {
                record: first,
                expires_at: 10,
            },
        );
        let second = rec("echo", b"two", IdempotencyStatus::Pending);
        assert_eq!(
            claim_in_map(&mut records, 11, &k, &second).unwrap(),
            IdempotencyClaim::Proceed
        );
        assert_eq!(
            records[&storage_key(&second.function, &k)]
                .record
                .content_hash,
            second.content_hash
        );
    }

    #[test]
    fn same_key_different_functions_do_not_collide() {
        let mut records = HashMap::new();
        let k = key("retry-1");
        let echo = rec("echo", b"one", IdempotencyStatus::Pending);
        let other = rec("other", b"one", IdempotencyStatus::Pending);
        assert_eq!(
            claim_in_map(&mut records, 1, &k, &echo).unwrap(),
            IdempotencyClaim::Proceed
        );
        assert_eq!(
            claim_in_map(&mut records, 1, &k, &other).unwrap(),
            IdempotencyClaim::Proceed
        );
        assert_eq!(records.len(), 2);
    }

    #[test]
    fn complete_after_pending_marks_replay() {
        let mut records = HashMap::new();
        let k = key("retry-1");
        let pending = rec("echo", b"one", IdempotencyStatus::Pending);
        claim_in_map(&mut records, 1, &k, &pending).unwrap();
        complete_in_map(&mut records, 1, &k, &pending).unwrap();
        assert_eq!(
            claim_in_map(&mut records, 1, &k, &pending).unwrap(),
            IdempotencyClaim::Replay(IdempotencyRecord {
                status: IdempotencyStatus::Completed,
                ..pending
            })
        );
    }
}
