//! Multi-hop signal relay (Spec Section 15).
//!
//! Enables signals to traverse intermediate SOMA nodes when the
//! recipient is not directly connected to the sender.

use super::signal::Signal;

/// Check if a signal should be relayed (recipient != us).
pub fn should_relay(signal: &Signal, our_id: &str) -> bool {
    let recipient = signal
        .metadata
        .get("recipient")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    !recipient.is_empty() && recipient != our_id
}

/// Prepare a signal for relay: increment `hop_count`, append to `relay_path`.
///
/// Returns an error if max hops would be exceeded or if a relay loop
/// is detected (`our_id` already appears in the `relay_path`).
pub fn prepare_relay(signal: &mut Signal, our_id: &str) -> Result<(), &'static str> {
    let metadata = signal
        .metadata
        .as_object_mut()
        .ok_or("invalid metadata")?;

    // Check max_hops
    #[allow(clippy::cast_possible_truncation)] // hop counts are small values
    let max_hops = metadata
        .get("max_hops")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(3) as u32;
    #[allow(clippy::cast_possible_truncation)] // hop counts are small values
    let hop_count = metadata
        .get("hop_count")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0) as u32;

    if hop_count >= max_hops {
        return Err("max hops exceeded");
    }

    // Check loop prevention
    let relay_path = metadata
        .get("relay_path")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    if relay_path.contains(&our_id) {
        return Err("relay loop detected");
    }

    // Update relay_path
    let mut new_path: Vec<serde_json::Value> = relay_path
        .iter()
        .map(|s| serde_json::Value::String(s.to_string()))
        .collect();
    new_path.push(serde_json::Value::String(our_id.to_string()));
    metadata.insert(
        "relay_path".into(),
        serde_json::Value::Array(new_path),
    );

    // Increment hop_count
    metadata.insert("hop_count".into(), serde_json::json!(hop_count + 1));

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::signal::{Signal, SignalType};

    fn make_signal_with_recipient(recipient: &str) -> Signal {
        let mut s = Signal::new(SignalType::Data, "sender-a".to_string());
        if let serde_json::Value::Object(ref mut map) = s.metadata {
            map.insert(
                "recipient".into(),
                serde_json::Value::String(recipient.to_string()),
            );
        }
        s
    }

    #[test]
    fn test_should_relay_true() {
        let signal = make_signal_with_recipient("soma-c");
        assert!(should_relay(&signal, "soma-b"));
    }

    #[test]
    fn test_should_relay_false_when_recipient_is_us() {
        let signal = make_signal_with_recipient("soma-b");
        assert!(!should_relay(&signal, "soma-b"));
    }

    #[test]
    fn test_should_relay_false_when_no_recipient() {
        let signal = Signal::new(SignalType::Data, "sender-a".to_string());
        assert!(!should_relay(&signal, "soma-b"));
    }

    #[test]
    fn test_prepare_relay_increments_hop_count() {
        let mut signal = make_signal_with_recipient("soma-d");
        prepare_relay(&mut signal, "soma-b").unwrap();

        let hop_count = signal
            .metadata
            .get("hop_count")
            .and_then(|v| v.as_u64())
            .unwrap();
        assert_eq!(hop_count, 1);

        let relay_path = signal
            .metadata
            .get("relay_path")
            .and_then(|v| v.as_array())
            .unwrap();
        assert_eq!(relay_path.len(), 1);
        assert_eq!(relay_path[0].as_str().unwrap(), "soma-b");
    }

    #[test]
    fn test_prepare_relay_max_hops_exceeded() {
        let mut signal = make_signal_with_recipient("soma-d");
        if let serde_json::Value::Object(ref mut map) = signal.metadata {
            map.insert("max_hops".into(), serde_json::json!(2));
            map.insert("hop_count".into(), serde_json::json!(2));
        }

        let result = prepare_relay(&mut signal, "soma-c");
        assert_eq!(result, Err("max hops exceeded"));
    }

    #[test]
    fn test_prepare_relay_loop_detected() {
        let mut signal = make_signal_with_recipient("soma-d");
        if let serde_json::Value::Object(ref mut map) = signal.metadata {
            map.insert(
                "relay_path".into(),
                serde_json::json!(["soma-a", "soma-b"]),
            );
            map.insert("hop_count".into(), serde_json::json!(2));
        }

        let result = prepare_relay(&mut signal, "soma-b");
        assert_eq!(result, Err("relay loop detected"));
    }

    #[test]
    fn test_prepare_relay_multi_hop() {
        let mut signal = make_signal_with_recipient("soma-d");
        prepare_relay(&mut signal, "soma-b").unwrap();
        prepare_relay(&mut signal, "soma-c").unwrap();

        let hop_count = signal
            .metadata
            .get("hop_count")
            .and_then(|v| v.as_u64())
            .unwrap();
        assert_eq!(hop_count, 2);

        let relay_path = signal
            .metadata
            .get("relay_path")
            .and_then(|v| v.as_array())
            .unwrap();
        assert_eq!(relay_path.len(), 2);
        assert_eq!(relay_path[0].as_str().unwrap(), "soma-b");
        assert_eq!(relay_path[1].as_str().unwrap(), "soma-c");
    }
}
