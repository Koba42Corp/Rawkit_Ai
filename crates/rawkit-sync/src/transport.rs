use crate::message::Message;

/// Trait for pluggable network transports.
///
/// Implementations handle the actual bytes-on-wire, allowing the peer manager
/// to be transport-agnostic. Ship with WebSocket; add WebRTC later.
#[async_trait::async_trait]
pub trait Transport: Send + Sync {
    async fn send(&self, msg: &Message) -> Result<(), TransportError>;
    async fn recv(&self) -> Result<Message, TransportError>;
    async fn close(&self) -> Result<(), TransportError>;
    fn peer_id(&self) -> &str;
    fn is_connected(&self) -> bool;
}

#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    #[error("connection failed: {0}")]
    ConnectionFailed(String),
    #[error("send failed: {0}")]
    SendFailed(String),
    #[error("receive failed: {0}")]
    ReceiveFailed(String),
    #[error("connection closed")]
    Closed,
    #[error("serialization error: {0}")]
    Serialization(String),
}

// async_trait is not in Cargo.toml yet — we'll use a simpler approach for now
// and add the full WebSocket transport implementation when we build the server.
