use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use clap::{Parser, Subcommand};
use futures_util::{SinkExt, StreamExt};
use rawkit_core::{Graph, Value};
use rawkit_sync::message::{Message, MessageKind, UpdateEntry};
use rawkit_sync::PeerManager;
use rawkit_vectors::VectorIndex;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast;
use tokio_tungstenite::tungstenite;
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "rawkit")]
#[command(about = "Rawkit - Decentralized vector-graph memory for AI agents")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Path to SQLite database file (used by put/get/ls/serve).
    #[arg(short, long, default_value = "rawkit.db", global = true)]
    db: String,
}

#[derive(Subcommand)]
enum Commands {
    /// Start a Rawkit relay server.
    Serve {
        /// Port to listen on.
        #[arg(short, long, default_value_t = 8765)]
        port: u16,
    },
    /// Write a value to the graph.
    Put {
        /// Node soul (e.g., "users/alice").
        soul: String,
        /// Property key.
        key: String,
        /// Value (JSON string).
        value: String,
    },
    /// Read a value from the graph.
    Get {
        /// Node soul.
        soul: String,
        /// Optional property key. Omit to get entire node.
        key: Option<String>,
    },
    /// List nodes by prefix.
    #[command(name = "ls")]
    List {
        /// Path prefix to list.
        prefix: String,
    },
    /// Connect to a relay and sync.
    Sync {
        /// Relay WebSocket URL (e.g., ws://localhost:8765).
        url: String,
    },
    /// Run benchmarks comparing Rawkit performance.
    Bench {
        /// Number of operations to run.
        #[arg(short, long, default_value_t = 100_000)]
        ops: usize,
    },
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Serve { port } => {
            run_server(port, &cli.db).await;
        }

        Commands::Put { soul, key, value } => {
            let graph = Graph::sqlite(&cli.db).expect("Failed to open database");
            let parsed: Value = serde_json::from_str(&value).unwrap_or(Value::text(&value));
            graph.put(&soul, &key, parsed.clone());
            println!(
                "PUT {soul}.{key} = {}",
                serde_json::to_string(&parsed).unwrap()
            );
        }

        Commands::Get { soul, key } => {
            let graph = Graph::sqlite(&cli.db).expect("Failed to open database");
            match key {
                Some(k) => match graph.get(&soul, &k) {
                    Some(val) => {
                        println!("{soul}.{k} = {}", serde_json::to_string(&val).unwrap())
                    }
                    None => println!("(not found)"),
                },
                None => match graph.get_node(&soul) {
                    Some(node) => {
                        println!("{soul}:");
                        for (k, v) in node.entries() {
                            println!("  {k} = {}", serde_json::to_string(v).unwrap());
                        }
                    }
                    None => println!("(not found)"),
                },
            }
        }

        Commands::List { prefix } => {
            let graph = Graph::sqlite(&cli.db).expect("Failed to open database");
            let souls = graph.list(&prefix);
            if souls.is_empty() {
                println!("No nodes found with prefix: {prefix}");
            } else {
                for soul in &souls {
                    println!("{soul}");
                }
            }
        }

        Commands::Sync { url } => {
            run_client_sync(&url, &cli.db).await;
        }

        Commands::Bench { ops } => {
            run_benchmarks(ops);
        }
    }
}

// ─── WebSocket Relay Server ───────────────────────────────────────────────────

async fn run_server(port: u16, db_path: &str) {
    let graph = Graph::sqlite(db_path).expect("Failed to open database");
    let peer_manager = Arc::new(PeerManager::new(graph));
    let (broadcast_tx, _) = broadcast::channel::<(String, String)>(1024); // (sender_id, json_msg)

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = TcpListener::bind(&addr)
        .await
        .expect("Failed to bind address");

    println!();
    println!("  ╔══════════════════════════════════════════════════╗");
    println!("  ║            RAWKIT RELAY SERVER v0.1.0            ║");
    println!("  ╚══════════════════════════════════════════════════╝");
    println!();
    println!("  Listening on:  ws://0.0.0.0:{port}");
    println!("  Database:      {db_path}");
    println!("  Press Ctrl+C to stop.");
    println!();

    loop {
        tokio::select! {
            Ok((stream, addr)) = listener.accept() => {
                let pm = Arc::clone(&peer_manager);
                let tx = broadcast_tx.clone();
                let rx = broadcast_tx.subscribe();
                tokio::spawn(handle_connection(stream, addr, pm, tx, rx));
            }
            _ = tokio::signal::ctrl_c() => {
                println!("\n  Shutting down.");
                break;
            }
        }
    }
}

