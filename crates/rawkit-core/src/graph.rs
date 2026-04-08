use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use crate::ham::{self, Ham, HamResult};
use crate::node::Node;
use crate::storage::{MemoryStorage, StorageAdapter};
use crate::value::Value;
use crate::Soul;

/// Callback type for subscriptions.
pub type ChangeCallback = Box<dyn Fn(&str, &str, &Value) + Send + Sync>;

/// A subscription handle — drop to unsubscribe.
pub struct Subscription {
    id: u64,
    graph: Arc<GraphInner>,
}

impl Drop for Subscription {
    fn drop(&mut self) {
        self.graph.listeners.write().unwrap().remove(&self.id);
    }
}

struct GraphInner {
    storage: Box<dyn StorageAdapter>,
    /// Pending updates deferred by HAM (future timestamps).
    deferred: RwLock<Vec<DeferredUpdate>>,
    /// Active subscriptions: listener_id -> (soul_pattern, callback).
    listeners: RwLock<HashMap<u64, (String, ChangeCallback)>>,
    next_listener_id: RwLock<u64>,
}

#[derive(Debug)]
struct DeferredUpdate {
    soul: Soul,
    key: String,
    value: Value,
    state: f64,
}

/// The Rawkit graph database.
///
/// Provides the core API for reading and writing data, with HAM-based
/// conflict resolution and pluggable storage backends.
///
/// # Example
/// ```
/// use rawkit_core::{Graph, Value};
///
/// let graph = Graph::in_memory();
/// graph.put("users/alice", "name", Value::text("Alice"));
/// let val = graph.get("users/alice", "name");
/// assert_eq!(val, Some(Value::text("Alice")));
/// ```
pub struct Graph {
    inner: Arc<GraphInner>,
}

impl Graph {
    /// Create a graph with the given storage backend.
    pub fn new(storage: Box<dyn StorageAdapter>) -> Self {
        Graph {
            inner: Arc::new(GraphInner {
                storage,
                deferred: RwLock::new(Vec::new()),
                listeners: RwLock::new(HashMap::new()),
                next_listener_id: RwLock::new(0),
            }),
        }
    }

    /// Create a graph using in-memory storage (for testing / ephemeral use).
    pub fn in_memory() -> Self {
        Self::new(Box::new(MemoryStorage::new()))
    }

    /// Create a graph using SQLite storage.
    #[cfg(feature = "sqlite")]
    pub fn sqlite(path: &str) -> Result<Self, crate::storage::StorageError> {
        let storage = crate::storage::sqlite::SqliteStorage::new(path)?;
        Ok(Self::new(Box::new(storage)))
    }

    /// Get a single property value from a node.
    pub fn get(&self, soul: &str, key: &str) -> Option<Value> {
        self.inner
            .storage
            .get(soul)
            .and_then(|node| node.get(key).cloned())
            .filter(|v| !v.is_null())
    }

    /// Get an entire node.
    pub fn get_node(&self, soul: &str) -> Option<Node> {
        self.inner.storage.get(soul)
    }

    /// Write a property to a node, creating the node if it doesn't exist.
    /// Uses the current machine time as the state.
    pub fn put(&self, soul: &str, key: &str, value: Value) {
        let state = ham::now_ms();
        self.put_with_state(soul, key, value, state);
    }

    /// Write a property with an explicit state (timestamp).
    /// Used by the sync engine when applying remote updates.
    pub fn put_with_state(&self, soul: &str, key: &str, value: Value, state: f64) {
        let machine_state = ham::now_ms();

        // Get current node state for conflict resolution
        let current_node = self.inner.storage.get(soul);
        let (current_state, current_value) = current_node
            .as_ref()
            .map(|n| {
                (
                    n.state_of(key).unwrap_or(0.0),
                    n.get(key).cloned().unwrap_or(Value::Null),
                )
            })
            .unwrap_or((0.0, Value::Null));

        match Ham::resolve(machine_state, state, current_state, &value, &current_value) {
            HamResult::Accept => {
                let mut node = current_node.unwrap_or_else(|| Node::new(soul));
                node.put(key, value.clone(), state);
                self.inner.storage.put(soul, &node).ok();
                self.notify_listeners(soul, key, &value);
            }
            HamResult::Defer => {
                self.inner.deferred.write().unwrap().push(DeferredUpdate {
                    soul: soul.to_string(),
                    key: key.to_string(),
                    value,
                    state,
                });
            }
            HamResult::Discard | HamResult::_Tiebreak => {
                // Incoming update is older or tied — ignore it.
            }
        }
    }

