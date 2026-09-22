//! Invoke-credit accounts and bearer keys.
//!
//! The balance counts invoke credits. The bearer secret is returned once at
//! issue time; the stored [`KeyRecord`] keeps only its SHA-256.

use sha2::{Digest, Sha256};
use std::fmt;

use crate::DomainError;

const SECRET_PREFIX: &str = "nfk_";
const SECRET_RANDOM_LEN: usize = 32;
const ID_LEN: usize = 16;

/// 128-bit account identifier. Credit does not require the bearer, so the id
/// itself must be unguessable.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AccountId([u8; ID_LEN]);

impl AccountId {
    pub fn generate() -> Result<Self, DomainError> {
        Ok(Self(random_bytes()?))
    }

    pub fn from_hex(hex_str: &str) -> Result<Self, DomainError> {
        let bytes = decode_fixed(hex_str).map_err(|_| DomainError::InvalidAccountId)?;
        Ok(Self(bytes))
    }

    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }
}

impl fmt::Display for AccountId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

/// Account balance is a count of invoke credits, not a USDC amount.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub id: AccountId,
    pub balance: u64,
}

impl Account {
    pub fn open() -> Result<Self, DomainError> {
        Ok(Self {
            id: AccountId::generate()?,
            balance: 0,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct KeyId([u8; ID_LEN]);

impl KeyId {
    pub fn generate() -> Result<Self, DomainError> {
        Ok(Self(random_bytes()?))
    }

    pub fn from_hex(hex_str: &str) -> Result<Self, DomainError> {
        let bytes = decode_fixed(hex_str).map_err(|_| DomainError::InvalidKeyId)?;
        Ok(Self(bytes))
    }

    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }
}

impl fmt::Display for KeyId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

/// SHA-256 of the bearer secret string (`nfk_` plus the random hex).
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct SecretHash([u8; 32]);

impl SecretHash {
    pub fn of(secret: &BearerSecret) -> Self {
        let digest = Sha256::digest(secret.as_str().as_bytes());
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&digest);
        Self(arr)
    }

    pub fn from_hex(hex_str: &str) -> Result<Self, DomainError> {
        let bytes = hex::decode(hex_str).map_err(|_| DomainError::InvalidBearer)?;
        if bytes.len() != 32 {
            return Err(DomainError::InvalidBearer);
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&bytes);
        Ok(Self(arr))
    }

    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }
}

impl fmt::Debug for SecretHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SecretHash({})", self.to_hex())
    }
}

/// Bearer secret returned once. `nfk_` plus 32 CSPRNG bytes, hex-encoded.
#[derive(Clone, PartialEq, Eq)]
pub struct BearerSecret(String);

impl BearerSecret {
    pub fn generate() -> Result<Self, DomainError> {
        let raw: [u8; SECRET_RANDOM_LEN] = random_bytes()?;
        Ok(Self(format!("{SECRET_PREFIX}{}", hex::encode(raw))))
    }

    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        let rest = raw
            .strip_prefix(SECRET_PREFIX)
            .ok_or(DomainError::InvalidBearer)?;
        if rest.len() != SECRET_RANDOM_LEN * 2 || !rest.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(DomainError::InvalidBearer);
        }
        Ok(Self(format!(
            "{SECRET_PREFIX}{}",
            rest.to_ascii_lowercase()
        )))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for BearerSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("BearerSecret([redacted])")
    }
}

/// Persisted key. Holds the secret hash only — never the secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyRecord {
    pub secret_hash: SecretHash,
    pub account_id: AccountId,
    pub key_id: KeyId,
    pub revoked: bool,
    /// `false` means the key may spend up to the account balance.
    pub capped: bool,
    /// Invoke credits remaining on the cap. Present only when [`Self::capped`].
    pub cap_remaining: Option<u64>,
}

/// Key metadata safe to return to the holder. No secret and no hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyMeta {
    pub key_id: KeyId,
    pub revoked: bool,
    pub capped: bool,
    pub cap_remaining: Option<u64>,
}

