//! OpenCode v2.1.3 Wire Protocol Compatibility & Session Header Formatter.
//!
//! Implements protocol framing and session specifications matching OpenCode v2.1.3 client:
//! 1. Formatted 48-bit inverted timestamp in `x-opencode-session` (`ses_<12hex><14base62>`).
//! 2. Mandatory `stream: true` upstream framing (with local SSE-to-JSON aggregation for non-streaming downstream clients).
//! 3. Standard tool schema declarations (with schema aliasing when downstream defines custom tools).

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const OPENCODE_CLIENT_UA: &str = "opencode/latest/2.1.3/cli";
pub const DEFAULT_PROJECT_ID: &str = "global";

const BASE62: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
static SESSION_SEQ: AtomicU64 = AtomicU64::new(1);

const REAL_READ_DESC: &str = "Read the contents of a file or directory. Supports text files, images, and PDFs. Images and PDFs are presented directly to the model. Each text line is prefixed by its 1-based line number as <line>: <content>. The prefix is for reference and is not part of the file content. Directory entries are returned one per line. Use offset and limit to read large files or directories in sections. Prefer one larger read over many small slices, and use grep to find specific content in large files.";

const REAL_SHELL_DESC: &str = "Execute a shell command and return its output. Commands run on Linux using bash. Quote file paths containing spaces or special characters. Prefer dedicated tools over shell commands when possible. When output is large, the full result is saved to a file and a truncated preview is returned. Rely on automatic truncation unless filtering the output is more useful. Commands accept an optional timeout, background commands have no timeout by default. Background commands return immediately, and you will be notified when they complete.";

const NOTE_NO_TOOLS: &str = "Do not call the `read` or `shell` tools; they are placeholders that do nothing. Reply directly without invoking tools.";
const NOTE_PLACEHOLDER_TOOLS: &str = "Do not call the `read` or `shell` tools; they are placeholders that do nothing. Use the other tools you have been given instead.";
const NOTE_ALIASED_BOTH: &str = "The `read` and `shell` entries in the tool list are inert placeholders -- never call them. To read files or run commands, call `read_client` and `shell_client` instead.";
const NOTE_ALIASED_READ_ONLY: &str = "The `read` and `shell` entries in the tool list are inert placeholders -- never call them. To read files, call `read_client`; to run commands, use your other provided command tool (such as `bash`).";
const NOTE_ALIASED_SHELL_ONLY: &str = "The `read` and `shell` entries in the tool list are inert placeholders -- never call them. To run commands, call `shell_client`; to read files, use your other provided file tool.";

/// Formats an OpenCode v2.1.3 session identifier: `ses_<12-hex-inv-ts><14-base62>`.
///
/// The first 12 hex characters encode `~((ms << 12) | seq) & 0xffff_ffff_ffff`
/// verifying session freshness and monotonic sequencing.
pub fn mint_session_id(now_ms_override: Option<u64>) -> String {
    let d = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    let now_ms = now_ms_override.unwrap_or_else(|| d.as_millis() as u64);
    let seq = (SESSION_SEQ.fetch_add(1, Ordering::Relaxed) & 0xfff).max(1);
    let r = (now_ms << 12) | seq;
    let inv = (!r) & 0xffff_ffff_ffff;

    let mut state = d.as_nanos() as u64 ^ seq.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ now_ms;
    if state == 0 {
        state = 0xdead_beef_cafe_babe;
    }
    let mut tail = String::with_capacity(14);
    for _ in 0..14 {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        tail.push(BASE62[(state as usize) % 62] as char);
    }
    format!("ses_{inv:012x}{tail}")
}

/// Decode the embedded millisecond timestamp from a `ses_<12hex><14base62>` ID.
pub fn decode_session_timestamp_ms(session_id: &str) -> Option<u64> {
    let rest = session_id.strip_prefix("ses_")?;
    if rest.len() != 26 {
        return None;
    }
    let hex_part = &rest[..12];
    let inv = u64::from_str_radix(hex_part, 16).ok()?;
    let raw = (!inv) & 0xffff_ffff_ffff;
    Some(raw >> 12)
}

/// Binds `(conversation_seed, exit_key)` to a stable, fresh `ses_...` ID.
///
/// Session state and prompt caching are tracked per session and egress path.
/// Binding one session per `(conversation, exit_key)` preserves prompt cache hits
/// on the same connection while maintaining isolated session state across multi-homed paths.
pub struct SessionLedger {
    inner: Mutex<HashMap<String, (String, Instant)>>,
    remint_after: Duration,
    capacity: usize,
}