async fn handle_connection(
    stream: TcpStream,
    addr: SocketAddr,
    peer_manager: Arc<PeerManager>,
    broadcast_tx: broadcast::Sender<(String, String)>,
    mut broadcast_rx: broadcast::Receiver<(String, String)>,
) {
    let ws_stream = match tokio_tungstenite::accept_async(stream).await {
        Ok(ws) => ws,
        Err(e) => {
            error!("WebSocket handshake failed for {addr}: {e}");
            return;
        }
    };

    let peer_id = addr.to_string();
    peer_manager.add_peer(&peer_id);
    info!("Peer connected: {peer_id}");

    let (mut ws_sender, mut ws_receiver) = ws_stream.split();

    loop {
        tokio::select! {
            // Incoming message from this peer
            msg = ws_receiver.next() => {
                match msg {
                    Some(Ok(tungstenite::Message::Text(text))) => {
                        match Message::from_json(&text) {
                            Ok(rawkit_msg) => {
                                // Process through peer manager (applies HAM, updates graph)
                                if let Some(response) = peer_manager.handle_message(&rawkit_msg) {
                                    let json = response.to_json();
                                    if ws_sender.send(tungstenite::Message::Text(json.clone().into())).await.is_err() {
                                        break;
                                    }
                                }

                                // Broadcast PUT messages to other peers
                                if matches!(rawkit_msg.kind, MessageKind::Put { .. }) {
                                    let _ = broadcast_tx.send((peer_id.clone(), text));
                                }
                            }
                            Err(e) => {
                                warn!("Invalid message from {peer_id}: {e}");
                            }
                        }
                    }
                    Some(Ok(tungstenite::Message::Close(_))) | None => break,
                    Some(Err(e)) => {
                        warn!("WebSocket error from {peer_id}: {e}");
                        break;
                    }
                    _ => {} // ping/pong/binary handled by tungstenite
                }
            }

            // Broadcast from another peer — forward to this one
            Ok((sender_id, json)) = broadcast_rx.recv() => {
                if sender_id != peer_id {
                    if ws_sender.send(tungstenite::Message::Text(json.into())).await.is_err() {
                        break;
                    }
                }
            }
        }
    }

    peer_manager.remove_peer(&peer_id);
    info!("Peer disconnected: {peer_id}");
}

// ─── Client Sync ──────────────────────────────────────────────────────────────

async fn run_client_sync(url: &str, db_path: &str) {
    let graph = Graph::sqlite(db_path).expect("Failed to open database");
    let peer_manager = Arc::new(PeerManager::new(graph.clone()));

    println!("Connecting to {url}...");

    let (ws_stream, _) = tokio_tungstenite::connect_async(url)
        .await
        .expect("Failed to connect to relay");

    println!("Connected. Syncing...");

    let (mut ws_sender, mut ws_receiver) = ws_stream.split();

    // Send all local data to the relay
    let all_souls = graph.list("");
    let mut sent = 0;
    for soul in &all_souls {
        if let Some(node) = graph.get_node(soul) {
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
            if !updates.is_empty() {
                let msg = Message::new_put(soul.clone(), updates);
                ws_sender
                    .send(tungstenite::Message::Text(msg.to_json().into()))
                    .await
                    .ok();
                sent += 1;
            }
        }
    }
    println!("Pushed {sent} nodes to relay.");

    // Listen for incoming updates
    println!("Listening for updates (Ctrl+C to stop)...");
    loop {
        tokio::select! {
            msg = ws_receiver.next() => {
                match msg {
                    Some(Ok(tungstenite::Message::Text(text))) => {
                        if let Ok(rawkit_msg) = Message::from_json(&text) {
                            if let Some(response) = peer_manager.handle_message(&rawkit_msg) {
                                ws_sender
                                    .send(tungstenite::Message::Text(response.to_json().into()))
                                    .await
                                    .ok();
                            }
                            if let MessageKind::Put { soul, updates, .. } = &rawkit_msg.kind {
                                println!("  Received: {soul} ({} props)", updates.len());
                            }
                        }
                    }
                    Some(Ok(tungstenite::Message::Close(_))) | None => {
                        println!("Relay disconnected.");
                        break;
                    }
                    _ => {}
                }
            }
            _ = tokio::signal::ctrl_c() => {
                println!("\nDisconnecting.");
                break;
            }
        }
    }
}

// ─── Benchmarks ───────────────────────────────────────────────────────────────

