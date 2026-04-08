/**
 * Rawkit_AI Browser Playground
 *
 * Two independent Rawkit WASM instances (Panel A and Panel B), each with
 * their own WebSocket connection to the relay. Writes on Panel A travel
 * through the relay and appear on Panel B — and vice versa.
 *
 * Wire protocol (matches rawkit-sync/src/message.rs):
 *   { "#": "<uuid>", "type": "put", "soul": "<soul>",
 *     "updates": { "<key>": { "v": <value>, "s": <state_f64> } } }
 *   { "#": "<uuid>", "type": "ack", "ok": "<msg-id>" }
 */

import init, { Rawkit } from './pkg/rawkit_wasm.js';

// Default to the public relay. Override with ?relay=ws://localhost:8765
const params = new URLSearchParams(window.location.search);
const RELAY_URL = params.get('relay') || 'wss://rawkit.koba42.com';

// ── Tiny UUID v4 (no crypto needed — just for dedup) ─────────────────────────
function uuid() {
  return 'xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx'.replace(/[xy]/g, c => {
    const r = (Math.random() * 16) | 0;
    return (c === 'x' ? r : (r & 0x3) | 0x8).toString(16);
  });
}

function now() { return Date.now(); }

function formatTime(ts) {
  const d = new Date(ts);
  return d.toLocaleTimeString('en-US', { hour12: false, hour: '2-digit', minute: '2-digit', second: '2-digit' })
    + '.' + String(d.getMilliseconds()).padStart(3, '0');
}

// ── Log helpers ───────────────────────────────────────────────────────────────
function appendLog(logEl, entry) {
  // Remove "waiting" placeholder on first real entry
  const placeholder = logEl.querySelector('.log-system');
  if (placeholder) placeholder.closest('.log-entry')?.remove();

  logEl.insertAdjacentHTML('afterbegin', entry);

  // Cap at 200 entries
  const entries = logEl.querySelectorAll('.log-entry');
  if (entries.length > 200) entries[entries.length - 1].remove();
}

function logWrite(logEl, soul, key, value, isLocal) {
  const cls = isLocal ? 'local' : 'incoming';
  const arrow = isLocal ? '→' : '←';
  const valueStr = JSON.stringify(value);
  appendLog(logEl, `
    <div class="log-entry ${cls}">
      <span class="log-time">${formatTime(now())}</span>
      <span>${arrow}</span>
      <span class="log-soul">${escHtml(soul)}</span>
      <span class="log-key">.${escHtml(key)}</span>
      <span style="color:#555">=</span>
      <span class="log-value">${escHtml(valueStr)}</span>
    </div>`);
}

function logSystem(logEl, msg) {
  appendLog(logEl, `
    <div class="log-entry">
      <span class="log-time">${formatTime(now())}</span>
      <span class="log-system">${escHtml(msg)}</span>
    </div>`);
}

function logVector(logEl, soul, dims) {
  appendLog(logEl, `
    <div class="log-entry vector">
      <span class="log-time">${formatTime(now())}</span>
      <span>⊕</span>
      <span class="log-soul" style="color:#818cf8">${escHtml(soul)}</span>
      <span class="log-meta">vector embedded (${dims} dims, local hash)</span>
    </div>`);
}

function escHtml(s) {
  return String(s)
    .replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');
}

// ── RawkitPanel ──────────────────────────────────────────────────────────────
/**
 * One panel = one Rawkit WASM instance + one WebSocket to the relay.
 * Panels are completely independent — the relay is the only shared state.
 */
class RawkitPanel {
  constructor({ name, relayUrl, logEl, badgeEl, onConnected, onDisconnected }) {
    this.name = name;
    this.relayUrl = relayUrl;
    this.logEl = logEl;
    this.badgeEl = badgeEl;
    this.onConnected = onConnected;
    this.onDisconnected = onDisconnected;

    this.rawkit = null;     // Rawkit WASM instance
    this.ws = null;
    this.connected = false;
    this.sentIds = new Set(); // for echo dedup
    this.reconnectTimer = null;
    this.reconnectDelay = 1000;
  }

  init(rawkitInstance) {
    this.rawkit = rawkitInstance;
    logSystem(this.logEl, `WASM ready — connecting to relay…`);
    this._connect();
  }

  _connect() {
    if (this.ws) { this.ws.onclose = null; this.ws.close(); }

    this.ws = new WebSocket(this.relayUrl);

    this.ws.onopen = () => {
      this.connected = true;
      this.reconnectDelay = 1000;
      this.badgeEl.textContent = 'connected';
      this.badgeEl.classList.add('connected');
      logSystem(this.logEl, `WebSocket connected to ${this.relayUrl}`);
      this.onConnected?.();
    };

    this.ws.onmessage = (event) => {
      this._handleMessage(event.data);
    };

    this.ws.onerror = () => {
      logSystem(this.logEl, `WebSocket error — is the relay running? (rawkit serve --port 8765)`);
    };

    this.ws.onclose = () => {
      this.connected = false;
      this.badgeEl.textContent = 'disconnected';
      this.badgeEl.classList.remove('connected');
      this.onDisconnected?.();
      logSystem(this.logEl, `Disconnected — retrying in ${this.reconnectDelay / 1000}s…`);
      this.reconnectTimer = setTimeout(() => {
        this.reconnectDelay = Math.min(this.reconnectDelay * 2, 10000);
        this._connect();
      }, this.reconnectDelay);
    };
  }