impl Default for SessionLedger {
    fn default() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            remint_after: Duration::from_secs(6 * 24 * 3600),
            capacity: 4096,
        }
    }
}

impl SessionLedger {
    pub fn session_for_exit(&self, conv_seed: &str, exit_key: &str) -> String {
        let seed = conv_seed.trim();
        if seed.is_empty() {
            return mint_session_id(None);
        }
        let composite = format!("{seed}\0exit\0{exit_key}");
        let mut guard = self.inner.lock().unwrap();
        if let Some((id, minted_at)) = guard.get(&composite) {
            if minted_at.elapsed() < self.remint_after {
                return id.clone();
            }
        }
        if guard.len() >= self.capacity {
            guard.clear();
        }
        let id = mint_session_id(None);
        guard.insert(composite, (id.clone(), Instant::now()));
        id
    }
}

/// Derive a stable conversation seed from the first user/system message in a request.
pub fn derive_conversation_seed(body: &Value) -> String {
    let mut hasher = Sha256::new();
    if let Some(msgs) = body.get("messages").and_then(|m| m.as_array()) {
        for m in msgs.iter().take(2) {
            if let Some(role) = m.get("role").and_then(|r| r.as_str()) {
                hasher.update(role.as_bytes());
            }
            if let Some(content) = m.get("content") {
                hasher.update(content.to_string().as_bytes());
            }
        }
    }
    let digest = hasher.finalize();
    format!("{:x}", digest)
}

/// Standard OpenCode v2.1.3 headers for upstream requests.
pub fn gateway_headers(session_id: &str, project_id: &str) -> Vec<(String, String)> {
    let proj = if project_id.trim().is_empty() {
        DEFAULT_PROJECT_ID
    } else {
        project_id
    };
    vec![
        ("Authorization".into(), "Bearer public".into()),
        ("Content-Type".into(), "application/json".into()),
        ("Accept".into(), "text/event-stream".into()),
        ("User-Agent".into(), OPENCODE_CLIENT_UA.into()),
        ("x-opencode-client".into(), "cli".into()),
        ("x-opencode-project".into(), proj.into()),
        ("x-opencode-session".into(), session_id.into()),
        ("x-opencode-session-id".into(), session_id.into()),
        ("x-session-affinity".into(), session_id.into()),
        ("x-session-id".into(), session_id.into()),
    ]
}

pub fn fingerprint_read_tool() -> Value {
    json!({
        "type": "function",
        "function": {
            "name": "read",
            "description": REAL_READ_DESC,
            "parameters": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "File or directory to read" },
                    "offset": { "type": "integer", "minimum": 0, "description": "The line or directory entry to start reading from (1-based)" },
                    "limit": { "type": "integer", "minimum": 0, "description": "The maximum number of lines or directory entries to read (defaults to and capped at 2000)" }
                },
                "required": ["path"],
                "additionalProperties": false
            },
            "strict": false
        }
    })
}

pub fn fingerprint_shell_tool() -> Value {
    json!({
        "type": "function",
        "function": {
            "name": "shell",
            "description": REAL_SHELL_DESC,
            "parameters": {
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "Shell command string to execute" },
                    "workdir": { "type": "string", "description": "Working directory to execute the command in. Defaults to the current working directory. When possible, avoid changing directories in the command and set the working directory here instead." },
                    "timeout": { "type": "integer", "minimum": 0, "description": "Timeout in milliseconds. Set to 0 to disable the timeout. Defaults to 120000 for foreground commands. Background commands have no timeout by default." },
                    "background": { "type": "boolean", "description": "Run the command in the background and return immediately (useful for dev servers and long-running builds). You do not need to use '&' at the end of the command when using this parameter. You will be notified when it completes. DO NOT poll for completion." }
                },
                "required": ["command"],
                "additionalProperties": false
            },
            "strict": false
        }
    })
}

#[derive(Debug, Clone)]
pub struct FingerprintPlan {
    /// True if downstream requested `stream: false` (so we must aggregate SSE into JSON).
    pub downstream_wants_stream: bool,
    /// True if downstream provided no tools at all (so placeholder tool calls should be dropped).
    pub drop_placeholder_tool_calls: bool,
    /// True if downstream had its own `read` or `shell` tool aliased to `*_client`.
    pub aliased_client_tools: bool,
}

