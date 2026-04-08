# Rawkit_AI — Browser Playground Demo

Two independent Rawkit WASM instances syncing through the relay in real time.

## Run it

**Option A — Use the public relay (zero setup):**
```bash
python3 -m http.server 3000 --directory demo
# Open http://localhost:3000/
```
Connects to `wss://rawkit.koba42.com` by default.

**Option B — Use a local relay:**
```bash
cargo run --release -p rawkit-server -- serve --port 8765
python3 -m http.server 3000 --directory demo
# Open http://localhost:3000/?relay=ws://localhost:8765
```

> Must be served over HTTP — WASM won't load from `file://`.

## What it demonstrates

- **Panel A (left):** Write graph data and embed vectors
- **Panel B (right):** Receives writes via relay, runs semantic vector search
- Writes travel: Panel A → WebSocket → Relay → WebSocket → Panel B
- Each panel has its own WASM Rawkit instance with in-memory storage
- HAM CRDT conflict resolution runs in the browser via WASM

## Rebuild WASM (after changing Rust source)

```bash
wasm-pack build bindings/rawkit-wasm --target web --out-dir ../../demo/pkg --release
```