  _handleMessage(data) {
    let msg;
    try { msg = JSON.parse(data); } catch { return; }

    const id = msg['#'];
    const type = msg.type;

    // Skip echoes of our own messages
    if (this.sentIds.has(id)) {
      this.sentIds.delete(id); // clean up
      return;
    }

    if (type === 'put') {
      const soul = msg.soul;
      const updates = msg.updates || {};

      for (const [key, entry] of Object.entries(updates)) {
        const value = entry.v;  // already a JS primitive (null/bool/number/string)
        const state = entry.s;  // f64 timestamp

        // Apply to our local WASM graph with the sender's state for correct HAM
        try {
          this.rawkit.put_with_state(soul, key, value, state);
        } catch (e) {
          console.warn(`[${this.name}] put_with_state failed:`, e);
        }

        logWrite(this.logEl, soul, key, value, false);
      }

      // Send ACK back
      const ack = JSON.stringify({ '#': uuid(), type: 'ack', ok: id });
      if (this.ws?.readyState === WebSocket.OPEN) this.ws.send(ack);
    }
    // ack/sub/unsub — ignore in the demo
  }

  /**
   * Write a value locally and broadcast to relay.
   */
  put(soul, key, value) {
    if (!this.rawkit) return;

    const state = now();
    // Write locally with our own clock
    this.rawkit.put(soul, key, value);
    logWrite(this.logEl, soul, key, value, true);

    // Broadcast to relay
    this._sendPut(soul, key, value, state);
  }

  _sendPut(soul, key, value, state) {
    if (!this.ws || this.ws.readyState !== WebSocket.OPEN) return;

    const id = uuid();
    const msg = {
      '#': id,
      type: 'put',
      soul,
      updates: {
        [key]: { v: value, s: state }
      }
    };

    this.sentIds.add(id);
    this.ws.send(JSON.stringify(msg));
  }

  /**
   * Embed text → local hash vector, upsert into WASM vector index.
   * Returns the embedding for re-use.
   */
  embedAndUpsert(soul, text) {
    if (!this.rawkit) return null;

    // Generate a deterministic local-hash embedding in JS
    // (matches the Rust LocalHashEmbedding algorithm — n-gram hashing)
    const dims = 64; // compact for the demo
    const vec = new Float32Array(dims);
    const lower = text.toLowerCase();

    // Unigrams
    for (let i = 0; i < lower.length; i++) {
      const c = lower.charCodeAt(i);
      const h = djb2([c]) % dims;
      vec[h] += 1.0;
      vec[(h + i) % dims] += 0.3;
    }

    // Bigrams
    for (let i = 0; i < lower.length - 1; i++) {
      const h = djb2([lower.charCodeAt(i), lower.charCodeAt(i + 1)]) % dims;
      vec[h] += 1.5;
    }

    // Word-level
    for (const word of lower.split(/\s+/)) {
      if (!word) continue;
      const h = djb2Str(word) % dims;
      vec[h] += 3.0;
      vec[(h + word.length) % dims] += 0.5;
    }

    // L2 normalize
    let norm = 0;
    for (let i = 0; i < dims; i++) norm += vec[i] * vec[i];
    norm = Math.sqrt(norm);
    if (norm > 0) for (let i = 0; i < dims; i++) vec[i] /= norm;

    try {
      this.rawkit.upsert_vector(soul, vec);
    } catch (e) {
      console.warn(`[${this.name}] upsert_vector failed:`, e);
      return null;
    }

    logVector(this.logEl, soul, dims);
    return vec;
  }

  /**
   * Semantic search. Returns array of {soul, score}.
   */
  search(queryText, topK = 5) {
    if (!this.rawkit) return [];

    const vec = this.embedAndUpsert('__query__', queryText);
    // Remove the temporary query soul (clean up)
    // (No remove in the WASM binding — just leave it, it won't affect correctness)

    try {
      const results = this.rawkit.search_vectors(vec, topK);
      return (results || []).filter(r => r[0] !== '__query__');
    } catch (e) {
      console.warn(`[${this.name}] search_vectors failed:`, e);
      return [];
    }
  }
}

// ── DJB2 hash helpers (mirrors Rust LocalHashEmbedding) ───────────────────────
function djb2(codes) {
  let h = 5381;
  for (const c of codes) h = (Math.imul(h, 33) + c) >>> 0;
  return h;
}
function djb2Str(s) {
  let h = 5381;
  for (let i = 0; i < s.length; i++) h = (Math.imul(h, 33) + s.charCodeAt(i)) >>> 0;
  return h;
}

