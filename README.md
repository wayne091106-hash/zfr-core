> **Module Status**: v0.2.0 — Stable internal transport crate (28 unit tests passing). Adds background daemon/Windows-service operation, a quota-aware background health prober, API-key auth, JSON config file support, and exponential backoff for broken routes on top of the standard OpenAI-compatible protocol normalization and dual-stack IPv6/IPv4 connection pooling.

# ZFR Core — High-Performance Protocol Transport & Model Router

**ZFR Core** (`zfr-core`) 是一套以 Rust 打造的高效能底層傳輸核心、CLI 與 OpenAI 相容本地代理閘道，提供 **OpenCode v2.1.3 與 Kilo 協定規格相容性**，並具備**多線路高可用路由與雙棧連接池**（支援 IPv6/IPv4 雙棧自動切換、SOCKS5/HTTP 傳輸代理池與本地網卡介面綁定）。

**ZFR Core** (`zfr-core`) is a high-performance Rust transport engine, CLI, and OpenAI-compatible local gateway providing standard wire protocol compatibility for OpenCode v2.1.3 and Kilo endpoints, featuring multi-homed high-availability routing and dual-stack connection pooling (including IPv6/IPv4 dual-stack switching, SOCKS5/HTTP proxy support, and local interface binding).

---

## Protocol Compatibility & System Architecture

The OpenCode v2.1.3 client wire protocol and its upstream `inference-next` gateway enforce strict protocol framing, session header formats, and payload structures. Standard HTTP clients without protocol framing alignment typically fail handshake validation (`403` or `429` errors). ZFR Core provides an exact wire protocol compatibility layer adhering strictly to OpenCode v2.1.3 client specifications:

1. **OpenCode v2.1.3 Wire Protocol Compatibility & Session Header Formatter (`x-opencode-session`)**:
   - Format: `ses_<12-hex><14-base62>`, where the 12-hex prefix encodes the 48-bit timestamp bitmask: `~((now_ms << 12) | seq) & 0xffff_ffff_ffff` for monotonic session sequencing.
2. **Upstream SSE Stream Normalization**:
   - Upstream endpoints require SSE streaming framing (`stream: true`). ZFR Core manages upstream SSE connections transparently and aggregates responses into standard JSON completion structures when downstream clients request `stream: false`.
3. **Tool Schema Aliasing & Wire Compatibility**:
   - Upstream protocol framing requires verbatim OpenCode `read` and `shell` tool schema definitions with `"strict": false`, `"max_completion_tokens": 32000`, and top-level JSON key serialization order (`model`, `messages`, `tools`, `stream`, `stream_options`, `max_completion_tokens`).
   - Rust's `serde_json` emits standard UTF-8 bytes matching browser and JavaScript `JSON.stringify` serialization.
   - When downstream clients supply custom `read` or `shell` tools, ZFR Core safely aliases them to `read_client` / `shell_client` on upstream egress and unaliases tool calls on ingress.
4. **Multi-Homed IPv6/IPv4 Dual-Stack Transport & Endpoint Health Pool**:
   - Concurrency budgets and rate limits are partitioned per `(egress_ip, model)`.
   - In default configuration (`--egress auto`), ZFR Core provisions independent IPv6 (`[::]`) and IPv4 (`0.0.0.0`) sockets—giving Cloudflare WARP (`1.1.1.1`) and dual-stack environments two independent egress network paths—alongside arbitrary HTTP/SOCKS5 proxy pools and local interface IP bindings (`bind://<ip>`).

---

## Supported Model Endpoints

### Lane 1: OpenCode Lane (`opencode/*`)
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

### Lane 2: Kilo Lane (`kilo/*`)
- `kilo-auto/free`, `reflection/beam-free`, `nvidia/nemotron-3.5-super-vl:free`, `nvidia/nemotron-3-ultra-550b-a55b:free`, `nvidia/nemotron-3.5-lightning:free`, `cohere/north-mini-code:free`, `poolside/laguna-s-2.1:free`, `dots-studio/dots-3-note-preview:free`, `recursive/flint-v1.0:free`, `stealth/glyph-cluster`, `openrouter/free`.

---

## Installation

### One-liner (Windows, PowerShell)

```powershell
# Download + install the latest release to %LOCALAPPDATA%\Programs\zfr and add to PATH
irm https://raw.githubusercontent.com/wayne091106-hash/zfr-core/main/install.ps1 | iex

# Same, but also register + start the Windows service (run PowerShell as Administrator)
& ([scriptblock]::Create((irm https://raw.githubusercontent.com/wayne091106-hash/zfr-core/main/install.ps1))) -Service
```

### Manual download

