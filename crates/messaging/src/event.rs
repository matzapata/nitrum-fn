use domain::PublishQueuedEvent;
use serde::Deserialize;

use application::AppError;

/// SNS→SQS envelope when raw message delivery is off.
#[derive(Debug, Deserialize)]
struct SnsEnvelope {
    #[serde(rename = "Message")]
    message: String,
}

/// Parse a queue body: raw `PublishQueuedEvent` JSON, or an SNS envelope wrapping it.
pub fn parse_queued_event(body: &str) -> Result<PublishQueuedEvent, AppError> {
    if let Ok(event) = serde_json::from_str::<PublishQueuedEvent>(body) {
        return Ok(event);
    }
    let envelope: SnsEnvelope = serde_json::from_str(body)
        .map_err(|e| AppError::Storage(format!("invalid queue message: {e}")))?;
    serde_json::from_str(&envelope.message)
        .map_err(|e| AppError::Storage(format!("invalid SNS Message payload: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_raw_event() {
        let raw = r#"{"function":"echo","content_hash":"ab","wasm_bytes":1}"#;
        let event = parse_queued_event(raw).expect("raw");
        assert_eq!(event.function, "echo");
        assert_eq!(event.wasm_bytes, 1);
        assert_eq!(event.queued_at_ms, 0);
    }

    #[test]
    fn parses_sns_envelope() {
        let body = r#"{"Type":"Notification","Message":"{\"function\":\"echo\",\"content_hash\":\"ab\",\"wasm_bytes\":2,\"queued_at_ms\":9}"}"#;
        let event = parse_queued_event(body).expect("envelope");
        assert_eq!(event.function, "echo");
        assert_eq!(event.wasm_bytes, 2);
        assert_eq!(event.queued_at_ms, 9);
    }

    #[test]
    fn rejects_non_event_json() {
        let err = parse_queued_event(r#"{"hello":"world"}"#).expect_err("poison");
        assert!(err.to_string().contains("invalid"), "{err}");
    }
}
