use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};

use rawkit_core::Graph;
use crate::message::{Message, MessageKind, UpdateEntry};

/// Manages peer connections and syncs graph updates across the network.
///
/// Key behaviors:
/// - Applies incoming PUTs to the local graph (HAM resolves conflicts)
/// - Broadcasts local writes to connected peers
/// - Tracks which messages we've seen to prevent echo loops
/// - Manages subscriptions per peer
pub struct PeerManager {
    graph: Graph,
    /// Set of message IDs we've already processed (prevents loops).
    seen: Arc<RwLock<HashSet<String>>>,
    /// Connected peer IDs.
    peers: Arc<RwLock<Vec<String>>>,
    /// Outbound message queue per peer.
    outbox: Arc<RwLock<HashMap<String, Vec<Message>>>>,
}

impl PeerManager {
    pub fn new(graph: Graph) -> Self {
        PeerManager {
            graph,
            seen: Arc::new(RwLock::new(HashSet::new())),
            peers: Arc::new(RwLock::new(Vec::new())),
            outbox: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Register a new peer connection.
    pub fn add_peer(&self, peer_id: &str) {
        self.peers.write().unwrap().push(peer_id.to_string());
        self.outbox
            .write()
            .unwrap()
            .insert(peer_id.to_string(), Vec::new());
    }

    /// Remove a disconnected peer.
    pub fn remove_peer(&self, peer_id: &str) {
        self.peers.write().unwrap().retain(|p| p != peer_id);
        self.outbox.write().unwrap().remove(peer_id);
    }

    /// Process an incoming message from a peer.
    /// Returns an optional response message (e.g., ACK or data response).
    pub fn handle_message(&self, msg: &Message) -> Option<Message> {
        // Dedup: skip if we've seen this message ID
        {
            let mut seen = self.seen.write().unwrap();
            if seen.contains(&msg.id) {
                return None;
            }
            seen.insert(msg.id.clone());

            // Prevent unbounded growth of seen set
            if seen.len() > 100_000 {
                seen.clear();
            }
        }

        match &msg.kind {
            MessageKind::Put {
                soul, updates, ..
            } => {
                // Apply each update through HAM conflict resolution
                for (key, entry) in updates {
                    self.graph
                        .put_with_state(soul, key, entry.value.clone(), entry.state);
                }

                // Forward to other peers (gossip)
                self.broadcast(msg);

                // ACK
                Some(Message::new_ack(msg.id.clone()))
            }

            MessageKind::Get { soul, key } => {
                // Respond with current data
                if let Some(node) = self.graph.get_node(soul) {
                    let mut updates = HashMap::new();

                    match key {
                        Some(k) => {
                            if let Some(val) = node.get(k) {
                                updates.insert(
                                    k.clone(),
                                    UpdateEntry {
                                        value: val.clone(),
                                        state: node.state_of(k).unwrap_or(0.0),
                                    },
                                );
                            }
                        }
                        None => {
                            for (k, v) in node.all_entries() {
                                updates.insert(
                                    k.clone(),
                                    UpdateEntry {
                                        value: v.clone(),
                                        state: node.state_of(k).unwrap_or(0.0),
                                    },
                                );
                            }
                        }
                    }

                    if !updates.is_empty() {
                        Some(Message::new_put(soul.clone(), updates))
                    } else {
                        None
                    }
                } else {
                    None
                }
            }

            MessageKind::Ack { .. } => None,
            MessageKind::Sub { .. } | MessageKind::Unsub { .. } => None,
        }
    }

    /// Queue a message for broadcast to all peers except the sender.
    fn broadcast(&self, msg: &Message) {
        let peers = self.peers.read().unwrap();
        let mut outbox = self.outbox.write().unwrap();

        for peer_id in peers.iter() {
            if let Some(queue) = outbox.get_mut(peer_id) {
                queue.push(msg.clone());
            }
        }
    }

    /// Drain the outbox for a specific peer.
    pub fn drain_outbox(&self, peer_id: &str) -> Vec<Message> {
        self.outbox
            .write()
            .unwrap()
            .get_mut(peer_id)
            .map(|q| q.drain(..).collect())
            .unwrap_or_default()
    }

    /// Get the number of connected peers.
    pub fn peer_count(&self) -> usize {
        self.peers.read().unwrap().len()
    }

    /// Get a reference to the underlying graph.
    pub fn graph(&self) -> &Graph {
        &self.graph
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rawkit_core::Value;

    #[test]
    fn test_peer_sync() {
        // Create two peer managers with separate graphs
        let graph_a = Graph::in_memory();
        let graph_b = Graph::in_memory();

        let peer_a = PeerManager::new(graph_a.clone());
        let peer_b = PeerManager::new(graph_b.clone());

        peer_a.add_peer("b");
        peer_b.add_peer("a");

        // Peer A writes data
        graph_a.put("users/alice", "name", Value::text("Alice"));

        // Simulate: get the node from A's graph and create a PUT message
        let node = graph_a.get_node("users/alice").unwrap();
        let mut updates = HashMap::new();
        for (k, v) in node.all_entries() {
            updates.insert(
                k.clone(),
                UpdateEntry {
                    value: v.clone(),
                    state: node.state_of(k).unwrap_or(0.0),
                },
            );
        }
        let msg = Message::new_put("users/alice".to_string(), updates);

        // Peer B processes the message
        let response = peer_b.handle_message(&msg);
        assert!(response.is_some()); // should ACK

        // Peer B should now have the data
        assert_eq!(
            graph_b.get("users/alice", "name"),
            Some(Value::text("Alice"))
        );
    }

    #[test]
    fn test_get_response() {
        let graph = Graph::in_memory();
        graph.put("test", "value", Value::text("hello"));

        let peer = PeerManager::new(graph);
        let get_msg = Message::new_get("test".to_string(), Some("value".to_string()));

        let response = peer.handle_message(&get_msg);
        assert!(response.is_some());

        match response.unwrap().kind {
            MessageKind::Put { soul, updates, .. } => {
                assert_eq!(soul, "test");
                assert_eq!(updates["value"].value, Value::text("hello"));
            }
            _ => panic!("expected Put response"),
        }
    }

    #[test]
    fn test_dedup_prevents_echo() {
        let graph = Graph::in_memory();
        let peer = PeerManager::new(graph);

        let msg = Message::new_put(
            "test".to_string(),
            HashMap::from([(
                "key".to_string(),
                UpdateEntry {
                    value: Value::text("val"),
                    state: 1000.0,
                },
            )]),
        );

        // First time: processes
        assert!(peer.handle_message(&msg).is_some());
        // Second time: deduped
        assert!(peer.handle_message(&msg).is_none());
    }
}