Grab `zfr-x86_64-pc-windows-msvc.zip` (or the Linux `.tar.gz`) from
[GitHub Releases](https://github.com/wayne091106-hash/zfr-core/releases/latest),
extract `zfr.exe` anywhere on your `PATH`, done — no Rust toolchain required.

### Build from Source (Rust 1.80+)
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

# With custom SOCKS5 / HTTP proxy pool or local interface binding:
cargo run --release -- --egress ipv6,ipv4,socks5://127.0.0.1:1080,http://127.0.0.1:7890#pool serve
```

### 5. Call via Standard OpenAI SDK / `curl` (Both `stream: true` and `stream: false` Supported)
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

## Background Operation (常駐執行)

### Daemon mode (no Administrator required)

```bash
# Start detached in the background — logs go to %LOCALAPPDATA%\zfr\zfr.log
zfr serve --bind 127.0.0.1:18788 --daemon

# Stop it again
zfr stop
```

`--daemon` re-launches `zfr serve` as a detached process (no console window), writes the
PID to `%LOCALAPPDATA%\zfr\zfr.pid` and redirects logs to `%LOCALAPPDATA%\zfr\zfr.log`.
All other flags (`--bind`, `--egress`, `--api-key`, `--probe-interval`, `--config`) are
forwarded to the background instance. The detached child never inherits the caller's
stdio handles, so it is safe to use inside shell pipelines and scripts.

### Windows Service (auto-start at boot, restart on failure)

Requires an **Administrator** shell:

```bash
# Install (persists current --bind / --egress / --api-key / --probe-interval into
# %PROGRAMDATA%\zfr\config.json and registers an auto-start service)
zfr service install --bind 127.0.0.1:18788

zfr service start      # start now
zfr service stop       # graceful stop
zfr service status     # running / stopped / ...
zfr service uninstall  # remove
```

The service runs as LocalSystem via the Service Control Manager, starts automatically
at boot, and is configured to restart 10 seconds after an unexpected exit. Service logs
are written to `%LOCALAPPDATA%\zfr\zfr-service.log`.

### Background health prober (quota-aware)

While `serve` is running, a background tokio task sweeps every catalog model across
all egress paths every `--probe-interval` seconds (default **300**, `0` disables):

- Models with at least one healthy exit are probed normally; results seed the
  `(exit, model)` cooldown state, so background-observed failures immediately
  divert live traffic to healthy routes.
- Models whose **every** exit is on cooldown (e.g. daily free-tier quota exhausted,
  `Retry-After` ~11h) are **skipped entirely** — no upstream requests are spent
  renewing their cooldown windows. They show up in `/v1/status` as
  `skipped: all exits on cooldown (quota429, 40763s left)`.
- Persistently broken routes back off **exponentially** (30s → 1m → 2m → … capped
  at 1h), while `Quota429` windows always follow the server's `Retry-After`.

The latest snapshot is exposed at `GET /v1/status`:

```json
{
  "service": "zfr-core", "version": "0.2.0", "uptime_secs": 42,
  "exits": 2, "active_cooldowns": [],
  "prober": { "enabled": true, "interval_secs": 300, "models_ok": 21, "models_total": 23, "reports": [...] }
}
```

> Note: the prober consumes free-tier quota (one small request per model per cycle).
> Disable with `--probe-interval 0` if you want the gateway to be fully passive.

---

## Gateway Endpoints

| Endpoint | Description |
| :--- | :--- |
| `GET /health`, `GET /v1/health` | Liveness + exit count + active cooldowns (never requires auth) |
| `GET /v1/models` | OpenAI-compatible model list |
| `POST /v1/chat/completions` | OpenAI-compatible chat completions (SSE or JSON) |
| `GET /v1/egress` | Configured exits + active cooldown details |
| `GET /v1/probe` | On-demand probe of all models |
| `GET /v1/status` | Uptime, prober snapshot, cooldowns |

## API Key Authentication

```bash
zfr serve --api-key my-secret
```

With an API key set, every endpoint except `/health` requires
`Authorization: Bearer my-secret` and returns `401 invalid_api_key` otherwise.
The key can also come from the `ZFR_API_KEY` environment variable or the config file.

## Configuration File

Optional JSON config; search order (first match wins):

1. `--config <path>` (error if missing)
2. `$ZFR_CONFIG`
3. `./zfr.json`
4. `%PROGRAMDATA%\zfr\config.json` (machine-wide; used by the Windows service)
5. `%LOCALAPPDATA%\zfr\config.json`

```json
{
  "bind": "127.0.0.1:18788",
  "egress": ["auto"],
  "no_failover": false,
  "api_key": null,
  "probe_interval_secs": 300
}
```

Precedence: **CLI flags > environment variables > config file > defaults**.
Supported environment variables: `ZFR_EGRESS`, `ZFR_API_KEY`, `ZFR_BIND`,
`ZFR_PROBE_INTERVAL`, `ZFR_CONFIG`, `RUST_LOG` (tracing filter, default `info`).

---

## Testing

```bash
cargo test   # 28 unit tests: config, catalog, egress cooldown pool (incl. backoff & availability), wire fingerprinting, SSE rewriting, HTTP auth middleware
```

---

## License

MIT
