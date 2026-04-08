use serde::{Deserialize, Serialize};

use crate::Soul;

/// Primitive value types in the Rawkit graph.
/// Intentionally constrained — no nested objects (those become separate nodes with references).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Value {
    Null,
    Bool(bool),
    Number(f64),
    Text(String),
    /// A reference (link) to another node by its soul.
    Link(Link),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Link {
    #[serde(rename = "#")]
    pub soul: Soul,
}

impl Value {
    pub fn text(s: impl Into<String>) -> Self {
        Value::Text(s.into())
    }

    pub fn number(n: f64) -> Self {
        Value::Number(n)
    }

    pub fn link(soul: impl Into<Soul>) -> Self {
        Value::Link(Link { soul: soul.into() })
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    pub fn as_text(&self) -> Option<&str> {
        match self {
            Value::Text(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_number(&self) -> Option<f64> {
        match self {
            Value::Number(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_link(&self) -> Option<&Soul> {
        match self {
            Value::Link(link) => Some(&link.soul),
            _ => None,
        }
    }

    /// Lexicographic comparison for HAM tiebreaking.
    /// Converts to a canonical string representation for deterministic ordering.
    pub fn lexicographic_cmp(&self, other: &Value) -> std::cmp::Ordering {
        let a = self.to_lex_string();
        let b = other.to_lex_string();
        a.cmp(&b)
    }

    fn to_lex_string(&self) -> String {
        match self {
            Value::Null => String::new(),
            Value::Bool(b) => b.to_string(),
            Value::Number(n) => format!("{n}"),
            Value::Text(s) => s.clone(),
            Value::Link(link) => format!("~{}", link.soul),
        }
    }
}

impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Value::Text(s.to_string())
    }
}

impl From<String> for Value {
    fn from(s: String) -> Self {
        Value::Text(s)
    }
}

impl From<f64> for Value {
    fn from(n: f64) -> Self {
        Value::Number(n)
    }
}

impl From<i64> for Value {
    fn from(n: i64) -> Self {
        Value::Number(n as f64)
    }
}

impl From<bool> for Value {
    fn from(b: bool) -> Self {
        Value::Bool(b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_value_conversions() {
        assert_eq!(Value::from("hello"), Value::Text("hello".to_string()));
        assert_eq!(Value::from(42i64), Value::Number(42.0));
        assert_eq!(Value::from(true), Value::Bool(true));
    }

    #[test]
    fn test_lexicographic_ordering() {
        let a = Value::text("apple");
        let b = Value::text("banana");
        assert_eq!(a.lexicographic_cmp(&b), std::cmp::Ordering::Less);
    }

    #[test]
    fn test_link_serialization() {
        let link = Value::link("abc123");
        let json = serde_json::to_string(&link).unwrap();
        assert!(json.contains("\"#\":\"abc123\""));
    }
}
