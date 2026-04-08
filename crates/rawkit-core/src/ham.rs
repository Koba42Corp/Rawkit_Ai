use crate::Value;

/// Result of HAM (Hypothetical Amnesia Machine) conflict resolution.
///
/// Determines what to do with an incoming update for a single property
/// by comparing state vectors (timestamps) and values.
#[derive(Debug, Clone, PartialEq)]
pub enum HamResult {
    /// The incoming value is newer — accept and apply it.
    Accept,
    /// The incoming value is older — discard it (we already have newer data).
    Discard,
    /// The incoming value is from the future — defer processing.
    /// This handles clock skew in distributed systems.
    Defer,
    /// States are equal — use lexicographic tiebreaker on values.
    /// Returns Accept if incoming wins, Discard if current wins.
    /// This branch is resolved internally and never returned to callers.
    #[doc(hidden)]
    _Tiebreak,
}

/// The HAM conflict resolution engine.
///
/// Clean room implementation based on the published semantics:
/// - Compare timestamps (state vectors) to determine which write is newer
/// - If timestamps are equal, use deterministic lexicographic comparison
/// - If incoming timestamp exceeds machine time, defer (future state protection)
///
/// This is a per-property algorithm. Each property on a node is resolved independently.
pub struct Ham;

impl Ham {
    /// Resolve a conflict between an incoming update and the current state.
    ///
    /// # Arguments
    /// * `machine_state` - Current machine time (milliseconds since epoch)
    /// * `incoming_state` - Timestamp of the incoming update
    /// * `current_state` - Timestamp of the current local value
    /// * `incoming_value` - The incoming value
    /// * `current_value` - The current local value
    ///
    /// # Returns
    /// `HamResult::Accept` if the incoming value should replace the current one,
    /// `HamResult::Discard` if the incoming value should be ignored,
    /// `HamResult::Defer` if the incoming value is from the future.
    pub fn resolve(
        machine_state: f64,
        incoming_state: f64,
        current_state: f64,
        incoming_value: &Value,
        current_value: &Value,
    ) -> HamResult {
        // Future state protection: if the incoming state is ahead of our clock,
        // defer processing. This prevents a node with a far-future clock from
        // poisoning data across the network.
        if incoming_state > machine_state {
            return HamResult::Defer;
        }

        // If the incoming state is newer than what we have, accept it.
        if incoming_state > current_state {
            return HamResult::Accept;
        }

        // If the incoming state is older than what we have, discard it.
        if incoming_state < current_state {
            return HamResult::Discard;
        }

        // States are equal — deterministic tiebreaker using lexicographic comparison.
        // This ensures all peers converge to the same value regardless of message ordering.
        match incoming_value.lexicographic_cmp(current_value) {
            std::cmp::Ordering::Greater => HamResult::Accept,
            _ => HamResult::Discard,
        }
    }
}

/// Get the current machine time in milliseconds since Unix epoch.
pub fn now_ms() -> f64 {
    chrono::Utc::now().timestamp_millis() as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_newer_incoming_wins() {
        let result = Ham::resolve(
            1000.0,
            900.0,  // incoming is at t=900
            800.0,  // current is at t=800
            &Value::text("new"),
            &Value::text("old"),
        );
        assert_eq!(result, HamResult::Accept);
    }

    #[test]
    fn test_older_incoming_loses() {
        let result = Ham::resolve(
            1000.0,
            700.0,  // incoming is at t=700
            800.0,  // current is at t=800
            &Value::text("old"),
            &Value::text("new"),
        );
        assert_eq!(result, HamResult::Discard);
    }

    #[test]
    fn test_future_state_deferred() {
        let result = Ham::resolve(
            1000.0,
            2000.0, // incoming is from the future!
            800.0,
            &Value::text("future"),
            &Value::text("current"),
        );
        assert_eq!(result, HamResult::Defer);
    }

    #[test]
    fn test_equal_state_lexicographic_tiebreak() {
        // Same timestamp — "banana" > "apple" lexicographically, so banana wins
        let result = Ham::resolve(
            1000.0,
            500.0,
            500.0,
            &Value::text("banana"),
            &Value::text("apple"),
        );
        assert_eq!(result, HamResult::Accept);

        // Same timestamp — "apple" < "banana" lexicographically, so apple loses
        let result = Ham::resolve(
            1000.0,
            500.0,
            500.0,
            &Value::text("apple"),
            &Value::text("banana"),
        );
        assert_eq!(result, HamResult::Discard);
    }

    #[test]
    fn test_equal_state_equal_value_discards() {
        // Identical state and value — no change needed
        let result = Ham::resolve(
            1000.0,
            500.0,
            500.0,
            &Value::text("same"),
            &Value::text("same"),
        );
        assert_eq!(result, HamResult::Discard);
    }

    #[test]
    fn test_null_values() {
        // Null incoming (deletion) vs existing value
        let result = Ham::resolve(
            1000.0,
            900.0,
            800.0,
            &Value::Null,
            &Value::text("existing"),
        );
        assert_eq!(result, HamResult::Accept); // newer timestamp wins, even for deletions
    }

    #[test]
    fn test_number_tiebreak() {
        let result = Ham::resolve(
            1000.0,
            500.0,
            500.0,
            &Value::number(42.0),
            &Value::number(41.0),
        );
        assert_eq!(result, HamResult::Accept); // "42" > "41" lexicographically
    }
}
