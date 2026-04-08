use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use crate::node::Node;
use crate::Soul;

/// Trait for pluggable storage backends.
///
/// Implementations must be thread-safe (Send + Sync) to support concurrent access
/// from multiple peers and the sync engine.
pub trait StorageAdapter: Send + Sync {
    fn get(&self, soul: &str) -> Option<Node>;
    fn put(&self, soul: &str, node: &Node) -> Result<(), StorageError>;
    fn delete(&self, soul: &str) -> Result<(), StorageError>;
    fn list(&self, prefix: &str) -> Vec<Soul>;
    fn exists(&self, soul: &str) -> bool;
}

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("storage I/O error: {0}")]
    Io(String),
    #[error("serialization error: {0}")]
    Serialization(String),
    #[error("node not found: {0}")]
    NotFound(String),
}

// ---------------------------------------------------------------------------
// In-Memory Storage (for testing and ephemeral use)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct MemoryStorage {
    nodes: Arc<RwLock<HashMap<Soul, Node>>>,
}

impl MemoryStorage {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.nodes.read().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl StorageAdapter for MemoryStorage {
    fn get(&self, soul: &str) -> Option<Node> {
        self.nodes.read().unwrap().get(soul).cloned()
    }

    fn put(&self, soul: &str, node: &Node) -> Result<(), StorageError> {
        self.nodes
            .write()
            .unwrap()
            .insert(soul.to_string(), node.clone());
        Ok(())
    }

    fn delete(&self, soul: &str) -> Result<(), StorageError> {
        self.nodes.write().unwrap().remove(soul);
        Ok(())
    }

    fn list(&self, prefix: &str) -> Vec<Soul> {
        self.nodes
            .read()
            .unwrap()
            .keys()
            .filter(|k| k.starts_with(prefix))
            .cloned()
            .collect()
    }

    fn exists(&self, soul: &str) -> bool {
        self.nodes.read().unwrap().contains_key(soul)
    }
}

// ---------------------------------------------------------------------------
// SQLite Storage
// ---------------------------------------------------------------------------

#[cfg(feature = "sqlite")]
pub mod sqlite {
    use super::*;
    use rusqlite::Connection;
    use std::sync::Mutex;

    pub struct SqliteStorage {
        conn: Arc<Mutex<Connection>>,
    }

    impl SqliteStorage {
        pub fn new(path: &str) -> Result<Self, StorageError> {
            let conn =
                Connection::open(path).map_err(|e| StorageError::Io(e.to_string()))?;

            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS nodes (
                    soul TEXT PRIMARY KEY,
                    data TEXT NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_nodes_soul ON nodes(soul);
                PRAGMA journal_mode=WAL;
                PRAGMA synchronous=NORMAL;",
            )
            .map_err(|e| StorageError::Io(e.to_string()))?;

            Ok(Self {
                conn: Arc::new(Mutex::new(conn)),
            })
        }

        pub fn in_memory() -> Result<Self, StorageError> {
            Self::new(":memory:")
        }
    }

    impl StorageAdapter for SqliteStorage {
        fn get(&self, soul: &str) -> Option<Node> {
            let conn = self.conn.lock().unwrap();
            let mut stmt = conn
                .prepare("SELECT data FROM nodes WHERE soul = ?1")
                .ok()?;
            let data: String = stmt
                .query_row([soul], |row| row.get(0))
                .ok()?;
            serde_json::from_str(&data).ok()
        }

        fn put(&self, soul: &str, node: &Node) -> Result<(), StorageError> {
            let data = serde_json::to_string(node)
                .map_err(|e| StorageError::Serialization(e.to_string()))?;
            let conn = self.conn.lock().unwrap();
            conn.execute(
                "INSERT OR REPLACE INTO nodes (soul, data) VALUES (?1, ?2)",
                [soul, &data],
            )
            .map_err(|e| StorageError::Io(e.to_string()))?;
            Ok(())
        }

        fn delete(&self, soul: &str) -> Result<(), StorageError> {
            let conn = self.conn.lock().unwrap();
            conn.execute("DELETE FROM nodes WHERE soul = ?1", [soul])
                .map_err(|e| StorageError::Io(e.to_string()))?;
            Ok(())
        }

        fn list(&self, prefix: &str) -> Vec<Soul> {
            let conn = self.conn.lock().unwrap();
            let mut stmt = conn
                .prepare("SELECT soul FROM nodes WHERE soul LIKE ?1")
                .unwrap();
            let pattern = format!("{prefix}%");
            stmt.query_map([&pattern], |row| row.get(0))
                .unwrap()
                .filter_map(|r| r.ok())
                .collect()
        }

        fn exists(&self, soul: &str) -> bool {
            let conn = self.conn.lock().unwrap();
            let mut stmt = conn
                .prepare("SELECT 1 FROM nodes WHERE soul = ?1 LIMIT 1")
                .unwrap();
            stmt.exists([soul]).unwrap_or(false)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Value;

    fn run_storage_tests(storage: &dyn StorageAdapter) {
        // Put and get
        let mut node = Node::new("test/1");
        node.put("name", Value::text("Alice"), 1000.0);
        storage.put("test/1", &node).unwrap();

        let retrieved = storage.get("test/1").unwrap();
        assert_eq!(retrieved.get("name"), Some(&Value::text("Alice")));

        // Exists
        assert!(storage.exists("test/1"));
        assert!(!storage.exists("test/999"));

        // List
        let mut node2 = Node::new("test/2");
        node2.put("name", Value::text("Bob"), 1000.0);
        storage.put("test/2", &node2).unwrap();

        let list = storage.list("test/");
        assert_eq!(list.len(), 2);

        // Delete
        storage.delete("test/1").unwrap();
        assert!(!storage.exists("test/1"));
    }

    #[test]
    fn test_memory_storage() {
        let storage = MemoryStorage::new();
        run_storage_tests(&storage);
    }

    #[cfg(feature = "sqlite")]
    #[test]
    fn test_sqlite_storage() {
        let storage = sqlite::SqliteStorage::in_memory().unwrap();
        run_storage_tests(&storage);
    }
}