    /// Write multiple properties to a node at once.
    pub fn put_multi(&self, soul: &str, props: HashMap<String, Value>) {
        let state = ham::now_ms();
        for (key, value) in props {
            self.put_with_state(soul, &key, value, state);
        }
    }

    /// Delete a property (sets it to null with a new timestamp).
    pub fn delete(&self, soul: &str, key: &str) {
        self.put(soul, key, Value::Null);
    }

    /// Delete an entire node.
    pub fn delete_node(&self, soul: &str) {
        self.inner.storage.delete(soul).ok();
    }

    /// Add an item to a set (unordered collection) on a node.
    /// Returns the soul of the new set entry.
    pub fn set(&self, soul: &str, value: Value) -> Soul {
        let entry_soul = uuid::Uuid::new_v4().to_string();
        self.put(soul, &entry_soul, Value::link(&entry_soul));
        // Store the actual value in the entry node
        self.put(&entry_soul, "value", value);
        entry_soul
    }

    /// List all nodes with a given prefix.
    pub fn list(&self, prefix: &str) -> Vec<Soul> {
        self.inner.storage.list(prefix)
    }

    /// Subscribe to changes on a node. The callback fires whenever
    /// a property on the matching soul changes.
    ///
    /// The returned `Subscription` handle keeps the listener alive.
    /// Drop it to unsubscribe.
    pub fn on(&self, soul: &str, callback: ChangeCallback) -> Subscription {
        let mut id_lock = self.inner.next_listener_id.write().unwrap();
        let id = *id_lock;
        *id_lock += 1;

        self.inner
            .listeners
            .write()
            .unwrap()
            .insert(id, (soul.to_string(), callback));

        Subscription {
            id,
            graph: Arc::clone(&self.inner),
        }
    }

    /// Get current value once (equivalent to Gun's .once()).
    pub fn once(&self, soul: &str, key: &str) -> Option<Value> {
        self.get(soul, key)
    }

    /// Process any deferred updates whose timestamps are now in the past.
    pub fn process_deferred(&self) {
        let machine_state = ham::now_ms();
        let mut deferred = self.inner.deferred.write().unwrap();
        let still_deferred: Vec<DeferredUpdate> = Vec::new();

        let updates: Vec<DeferredUpdate> = deferred.drain(..).collect();
        drop(deferred);

        for update in updates {
            if update.state <= machine_state {
                self.put_with_state(&update.soul, &update.key, update.value, update.state);
            } else {
                self.inner.deferred.write().unwrap().push(update);
            }
        }

        let _ = still_deferred; // consumed above
    }

    fn notify_listeners(&self, soul: &str, key: &str, value: &Value) {
        let listeners = self.inner.listeners.read().unwrap();
        for (_, (pattern, callback)) in listeners.iter() {
            if soul == pattern || soul.starts_with(&format!("{pattern}/")) {
                callback(soul, key, value);
            }
        }
    }
}

