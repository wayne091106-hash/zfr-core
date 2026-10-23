# ZFR Core — OpenCode Zen Free Model Router

**ZFR Core** (`zfr-core`) 是一套以 Rust 打造的零依賴、高效能底層路由核心、CLI 與 OpenAI 相容本地閘道，免登入即可直連 **OpenCode Zen** 與 **Kilo 免費模型池**，並內建**多出口 IP 自動輪換**（支援 Cloudflare WARP `1.1.1.1` 的 IPv6/IPv4 雙棧自動切換、SOCKS5/HTTP 代理池與底層網卡 IP 綁定）。

**ZFR Core** (`zfr-core`) is a high-performance, zero-dependency Rust engine, CLI, and OpenAI-compatible local gateway that unlocks direct, zero-account access to **OpenCode Zen** and **Kilo Free Pool** models with built-in **multi-egress IP rotation** (including automatic IPv6/IPv4 dual-stack switching for Cloudflare WARP `1.1.1.1`, SOCKS5/HTTP proxy pools, and network interface binding).

---

## Why ZFR Core?

In mid-October 2026, OpenCode's `inference-next` free-tier gate began rejecting third-party gateways (such as `zen-gate <= v1.7.19`) with `403 FreeTierError` or punitive `429 FreeUsageLimitError` responses, leading `zen-gate v1.8.0` to delete its entire OpenCode Zen lane.

Reverse-engineering the official OpenCode v2.1.3 CLI revealed that upstream validation is **not** a permanent closure of the free tier, but a deterministic wire-level fingerprint check:

1. **48-bit Inverted Millisecond Timestamp in `x-opencode-session`**:
   - Format: `ses_<12-hex><14-base62>`, where the 12-hex prefix encodes `~((now_ms << 12) | seq) & 0xffff_ffff_ffff`.
2. **Mandatory `stream: true` on Upstream**:
   - Upstream immediately rejects `stream: false` with `403 FreeTierError`. ZFR Core always streams SSE upstream and transparently aggregates SSE into standard JSON when a downstream client requests `stream: false`.
3. **Byte-Exact `read` & `shell` Tool Fingerprinting (without HTML Escaping)**:
   - Upstream requires OpenCode's verbatim `read` and `shell` tool schemas with `"strict": false`, `"max_completion_tokens": 32000`, and top-level JSON key ordering (`model`, `messages`, `tools`, `stream`, `stream_options`, `max_completion_tokens`).
   - Crucially, Go's `encoding/json` escapes `<line>: <content>` inside the `read` tool description to `\u003cline\u003e: \u003ccontent\u003e`, triggering upstream detection. Rust's `serde_json` emits raw unescaped UTF-8 bytes matching JavaScript's `JSON.stringify`.
   - When a client defines its own `read` or `shell` tools, ZFR Core transparently aliases them to `read_client` / `shell_client` upstream and unaliases tool calls on the way back.
4. **Multi-Egress IP Rotation & Session-Per-Exit Affinity**:
   - Free quota is tracked per `(egress_ip, model)`.
   - By default (`--egress auto`), ZFR Core binds separate IPv6 (`[::]`) and IPv4 (`0.0.0.0`) sockets—giving Cloudflare WARP (`1.1.1.1`) and dual-stack users two independent egress IPs out of the box—and supports arbitrary HTTP/SOCKS5 proxy pools and `bind://<ip>` interface binding.

---

## Supported Free Models

### Lane 1: OpenCode Zen (`opencode/*`)
| Model ID | Context | Max Output | Notes |
| :--- | :---: | :---: | :--- |
| `deepseek-v4.1-flash-free` | 262K | 64K | DeepSeek V4.1 Flash reasoning & coding |
| `glm-5.3-flash-free` | 204K | 128K | GLM-5.3 Flash agentic & coding |
| `mimo-v2.6-flash-free` | 262K | 64K | Xiaomi MiMo V2.6 Flash |
| `beam-free` | 1M | 128K | 1M-context high-speed reasoning |
| `cat-jam-free` | 1M | 128K | 1M-context reasoning |
| `nemotron-3-ultra-free` | 1M | 128K | NVIDIA Nemotron 3 Ultra (550B A55B) |
| `nemotron-3.5-super-vl-free` | 262K | 64K | Vision-Language reasoning |
| `nemotron-3.5-lightning-free` | 262K | 262K | Ultra-low-latency reasoning |
| `flint-40b-free` | 262K | 64K | Recursive Flint 40B |
| `big-pickle` | 200K | 32K | General reasoning |

### Lane 2: Kilo Free Pool (`kilo/*`)
- `kilo-auto/free`, `reflection/beam-free`, `nvidia/nemotron-3.5-super-vl:free`, `nvidia/nemotron-3-ultra-550b-a55b:free`, `nvidia/nemotron-3.5-lightning:free`, `cohere/north-mini-code:free`, `poolside/laguna-s-2.1:free`, `dots-studio/dots-3-note-preview:free`, `recursive/flint-v1.0:free`, `stealth/glyph-cluster`, `openrouter/free`.

---

## Quick Start

### 1. Build from Source (Rust 1.80+)
```bash
git clone https://github.com/wayne091106-hash/zfr-core.git
cd zfr-core
cargo build --release
```

### 2. Probe Live Model Status
```bash
cargo run --release -- probe
```

### 3. Direct CLI Inference
```bash
cargo run --release -- run -m deepseek-v4.1-flash-free "Explain Rust ownership in one sentence."
```

### 4. Start the OpenAI-Compatible Gateway Server
```bash
# Default: listens on http://127.0.0.1:18788/v1 with IPv6 + IPv4 dual-stack egress rotation
cargo run --release -- serve --bind 127.0.0.1:18788

# With custom SOCKS5 / HTTP rotating proxies or local interface binding:
cargo run --release -- --egress ipv6,ipv4,socks5://127.0.0.1:1080,http://127.0.0.1:7890#pool serve
```

### 5. Call via Standard OpenAI SDK / `curl` (Both `stream: true` and `stream: false` Work!)
```bash
curl http://127.0.0.1:18788/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "deepseek-v4.1-flash-free",
    "stream": false,
    "messages": [{"role": "user", "content": "Hello from ZFR Core!"}]
  }'
```

---

## License

MIT