fn run_benchmarks(ops: usize) {
    println!();
    println!("  ╔══════════════════════════════════════════════════╗");
    println!("  ║          RAWKIT PERFORMANCE BENCHMARKS           ║");
    println!("  ╚══════════════════════════════════════════════════╝");
    println!();

    let graph = Graph::in_memory();
    let start = std::time::Instant::now();
    for i in 0..ops {
        graph.put_with_state(
            &format!("bench/{i}"),
            "value",
            Value::text("benchmark data payload"),
            i as f64,
        );
    }
    let put_elapsed = start.elapsed();
    let put_ops_sec = ops as f64 / put_elapsed.as_secs_f64();
    println!(
        "  Graph PUT     {:>10} ops  {:>8.1}ms  {:>12.0} ops/sec",
        ops,
        put_elapsed.as_secs_f64() * 1000.0,
        put_ops_sec
    );

    let start = std::time::Instant::now();
    for i in 0..ops {
        let _ = graph.get(&format!("bench/{i}"), "value");
    }
    let get_elapsed = start.elapsed();
    let get_ops_sec = ops as f64 / get_elapsed.as_secs_f64();
    println!(
        "  Graph GET     {:>10} ops  {:>8.1}ms  {:>12.0} ops/sec",
        ops,
        get_elapsed.as_secs_f64() * 1000.0,
        get_ops_sec
    );

    let start = std::time::Instant::now();
    for i in 0..ops {
        graph.put_with_state("conflict/node", "val", Value::number(i as f64), i as f64);
    }
    let ham_elapsed = start.elapsed();
    let ham_ops_sec = ops as f64 / ham_elapsed.as_secs_f64();
    println!(
        "  HAM Resolve   {:>10} ops  {:>8.1}ms  {:>12.0} ops/sec",
        ops,
        ham_elapsed.as_secs_f64() * 1000.0,
        ham_ops_sec
    );

    let vec_ops = ops.min(50_000);
    let index = VectorIndex::new(384);
    let start = std::time::Instant::now();
    for i in 0..vec_ops {
        let mut embedding = vec![0.0f32; 384];
        embedding[i % 384] = 1.0;
        index.upsert(&format!("vec/{i}"), embedding).ok();
    }
    let vec_insert_elapsed = start.elapsed();
    let vec_insert_ops_sec = vec_ops as f64 / vec_insert_elapsed.as_secs_f64();
    println!(
        "  Vec Insert    {:>10} ops  {:>8.1}ms  {:>12.0} ops/sec",
        vec_ops,
        vec_insert_elapsed.as_secs_f64() * 1000.0,
        vec_insert_ops_sec
    );

    let search_ops = 1000;
    let query = vec![1.0f32; 384];
    let start = std::time::Instant::now();
    for _ in 0..search_ops {
        index.search(&query, 10).ok();
    }
    let search_elapsed = start.elapsed();
    let search_ops_sec = search_ops as f64 / search_elapsed.as_secs_f64();
    println!(
        "  Vec Search    {:>10} ops  {:>8.1}ms  {:>12.0} ops/sec",
        search_ops,
        search_elapsed.as_secs_f64() * 1000.0,
        search_ops_sec
    );

    let crypto_ops = ops.min(100_000);
    let signing_key = ed25519_dalek::SigningKey::generate(&mut rand::thread_rng());
    let start = std::time::Instant::now();
    for _ in 0..crypto_ops {
        rawkit_crypto::sign(b"benchmark data for signing", &signing_key);
    }
    let sign_elapsed = start.elapsed();
    let sign_ops_sec = crypto_ops as f64 / sign_elapsed.as_secs_f64();
    println!(
        "  Ed25519 Sign  {:>10} ops  {:>8.1}ms  {:>12.0} ops/sec",
        crypto_ops,
        sign_elapsed.as_secs_f64() * 1000.0,
        sign_ops_sec
    );

    let sig = rawkit_crypto::sign(b"benchmark data for signing", &signing_key);
    let verifying_key = signing_key.verifying_key();
    let start = std::time::Instant::now();
    for _ in 0..crypto_ops {
        rawkit_crypto::verify(b"benchmark data for signing", &sig, &verifying_key).ok();
    }
    let verify_elapsed = start.elapsed();
    let verify_ops_sec = crypto_ops as f64 / verify_elapsed.as_secs_f64();
    println!(
        "  Ed25519 Vrfy  {:>10} ops  {:>8.1}ms  {:>12.0} ops/sec",
        crypto_ops,
        verify_elapsed.as_secs_f64() * 1000.0,
        verify_ops_sec
    );

    println!();
    println!("  ──────────────────────────────────────────────────");
    println!("  Rawkit v0.1.0 | Rust | SQLite + Vectors + Ed25519");
    println!();
}
