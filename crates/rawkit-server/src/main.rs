use clap::{Parser, Subcommand};
use rawkit_core::{Graph, Value};
use rawkit_vectors::VectorIndex;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "rawkit")]
#[command(about = "Rawkit - Decentralized vector-graph memory for AI agents")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start a Rawkit relay server.
    Serve {
        /// Port to listen on.
        #[arg(short, long, default_value_t = 8765)]
        port: u16,
        /// Path to SQLite database file.
        #[arg(short, long, default_value = "rawkit.db")]
        db: String,
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
        /// Property key.
        key: String,
    },
    /// List nodes by prefix.
    #[command(name = "ls")]
    List {
        /// Path prefix to list.
        prefix: String,
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
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Serve { port, db } => {
            println!("  Rawkit Relay Server");
            println!("  Database: {db}");
            println!("  Listening on: ws://0.0.0.0:{port}");
            println!();

            let _graph = Graph::sqlite(&db).expect("Failed to open database");
            let _vectors = VectorIndex::new(384); // all-MiniLM default

            // TODO: WebSocket server loop (rawkit-sync transport)
            println!("  WebSocket server coming in next iteration.");
            println!("  Graph and vector index initialized successfully.");

            // Keep alive
            tokio::signal::ctrl_c()
                .await
                .expect("Failed to listen for ctrl-c");
            println!("\n  Shutting down.");
        }

        Commands::Put { soul, key, value } => {
            let graph = Graph::in_memory();
            let parsed: Value = serde_json::from_str(&value).unwrap_or(Value::text(&value));
            graph.put(&soul, &key, parsed.clone());
            println!("  PUT {soul}.{key} = {}", serde_json::to_string(&parsed).unwrap());
        }

        Commands::Get { soul, key } => {
            let graph = Graph::in_memory();
            match graph.get(&soul, &key) {
                Some(val) => println!("  {soul}.{key} = {}", serde_json::to_string(&val).unwrap()),
                None => println!("  (not found)"),
            }
        }

        Commands::List { prefix } => {
            let graph = Graph::in_memory();
            let souls = graph.list(&prefix);
            if souls.is_empty() {
                println!("  No nodes found with prefix: {prefix}");
            } else {
                for soul in &souls {
                    println!("  {soul}");
                }
            }
        }

        Commands::Bench { ops } => {
            run_benchmarks(ops);
        }
    }
}

fn run_benchmarks(ops: usize) {
    println!();
    println!("  ╔══════════════════════════════════════════════════╗");
    println!("  ║          RAWKIT PERFORMANCE BENCHMARKS           ║");
    println!("  ╚══════════════════════════════════════════════════╝");
    println!();

    // --- Graph PUT benchmark ---
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

    // --- Graph GET benchmark ---
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

    // --- HAM conflict resolution benchmark ---
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

    // --- Vector index benchmark (smaller scale) ---
    let vec_ops = ops.min(10_000); // HNSW rebuild is expensive
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

    // --- Vector search benchmark ---
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

    // --- Crypto benchmark ---
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
    println!("  Rawkit v0.1.0 | Rust | SQLite + HNSW + Ed25519");
    println!();
}