/// Normalizes an OpenAI chat completion request body to conform to OpenCode v2.1.3
/// wire protocol framing requirements.
pub fn apply_fingerprint(body: &mut Value) -> FingerprintPlan {
    let downstream_wants_stream = body
        .get("stream")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    // 1. Force stream: true for upstream
    body["stream"] = json!(true);
    if body.get("stream_options").is_none() {
        body["stream_options"] = json!({"include_usage": true});
    }

    // 2. Normalize tool_choice: strip unsupported "none" value for upstream compatibility
    if body.get("tool_choice").and_then(|v| v.as_str()) == Some("none") {
        if let Some(obj) = body.as_object_mut() {
            obj.remove("tool_choice");
        }
    }

    // 3. Inspect and normalize/alias tool schemas
    let existing_tools = body
        .get("tools")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let caller_had_no_tools = existing_tools.is_empty();
    let mut aliased_read = false;
    let mut aliased_shell = false;
    let mut out_tools = Vec::with_capacity(existing_tools.len() + 2);

    for mut t in existing_tools {
        let name = t
            .get("function")
            .and_then(|f| f.get("name"))
            .and_then(|n| n.as_str())
            .unwrap_or("")
            .to_string();
        if name == "read" || name == "shell" {
            let desc = t
                .get("function")
                .and_then(|f| f.get("description"))
                .and_then(|d| d.as_str())
                .unwrap_or("");
            let is_exact = (name == "read" && desc == REAL_READ_DESC)
                || (name == "shell" && desc == REAL_SHELL_DESC);
            if !is_exact {
                if name == "read" {
                    aliased_read = true;
                } else {
                    aliased_shell = true;
                }
                if let Some(f) = t.get_mut("function").and_then(|f| f.as_object_mut()) {
                    f.insert("name".into(), json!(format!("{name}_client")));
                }
            }
        }
        // OpenCode 2.x marks every tool `strict: false`
        if let Some(f) = t.get_mut("function").and_then(|f| f.as_object_mut()) {
            f.entry("strict").or_insert(json!(false));
        }
        out_tools.push(t);
    }

    let has_exact_read = out_tools.iter().any(|t| {
        t.get("function")
            .and_then(|f| f.get("name"))
            .and_then(|n| n.as_str())
            == Some("read")
    });
    let has_exact_shell = out_tools.iter().any(|t| {
        t.get("function")
            .and_then(|f| f.get("name"))
            .and_then(|n| n.as_str())
            == Some("shell")
    });

    if !has_exact_read {
        out_tools.push(fingerprint_read_tool());
    }
    if !has_exact_shell {
        out_tools.push(fingerprint_shell_tool());
    }
    body["tools"] = json!(out_tools);

    // 3b. OpenCode 2.x client sets `max_completion_tokens: 32000` when no token ceiling is given
    if body.get("max_completion_tokens").is_none() && body.get("max_tokens").is_none() {
        body["max_completion_tokens"] = json!(32000);
    }

    let aliased = aliased_read || aliased_shell;
    // 4. If we aliased client `read`/`shell` tools, also alias historical assistant tool_calls
    if aliased {
        if let Some(msgs) = body.get_mut("messages").and_then(|m| m.as_array_mut()) {
            for msg in msgs.iter_mut() {
                if let Some(tcs) = msg.get_mut("tool_calls").and_then(|tc| tc.as_array_mut()) {
                    for tc in tcs.iter_mut() {
                        if let Some(f) = tc.get_mut("function").and_then(|f| f.as_object_mut()) {
                            if let Some(n) = f.get("name").and_then(|n| n.as_str()) {
                                if (n == "read" && aliased_read) || (n == "shell" && aliased_shell) {
                                    f.insert("name".into(), json!(format!("{n}_client")));
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // 5. Add system guidance so the model does not invoke compatibility placeholder tools
    let note = if caller_had_no_tools {
        NOTE_NO_TOOLS
    } else if aliased_read && aliased_shell {
        NOTE_ALIASED_BOTH
    } else if aliased_read {
        NOTE_ALIASED_READ_ONLY
    } else if aliased_shell {
        NOTE_ALIASED_SHELL_ONLY
    } else {
        NOTE_PLACEHOLDER_TOOLS
    };

    if let Some(msgs) = body.get_mut("messages").and_then(|m| m.as_array_mut()) {
        if let Some(first) = msgs.first_mut() {
            if first.get("role").and_then(|r| r.as_str()) == Some("system") {
                if let Some(c) = first.get("content").and_then(|c| c.as_str()) {
                    let combined = format!("{note}\n\n{c}");
                    first["content"] = json!(combined);
                }
            } else {
                msgs.insert(0, json!({"role": "system", "content": note}));
            }
        } else {
            msgs.push(json!({"role": "system", "content": note}));
        }
    }

    FingerprintPlan {
        downstream_wants_stream,
        drop_placeholder_tool_calls: caller_had_no_tools,
        aliased_client_tools: aliased,
    }
}

/// Unalias `read_client` -> `read` and `shell_client` -> `shell` in outbound tool calls.
pub fn unalias_tool_name(name: &str) -> &str {
    match name {
        "read_client" => "read",
        "shell_client" => "shell",
        other => other,
    }
}

/// Encodes a chat completion payload in the exact top-level key order emitted by
/// OpenCode 2.x's `JSON.stringify`:
/// `["model", "messages", "tools", "stream", "stream_options", "max_completion_tokens", ...]`
/// without HTML escaping `<`, `>`, or `&`.
pub fn encode_v2_chat_body(body: &Value) -> String {
    const V2_KEY_ORDER: &[&str] = &[
        "model",
        "messages",
        "tools",
        "stream",
        "stream_options",
        "max_completion_tokens",
    ];
    let Some(obj) = body.as_object() else {
        return serde_json::to_string(body).unwrap_or_else(|_| "{}".into());
    };
    let mut out = String::with_capacity(1024);
    out.push('{');
    let mut first = true;
    let mut write_kv = |k: &str, v: &Value| {
        if let (Ok(ks), Ok(vs)) = (serde_json::to_string(k), serde_json::to_string(v)) {
            if !first {
                out.push(',');
            }
            first = false;
            out.push_str(&ks);
            out.push(':');
            out.push_str(&vs);
        }
    };
    for &k in V2_KEY_ORDER {
        if let Some(v) = obj.get(k) {
            write_kv(k, v);
        }
    }
    let mut rest: Vec<&str> = obj
        .keys()
        .map(|s| s.as_str())
        .filter(|k| !V2_KEY_ORDER.contains(k))
        .collect();
    rest.sort_unstable();
    for k in rest {
        if let Some(v) = obj.get(k) {
            write_kv(k, v);
        }
    }
    out.push('}');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mint_and_decode_session_id_roundtrip() {
        let now_ms = 1_792_728_000_123u64;
        let sid = mint_session_id(Some(now_ms));
        assert!(sid.starts_with("ses_"));
        assert_eq!(sid.len(), 30);
        let decoded = decode_session_timestamp_ms(&sid).expect("valid hex prefix");
        assert_eq!(decoded, now_ms & 0x0f_ffff_ffff);
    }

    #[test]
    fn fingerprint_injects_tools_and_aliases_collisions() {
        let mut req = json!({
            "model": "deepseek-v4.1-flash-free",
            "stream": false,
            "tool_choice": "none",
            "messages": [{"role": "user", "content": "hi"}],
            "tools": [
                {
                    "type": "function",
                    "function": {
                        "name": "read",
                        "description": "Custom client read tool",
                        "parameters": {"type": "object"}
                    }
                }
            ]
        });

        let plan = apply_fingerprint(&mut req);
        assert!(!plan.downstream_wants_stream);
        assert!(!plan.drop_placeholder_tool_calls);
        assert!(plan.aliased_client_tools);
        assert_eq!(req["stream"], json!(true));
        assert!(req.get("tool_choice").is_none());

        let tools = req["tools"].as_array().unwrap();
        let names: Vec<&str> = tools
            .iter()
            .filter_map(|t| t["function"]["name"].as_str())
            .collect();
        assert!(names.contains(&"read_client"));
        assert!(names.contains(&"read"));
        assert!(names.contains(&"shell"));
        assert_eq!(unalias_tool_name("read_client"), "read");
    }

    #[test]
    fn encode_v2_chat_body_orders_keys_and_keeps_raw_symbols() {
        let body = json!({
            "max_completion_tokens": 32000,
            "stream": true,
            "messages": [{"role": "user", "content": "a<b>&c"}],
            "tools": [],
            "model": "beam-free",
            "stream_options": {"include_usage": true}
        });
        let s = encode_v2_chat_body(&body);
        let model_pos = s.find("\"model\"").unwrap();
        let messages_pos = s.find("\"messages\"").unwrap();
        let tools_pos = s.find("\"tools\"").unwrap();
        let stream_pos = s.find("\"stream\"").unwrap();
        let so_pos = s.find("\"stream_options\"").unwrap();
        let mct_pos = s.find("\"max_completion_tokens\"").unwrap();
        assert!(
            model_pos < messages_pos
                && messages_pos < tools_pos
                && tools_pos < stream_pos
                && stream_pos < so_pos
                && so_pos < mct_pos,
            "unexpected key order: {s}"
        );
        assert!(s.contains("a<b>&c"), "must not HTML-escape: {s}");
    }
}