impl Clone for Graph {
    fn clone(&self) -> Self {
        Graph {
            inner: Arc::clone(&self.inner),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn test_basic_put_get() {
        let graph = Graph::in_memory();
        graph.put("users/alice", "name", Value::text("Alice"));
        graph.put("users/alice", "age", Value::number(30.0));

        assert_eq!(
            graph.get("users/alice", "name"),
            Some(Value::text("Alice"))
        );
        assert_eq!(
            graph.get("users/alice", "age"),
            Some(Value::number(30.0))
        );
        assert_eq!(graph.get("users/alice", "missing"), None);
    }

    #[test]
    fn test_put_overwrites_with_newer_state() {
        let graph = Graph::in_memory();
        graph.put("test", "val", Value::text("first"));

        // A small delay ensures the second write has a newer timestamp
        std::thread::sleep(std::time::Duration::from_millis(2));
        graph.put("test", "val", Value::text("second"));

        assert_eq!(graph.get("test", "val"), Some(Value::text("second")));
    }

    #[test]
    fn test_delete_property() {
        let graph = Graph::in_memory();
        graph.put("test", "name", Value::text("Alice"));
        std::thread::sleep(std::time::Duration::from_millis(2));
        graph.delete("test", "name");
        assert_eq!(graph.get("test", "name"), None);
    }

    #[test]
    fn test_set_creates_collection() {
        let graph = Graph::in_memory();
        let soul1 = graph.set("messages/general", Value::text("Hello"));
        let soul2 = graph.set("messages/general", Value::text("World"));

        assert_ne!(soul1, soul2);

        // The set entries should be linked from the parent node
        let parent = graph.get_node("messages/general").unwrap();
        assert!(parent.get(&soul1).is_some());
        assert!(parent.get(&soul2).is_some());

        // And the entry nodes should contain the actual values
        assert_eq!(graph.get(&soul1, "value"), Some(Value::text("Hello")));
        assert_eq!(graph.get(&soul2, "value"), Some(Value::text("World")));
    }

    #[test]
    fn test_subscription() {
        let graph = Graph::in_memory();
        let count = Arc::new(AtomicUsize::new(0));
        let count_clone = Arc::clone(&count);

        let _sub = graph.on("users/alice", Box::new(move |_soul, _key, _val| {
            count_clone.fetch_add(1, Ordering::SeqCst);
        }));

        graph.put("users/alice", "name", Value::text("Alice"));
        graph.put("users/alice", "age", Value::number(30.0));
        graph.put("users/bob", "name", Value::text("Bob")); // different soul, shouldn't fire

        assert_eq!(count.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn test_subscription_drop_unsubscribes() {
        let graph = Graph::in_memory();
        let count = Arc::new(AtomicUsize::new(0));
        let count_clone = Arc::clone(&count);

        let sub = graph.on("test", Box::new(move |_, _, _| {
            count_clone.fetch_add(1, Ordering::SeqCst);
        }));

        graph.put("test", "a", Value::text("1"));
        assert_eq!(count.load(Ordering::SeqCst), 1);

        drop(sub);
        graph.put("test", "b", Value::text("2"));
        assert_eq!(count.load(Ordering::SeqCst), 1); // no change after unsubscribe
    }

    #[test]
    fn test_ham_conflict_resolution_newer_wins() {
        let graph = Graph::in_memory();

        // Write with explicit states to simulate distributed conflict
        graph.put_with_state("test", "name", Value::text("Alice"), 1000.0);
        graph.put_with_state("test", "name", Value::text("Bob"), 900.0); // older, should lose

        assert_eq!(graph.get("test", "name"), Some(Value::text("Alice")));
    }

    #[test]
    fn test_list_by_prefix() {
        let graph = Graph::in_memory();
        graph.put("users/alice", "name", Value::text("Alice"));
        graph.put("users/bob", "name", Value::text("Bob"));
        graph.put("posts/1", "title", Value::text("Hello"));

        let users = graph.list("users/");
        assert_eq!(users.len(), 2);

        let posts = graph.list("posts/");
        assert_eq!(posts.len(), 1);
    }

    #[test]
    fn test_put_multi() {
        let graph = Graph::in_memory();
        let mut props = HashMap::new();
        props.insert("name".to_string(), Value::text("Alice"));
        props.insert("age".to_string(), Value::number(30.0));
        props.insert("active".to_string(), Value::Bool(true));

        graph.put_multi("users/alice", props);

        assert_eq!(
            graph.get("users/alice", "name"),
            Some(Value::text("Alice"))
        );
        assert_eq!(
            graph.get("users/alice", "age"),
            Some(Value::number(30.0))
        );
        assert_eq!(
            graph.get("users/alice", "active"),
            Some(Value::Bool(true))
        );
    }

    #[cfg(feature = "sqlite")]
    #[test]
    fn test_sqlite_graph() {
        let graph = Graph::sqlite(":memory:").unwrap();
        graph.put("test", "name", Value::text("SQLite works"));
        assert_eq!(
            graph.get("test", "name"),
            Some(Value::text("SQLite works"))
        );
    }
}
