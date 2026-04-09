# ── Stage 1: Build ────────────────────────────────────────────────────────────
FROM rust:1.82-slim-bookworm AS builder

RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY Cargo.toml rust-toolchain.toml ./
COPY Cargo.lock* ./
COPY crates/ crates/

# Build only the server binary in release mode
RUN cargo build --release -p rawkit-server

# ── Stage 2: Runtime ──────────────────────────────────────────────────────────
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/rawkit /usr/local/bin/rawkit

# Persistent volume for the SQLite database
VOLUME /data
WORKDIR /data

EXPOSE 8765

ENTRYPOINT ["rawkit"]
CMD ["serve", "--port", "8765"]