// ── Relay status indicator ────────────────────────────────────────────────────
function updateRelayStatus(connectedCount) {
  const el = document.getElementById('relay-status');
  const text = document.getElementById('relay-status-text');
  el.className = 'relay-status';
  if (connectedCount === 2) {
    el.classList.add('connected');
    text.textContent = 'Relay connected (both panels)';
  } else if (connectedCount === 1) {
    el.classList.add('connected');
    text.textContent = 'Relay connected (1 panel)';
  } else {
    el.classList.add('error');
    text.textContent = 'Relay disconnected — run: rawkit serve --port 8765';
  }
}

// ── Main ──────────────────────────────────────────────────────────────────────
async function main() {
  // Update relay URL display
  document.getElementById('relay-url-display').textContent = RELAY_URL;
  document.getElementById('footer-relay').textContent = RELAY_URL;

  // Load WASM once — both panels share the module but get separate instances
  let wasmReady = false;
  try {
    await init();
    wasmReady = true;
  } catch (e) {
    console.error('WASM init failed:', e);
    document.getElementById('log-left').innerHTML =
      `<div class="log-entry"><span class="log-system" style="color:#ef4444">
        WASM failed to load. Serve this directory over HTTP (not file://).
        Try: python3 -m http.server 3000
      </span></div>`;
    return;
  }

  // Track connection counts for status indicator
  let connectedCount = 0;
  const onConn = () => { connectedCount++; updateRelayStatus(connectedCount); };
  const onDisc = () => { connectedCount = Math.max(0, connectedCount - 1); updateRelayStatus(connectedCount); };

  // ── Panel A (left — writer) ───────────────────────────────────────────────
  const panelA = new RawkitPanel({
    name: 'A',
    relayUrl: RELAY_URL,
    logEl: document.getElementById('log-left'),
    badgeEl: document.getElementById('badge-left'),
    onConnected: () => {
      onConn();
      document.getElementById('btn-write').disabled = false;
      document.getElementById('btn-embed').disabled = false;
    },
    onDisconnected: () => {
      onDisc();
      document.getElementById('btn-write').disabled = true;
      document.getElementById('btn-embed').disabled = true;
    },
  });

  // ── Panel B (right — receiver) ────────────────────────────────────────────
  const panelB = new RawkitPanel({
    name: 'B',
    relayUrl: RELAY_URL,
    logEl: document.getElementById('log-right'),
    badgeEl: document.getElementById('badge-right'),
    onConnected: () => {
      onConn();
      document.getElementById('btn-search').disabled = false;
    },
    onDisconnected: () => {
      onDisc();
      document.getElementById('btn-search').disabled = true;
    },
  });

  // Create separate Rawkit WASM instances for each panel
  panelA.init(new Rawkit(64));
  panelB.init(new Rawkit(64));

  updateRelayStatus(0);

  // ── Write button ──────────────────────────────────────────────────────────
  document.getElementById('btn-write').addEventListener('click', () => {
    const soul  = document.getElementById('inp-soul').value.trim();
    const key   = document.getElementById('inp-key').value.trim();
    const rawVal = document.getElementById('inp-value').value.trim();

    if (!soul || !key || !rawVal) return;

    // Auto-coerce: number if it looks numeric, otherwise string
    const value = rawVal === 'null' ? null
                : rawVal === 'true' ? true
                : rawVal === 'false' ? false
                : /^-?\d+(\.\d+)?$/.test(rawVal) ? parseFloat(rawVal)
                : rawVal;

    panelA.put(soul, key, value);
  });

  // Allow Enter key in inputs to trigger write
  ['inp-soul', 'inp-key', 'inp-value'].forEach(id => {
    document.getElementById(id).addEventListener('keydown', e => {
      if (e.key === 'Enter') document.getElementById('btn-write').click();
    });
  });

  // ── Embed button (Panel A) ────────────────────────────────────────────────
  document.getElementById('btn-embed').addEventListener('click', () => {
    const soul = document.getElementById('vec-soul').value.trim();
    const text = document.getElementById('vec-text').value.trim();
    if (!soul || !text) return;

    // Embed on both panels so Panel B can also search
    panelA.embedAndUpsert(soul, text);
    panelB.embedAndUpsert(soul, text);

    logSystem(document.getElementById('log-right'),
      `Vector embedded for "${soul}" (mirrored from Panel A)`);
  });

  // ── Search button (Panel B) ───────────────────────────────────────────────
  document.getElementById('btn-search').addEventListener('click', () => {
    const query = document.getElementById('search-query').value.trim();
    if (!query) return;

    const results = panelB.search(query, 5);
    const container = document.getElementById('search-results');

    if (!results || results.length === 0) {
      container.innerHTML = '<div class="search-result-item" style="color:#444">No results — embed some vectors first.</div>';
      return;
    }

    container.innerHTML = results.map(([soul, score]) => `
      <div class="search-result-item">
        <span class="search-result-soul">${escHtml(soul)}</span>
        <span class="search-result-score">${(score * 100).toFixed(1)}%</span>
      </div>
    `).join('');

    logSystem(document.getElementById('log-right'),
      `Search "${query}" → ${results.length} result(s)`);
  });

  document.getElementById('search-query').addEventListener('keydown', e => {
    if (e.key === 'Enter') document.getElementById('btn-search').click();
  });
}

main().catch(console.error);