impl From<&KeyRecord> for KeyMeta {
    fn from(record: &KeyRecord) -> Self {
        Self {
            key_id: record.key_id.clone(),
            revoked: record.revoked,
            capped: record.capped,
            cap_remaining: record.cap_remaining,
        }
    }
}

/// A newly issued key: the secret exists only on this value, not on [`KeyRecord`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewKey {
    pub secret: BearerSecret,
    pub record: KeyRecord,
}

impl NewKey {
    /// `spend_cap` is invoke credits. `None` leaves the key uncapped.
    pub fn issue(account_id: AccountId, spend_cap: Option<u64>) -> Result<Self, DomainError> {
        let secret = BearerSecret::generate()?;
        let (capped, cap_remaining) = match spend_cap {
            Some(remaining) => (true, Some(remaining)),
            None => (false, None),
        };
        Ok(Self {
            record: KeyRecord {
                secret_hash: SecretHash::of(&secret),
                account_id,
                key_id: KeyId::generate()?,
                revoked: false,
                capped,
                cap_remaining,
            },
            secret,
        })
    }
}

/// A further key may be issued only when `presented` is a non-revoked bearer
/// for `account_id`.
pub fn authorize_additional_key(
    account_id: &AccountId,
    presented: Option<&KeyRecord>,
) -> Result<(), DomainError> {
    match presented {
        Some(key) if !key.revoked && &key.account_id == account_id => Ok(()),
        _ => Err(DomainError::NotKeyHolder),
    }
}

fn random_bytes<const N: usize>() -> Result<[u8; N], DomainError> {
    let mut buf = [0u8; N];
    getrandom::fill(&mut buf).map_err(|_| DomainError::Entropy)?;
    Ok(buf)
}

fn decode_fixed(hex_str: &str) -> Result<[u8; ID_LEN], ()> {
    let bytes = hex::decode(hex_str).map_err(|_| ())?;
    if bytes.len() != ID_LEN {
        return Err(());
    }
    let mut arr = [0u8; ID_LEN];
    arr.copy_from_slice(&bytes);
    Ok(arr)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_account_balance_is_zero_invoke_credits() {
        let account = Account::open().expect("account");
        assert_eq!(account.balance, 0);
        assert_eq!(account.id.to_hex().len(), 32);
    }

    #[test]
    fn secret_is_not_recoverable_from_the_stored_record() {
        let account = Account::open().expect("account");
        let issued = NewKey::issue(account.id.clone(), None).expect("key");

        assert!(issued.secret.as_str().starts_with("nfk_"));
        assert_eq!(issued.secret.as_str().len(), 4 + 64);

        let record = &issued.record;
        let rendered = format!("{record:?}");
        assert!(
            !rendered.contains(issued.secret.as_str()),
            "stored record must not contain the secret: {rendered}"
        );
        let raw_hex = &issued.secret.as_str()[4..];
        assert!(
            !rendered.contains(raw_hex),
            "stored record must not contain the secret bytes: {rendered}"
        );
        assert_eq!(record.secret_hash, SecretHash::of(&issued.secret));
        assert_ne!(record.secret_hash.to_hex(), raw_hex);
        assert!(record.cap_remaining.is_none());
        assert!(!record.capped);
    }

    #[test]
    fn second_issue_requires_an_existing_non_revoked_bearer() {
        let account = Account::open().expect("account");
        let first = NewKey::issue(account.id.clone(), Some(5)).expect("first");
        assert!(authorize_additional_key(&account.id, None).is_err());

        let mut revoked = first.record.clone();
        revoked.revoked = true;
        assert!(authorize_additional_key(&account.id, Some(&revoked)).is_err());

        let other = Account::open().expect("other");
        assert!(authorize_additional_key(&other.id, Some(&first.record)).is_err());

        authorize_additional_key(&account.id, Some(&first.record)).expect("holder");
        let second = NewKey::issue(account.id.clone(), None).expect("second");
        assert_ne!(second.record.key_id, first.record.key_id);
        assert_ne!(second.secret.as_str(), first.secret.as_str());
        assert_eq!(second.record.account_id, account.id);
        assert_eq!(account.balance, 0);
    }
}
