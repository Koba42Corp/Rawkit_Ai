# Rawkit_AI — Browser Playground Demo

Two independent Rawkit WASM instances syncing through the relay in real time.

## Run it

**Step 1 — Start the relay:**
```bash
cargo run --release -p rawkit-server -- serve --port 8765
```

**Step 2 — Serve the demo:**
```bash
python3 -m http.server 3000 --directory demo
```

**Step 3 — Open in browser:**
```
http://localhost:3000/
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
