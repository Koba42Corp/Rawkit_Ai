use std::collections::HashMap;

use rawkit_core::{Soul, Value};
use serde::{Deserialize, Serialize};

/// Wire protocol message types.
///
/// Rawkit uses a simple two-command protocol (like Gun.js) with additions
/// for subscriptions and acknowledgments:
///
/// - PUT: Write data (includes state vector for HAM resolution)
/// - GET: Request data for a soul/key
/// - ACK: Acknowledge a PUT (prevents echo loops)
/// - SUB: Subscribe to changes on a path pattern
/// - UNSUB: Unsubscribe
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    /// Unique message ID for deduplication and ACK correlation.
    #[serde(rename = "#")]
    pub id: String,
    /// The message payload.
    #[serde(flatten)]
    pub kind: MessageKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum MessageKind {
    /// Write data to the graph.
    #[serde(rename = "put")]
    Put {
        /// The node soul being written.
        soul: Soul,
        /// Property updates: key -> (value, state_timestamp).
        updates: HashMap<String, UpdateEntry>,
        /// Optional: Ed25519 signature over the update (base64).
        #[serde(skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
        /// Optional: signer's public key (hex).
        #[serde(skip_serializing_if = "Option::is_none")]
        signer: Option<String>,
    },

    /// Request data.
    #[serde(rename = "get")]
    Get {
        /// The soul to retrieve.
        soul: Soul,
        /// Optional: specific property key. None = entire node.
        #[serde(skip_serializing_if = "Option::is_none")]
        key: Option<String>,
    },

    /// Acknowledge receipt of a PUT.
    #[serde(rename = "ack")]
    Ack {
        /// The message ID being acknowledged.
        #[serde(rename = "ok")]
        message_id: String,
    },

    /// Subscribe to changes on a path.
    #[serde(rename = "sub")]
    Sub {
        /// Path pattern to subscribe to (supports trailing * wildcard).
        path: String,
    },

    /// Unsubscribe from a path.
    #[serde(rename = "unsub")]
    Unsub { path: String },
}

/// A single property update with its HAM state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateEntry {
    /// The property value.
    #[serde(rename = "v")]
    pub value: Value,
    /// The HAM state (timestamp) for this property.
    #[serde(rename = "s")]
    pub state: f64,
}

impl Message {
    pub fn new_put(soul: Soul, updates: HashMap<String, UpdateEntry>) -> Self {
        Message {
            id: uuid::Uuid::new_v4().to_string(),
            kind: MessageKind::Put {
                soul,
                updates,
                signature: None,
                signer: None,
            },
        }
    }

    pub fn new_get(soul: Soul, key: Option<String>) -> Self {
        Message {
            id: uuid::Uuid::new_v4().to_string(),
            kind: MessageKind::Get { soul, key },
        }
    }

    pub fn new_ack(message_id: String) -> Self {
        Message {
            id: uuid::Uuid::new_v4().to_string(),
            kind: MessageKind::Ack { message_id },
        }
    }

    pub fn new_sub(path: String) -> Self {
        Message {
            id: uuid::Uuid::new_v4().to_string(),
            kind: MessageKind::Sub { path },
        }
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_put_message_serialization() {
        let mut updates = HashMap::new();
        updates.insert(
            "name".to_string(),
            UpdateEntry {
                value: Value::text("Alice"),
                state: 1000.0,
            },
        );

        let msg = Message::new_put("users/alice".to_string(), updates);
        let json = msg.to_json();
        let parsed = Message::from_json(&json).unwrap();

        match parsed.kind {
            MessageKind::Put { soul, updates, .. } => {
                assert_eq!(soul, "users/alice");
                assert_eq!(updates["name"].value, Value::text("Alice"));
                assert_eq!(updates["name"].state, 1000.0);
            }
            _ => panic!("expected Put message"),
        }
    }

    #[test]
    fn test_get_message() {
        let msg = Message::new_get("users/alice".to_string(), Some("name".to_string()));
        let json = msg.to_json();
        let parsed = Message::from_json(&json).unwrap();

        match parsed.kind {
            MessageKind::Get { soul, key } => {
                assert_eq!(soul, "users/alice");
                assert_eq!(key, Some("name".to_string()));
            }
            _ => panic!("expected Get message"),
        }
    }
}
