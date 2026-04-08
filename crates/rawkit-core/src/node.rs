use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::Value;

/// A soul is the globally unique identifier for a node in the graph.
pub type Soul = String;

/// Per-property state tracking for HAM conflict resolution.
/// Maps property name -> timestamp of last write.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StateVector(pub HashMap<String, f64>);

impl StateVector {
    pub fn get(&self, key: &str) -> Option<f64> {
        self.0.get(key).copied()
    }

    pub fn set(&mut self, key: &str, state: f64) {
        self.0.insert(key.to_string(), state);
    }
}

/// Metadata attached to every node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeMeta {
    /// The node's unique identifier.
    #[serde(rename = "#")]
    pub soul: Soul,
    /// State vector: per-property timestamps for conflict resolution.
    #[serde(rename = ">")]
    pub state: StateVector,
}

/// A node in the Rawkit graph.
///
/// Each node has a soul (unique ID), properties (key-value pairs where values
/// are primitives or links to other nodes), and metadata tracking the state
/// of each property for CRDT conflict resolution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    /// Reserved metadata field.
    #[serde(rename = "_")]
    pub meta: NodeMeta,
    /// Properties of this node. Keys are property names, values are primitives or links.
    #[serde(flatten)]
    pub props: HashMap<String, Value>,
}

impl Node {
    /// Create a new empty node with the given soul.
    pub fn new(soul: impl Into<Soul>) -> Self {
        let soul = soul.into();
        Node {
            meta: NodeMeta {
                soul,
                state: StateVector::default(),
            },
            props: HashMap::new(),
        }
    }

    /// Create a new node with an auto-generated UUID soul.
    pub fn new_auto() -> Self {
        Self::new(uuid::Uuid::new_v4().to_string())
    }

    pub fn soul(&self) -> &str {
        &self.meta.soul
    }

    /// Get a property value.
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.props.get(key)
    }

    /// Set a property value and update the state vector.
    pub fn put(&mut self, key: impl Into<String>, value: Value, state: f64) {
        let key = key.into();
        self.meta.state.set(&key, state);
        self.props.insert(key, value);
    }

    /// Delete a property by setting it to null (tombstone).
    pub fn delete(&mut self, key: &str, state: f64) {
        self.put(key.to_string(), Value::Null, state);
    }

    /// Get the state (timestamp) for a specific property.
    pub fn state_of(&self, key: &str) -> Option<f64> {
        self.meta.state.get(key)
    }

    /// Iterate over all non-null properties.
    pub fn entries(&self) -> impl Iterator<Item = (&String, &Value)> {
        self.props.iter().filter(|(_, v)| !v.is_null())
    }

    /// Iterate over all properties including tombstones.
    pub fn all_entries(&self) -> impl Iterator<Item = (&String, &Value)> {
        self.props.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_node() {
        let node = Node::new("user/alice");
        assert_eq!(node.soul(), "user/alice");
        assert!(node.props.is_empty());
    }

    #[test]
    fn test_put_and_get() {
        let mut node = Node::new("test");
        node.put("name", Value::text("Alice"), 1000.0);
        assert_eq!(node.get("name"), Some(&Value::text("Alice")));
        assert_eq!(node.state_of("name"), Some(1000.0));
    }

    #[test]
    fn test_delete_creates_tombstone() {
        let mut node = Node::new("test");
        node.put("name", Value::text("Alice"), 1000.0);
        node.delete("name", 2000.0);
        assert_eq!(node.get("name"), Some(&Value::Null));
        assert_eq!(node.state_of("name"), Some(2000.0));
        // entries() should skip tombstones
        assert_eq!(node.entries().count(), 0);
        // all_entries() should include them
        assert_eq!(node.all_entries().count(), 1);
    }

    #[test]
    fn test_serialization() {
        let mut node = Node::new("user/alice");
        node.put("name", Value::text("Alice"), 1000.0);
        node.put("age", Value::number(30.0), 1000.0);

        let json = serde_json::to_string_pretty(&node).unwrap();
        assert!(json.contains("user/alice"));
        assert!(json.contains("Alice"));

        let deserialized: Node = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.soul(), "user/alice");
        assert_eq!(deserialized.get("name"), Some(&Value::text("Alice")));
    }
}
