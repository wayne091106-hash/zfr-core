//! Core Request Router, Multi-Homed Dispatcher & Upstream SSE Stream Normalization.

use crate::catalog::{
    builtin_catalog, failover_chain, resolve_model, Lane, CLINE_API_BASE, KILO_GATEWAY_BASE,
    OPENCODE_ZEN_BASE,
};
use crate::egress::{EgressNode, EgressPool, FailureClass};
use crate::fingerprint::{
    apply_fingerprint, derive_conversation_seed, encode_v2_chat_body, gateway_headers,
    unalias_tool_name, FingerprintPlan, DEFAULT_PROJECT_ID, OPENCODE_CLIENT_UA,
};
use bytes::Bytes;
use futures::{Stream, StreamExt};
use serde::Serialize;
use serde_json::{json, Value};
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

pub type ByteStream = Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send>>;

#[derive(Debug, Clone, Serialize)]
pub struct ProbeReport {
    pub model: String,
    pub lane: Lane,
    pub exit_id: String,
    pub status: u16,
    pub ok: bool,
    pub latency_ms: u64,
    pub detail: String,
}

pub struct Router {
    pub egress: Arc<EgressPool>,
    pub allow_model_failover: bool,
    pub max_exit_retries: usize,
}

pub enum RoutedResponse {
    Stream {
        served_model: String,
        exit_id: String,
        stream: ByteStream,
    },
    Json {
        served_model: String,
        exit_id: String,
        body: Value,
    },
}

impl Router {
    pub fn new(egress: Arc<EgressPool>, allow_model_failover: bool) -> Self {
        Self {
            egress,
            allow_model_failover,
            max_exit_retries: 3,
        }
    }

    /// Probe endpoint health for a single model across healthy egress paths.
    pub async fn probe_model(&self, model: &str) -> ProbeReport {
        let (lane, canonical) = resolve_model(model);
        let req = json!({
            "model": canonical,
            "messages": [{"role": "user", "content": "ping"}],
            "max_tokens": 12,
            "stream": true
        });
        let exits = self.egress.candidates_for("", &canonical, self.max_exit_retries);
        let mut last_report = ProbeReport {
            model: canonical.clone(),
            lane,
            exit_id: "none".into(),
            status: 0,
            ok: false,
            latency_ms: 0,
            detail: "no egress available".into(),
        };

        for exit in exits {
            let t0 = Instant::now();
            match self.send_upstream_once(&exit, lane, &canonical, "", req.clone()).await {
                Ok((resp, _plan)) => {
                    let status = resp.status().as_u16();
                    let mut stream = resp.bytes_stream();
                    let _ = stream.next().await;
                    let ms = t0.elapsed().as_millis() as u64;
                    self.egress.revive(&exit.id, &canonical);
                    return ProbeReport {
                        model: canonical,
                        lane,
                        exit_id: exit.id,
                        status,
                        ok: true,
                        latency_ms: ms,
                        detail: "200 OK".into(),
                    };
                }
                Err((status, class, retry_after, detail)) => {
                    let ms = t0.elapsed().as_millis() as u64;
                    self.egress.report_failure(
                        &exit.id,
                        &canonical,
                        class,
                        retry_after,
                        &detail,
                    );
                    last_report = ProbeReport {
                        model: canonical.clone(),
                        lane,
                        exit_id: exit.id,
                        status,
                        ok: false,
                        latency_ms: ms,
                        detail,
                    };
                }
            }
        }
        last_report
    }

    /// Probe all catalog model endpoints concurrently.
    pub async fn probe_all(self: &Arc<Self>) -> Vec<ProbeReport> {
        let entries = builtin_catalog();
        let mut tasks = Vec::with_capacity(entries.len());
        for entry in entries {
            let r = Arc::clone(self);
            tasks.push(tokio::spawn(async move { r.probe_model(&entry.id).await }));
        }
        let mut out = Vec::with_capacity(tasks.len());
        for t in tasks {
            if let Ok(rep) = t.await {
                out.push(rep);
            }
        }
        out
    }

    /// Quota-aware probe sweep for the background prober.
    ///
    /// Models whose every exit is on cooldown are reported as `skipped` without
    /// touching the upstream, so exhausted quotas are not probed (and their
    /// server-provided cooldown windows are not needlessly renewed). Models with
    /// at least one healthy exit are probed normally across the healthy paths.
    pub async fn probe_available_models(self: &Arc<Self>) -> Vec<ProbeReport> {
        let cooldowns = self.egress.active_cooldowns();
        let cooldown_by_model: std::collections::HashMap<String, (u64, String)> = cooldowns
            .iter()
            .map(|c| (c.model.clone(), (c.remaining_secs, c.class_str().to_string())))
            .collect();

        let entries = builtin_catalog();
        let mut pre: Vec<Option<ProbeReport>> = Vec::with_capacity(entries.len());
        let mut tasks = Vec::new();
        for entry in entries {
            if !self.egress.model_has_healthy_exit(&entry.id) {
                let (secs, class) = cooldown_by_model
                    .get(&entry.id)
                    .cloned()
                    .unwrap_or((0, "cooldown".to_string()));
                pre.push(Some(ProbeReport {
                    model: entry.id,
                    lane: entry.lane,
                    exit_id: "none".into(),
                    status: 0,
                    ok: false,
                    latency_ms: 0,
                    detail: format!("skipped: all exits on cooldown ({class}, {secs}s left)"),
                }));
                continue;
            }
            pre.push(None);
            let r = Arc::clone(self);
            let model = entry.id.clone();
            tasks.push(tokio::spawn(async move { r.probe_model(&model).await }));
        }

        let mut tasks = tasks.into_iter();
        let mut out = Vec::with_capacity(pre.len());
        for slot in pre {
            match slot {
                Some(rep) => out.push(rep),
                None => {
                    if let Some(handle) = tasks.next() {
                        if let Ok(rep) = handle.await {
                            out.push(rep);
                        }
                    }
                }
            }
        }
        out
    }

    /// Execute a chat completion request with multi-homed egress routing and optional failover.
    pub async fn execute_chat(&self, raw_body: Value) -> Result<RoutedResponse, (u16, Value)> {
        let requested_model = raw_body
            .get("model")
            .and_then(|v| v.as_str())
            .unwrap_or("beam-free")
            .to_string();
        let conv_seed = derive_conversation_seed(&raw_body);

        let chain = if self.allow_model_failover {
            failover_chain(&requested_model)
        } else {
            vec![resolve_model(&requested_model)]
        };

        let mut last_err: (u16, String) = (502, "no upstream candidates".into());

        for (lane, model_id) in chain {
            let exits = self
                .egress
                .candidates_for(&conv_seed, &model_id, self.max_exit_retries);

            for exit in exits {
                match self
                    .send_upstream_once(&exit, lane, &model_id, &conv_seed, raw_body.clone())
                    .await
                {
                    Ok((resp, plan)) => {
                        self.egress.revive(&exit.id, &model_id);
                        if plan.downstream_wants_stream {
                            let s = build_filtered_sse_stream(resp, plan);
                            return Ok(RoutedResponse::Stream {
                                served_model: model_id,
                                exit_id: exit.id,
                                stream: s,
                            });
                        } else {
                            let aggregated = aggregate_sse_to_json(resp, &model_id, &plan).await?;
                            return Ok(RoutedResponse::Json {
                                served_model: model_id,
                                exit_id: exit.id,
                                body: aggregated,
                            });
                        }
                    }
                    Err((status, class, retry_after, detail)) => {
                        self.egress.report_failure(
                            &exit.id,
                            &model_id,
                            class,
                            retry_after,
                            &detail,
                        );
                        last_err = (status.max(400), detail);
                    }
                }
            }
        }

        Err((
            last_err.0,
            json!({
                "error": {
                    "type": "upstream_error",
                    "message": last_err.1
                }
            }),
        ))
    }

    async fn send_upstream_once(
        &self,
        exit: &EgressNode,
        lane: Lane,
        model_id: &str,
        conv_seed: &str,
        mut body: Value,
    ) -> Result<(reqwest::Response, FingerprintPlan), (u16, FailureClass, Option<u64>, String)> {
        body["model"] = json!(model_id);

        let (url, headers, plan) = match lane {
            Lane::OpenCodeZen => {
                let plan = apply_fingerprint(&mut body);
                let sid = self.egress.sessions.session_for_exit(conv_seed, &exit.id);
                let hdrs = gateway_headers(&sid, DEFAULT_PROJECT_ID);
                (
                    format!("{OPENCODE_ZEN_BASE}/chat/completions"),
                    hdrs,
                    plan,
                )
            }
            Lane::KiloFree => {
                let downstream_wants_stream = body
                    .get("stream")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                body["stream"] = json!(true);
                let hdrs = vec![
                    ("Authorization".into(), "Bearer anonymous".into()),
                    ("Content-Type".into(), "application/json".into()),
                    ("Accept".into(), "text/event-stream".into()),
                    ("User-Agent".into(), OPENCODE_CLIENT_UA.into()),
                ];
                (
                    format!("{KILO_GATEWAY_BASE}/chat/completions"),
                    hdrs,
                    FingerprintPlan {
                        downstream_wants_stream,
                        drop_placeholder_tool_calls: false,
                        aliased_client_tools: false,
                    },
                )
            }
            Lane::Cline => {
                let downstream_wants_stream = body
                    .get("stream")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                // api.cline.bot speaks standard OpenAI wire format; upstream is
                // always SSE so the shared aggregation/streaming paths apply.
                body["stream"] = json!(true);
                let token = std::env::var("ZFR_CLINE_TOKEN")
                    .ok()
                    .map(|t| t.trim().to_string())
                    .filter(|t| !t.is_empty());
                let Some(token) = token else {
                    return Err((
                        500u16,
                        FailureClass::Transport,
                        None,
                        "cline lane requires ZFR_CLINE_TOKEN (create an API key at app.cline.bot)"
                            .to_string(),
                    ));
                };
                let hdrs = vec![
                    ("Authorization".into(), format!("Bearer {token}")),
                    ("Content-Type".into(), "application/json".into()),
                    ("Accept".into(), "text/event-stream".into()),
                ];
                (
                    format!("{CLINE_API_BASE}/chat/completions"),
                    hdrs,
                    FingerprintPlan {
                        downstream_wants_stream,
                        drop_placeholder_tool_calls: false,
                        aliased_client_tools: false,
                    },
                )
            }
        };

        let payload_bytes = match lane {
            Lane::OpenCodeZen => encode_v2_chat_body(&body).into_bytes(),
            Lane::KiloFree => serde_json::to_vec(&body).unwrap_or_default(),
            Lane::Cline => serde_json::to_vec(&body).unwrap_or_default(),
        };

        // Relay exits wrap the upstream request into a JSON envelope and let the
        // relay container perform it from its own (freshly booted) egress IP; the
        // upstream status, Retry-After and body are streamed back untouched, so
        // the failure-classification path below is shared with direct exits.
        let send = if let Some(relay) = &exit.relay {
            let envelope = build_relay_envelope(&url, &headers, &payload_bytes);
            let mut rb = exit.client.post(&relay.url).json(&envelope);
            if let Some(key) = &relay.key {
                rb = rb.header("x-zfr-relay-key", key);
            }
            rb
        } else {
            let mut rb = exit.client.post(&url).body(payload_bytes);
            for (k, v) in headers {
                rb = rb.header(k, v);
            }
            rb
        };

        let resp = send.send().await.map_err(|e| {
            (
                502u16,
                FailureClass::Transport,
                None,
                format!("transport error via {}: {}", exit.id, e),
            )
        })?;

        let status = resp.status().as_u16();
        if status >= 400 {
            let retry_after = resp
                .headers()
                .get("retry-after")
                .and_then(|h| h.to_str().ok())
                .and_then(|s| s.trim().parse::<u64>().ok());
            let err_text = resp.text().await.unwrap_or_default();
            let class = classify_http_failure(status, &err_text);
            let summary = extract_error_message(&err_text, retry_after);
            return Err((status, class, retry_after, summary));
        }

        Ok((resp, plan))
    }
}

/// Wrap an upstream request into the JSON envelope understood by `modal/relay.py`:
/// the relay performs the request from its own egress IP and streams the upstream
/// status, selected headers and body straight back.
fn build_relay_envelope(url: &str, headers: &[(String, String)], payload: &[u8]) -> Value {
    let mut hdrs = serde_json::Map::new();
    for (k, v) in headers {
        hdrs.insert(k.clone(), Value::String(v.clone()));
    }
    json!({
        "method": "POST",
        "url": url,
        "headers": Value::Object(hdrs),
        "body": String::from_utf8_lossy(payload),
    })
}

fn classify_http_failure(status: u16, body: &str) -> FailureClass {
    let lower = body.to_lowercase();
    if status == 429 || lower.contains("freeusagelimiterror") || lower.contains("usage limit") {
        FailureClass::Quota429
    } else if status == 403 && lower.contains("freetiererror") {
        FailureClass::FreeTier403
    } else if status == 403 || lower.contains("region") || lower.contains("country") {
        FailureClass::RegionBlock
    } else {
        FailureClass::Upstream5xx
    }
}

fn extract_error_message(raw: &str, retry_after: Option<u64>) -> String {
    let base = if let Ok(v) = serde_json::from_str::<Value>(raw) {
        v.get("error")
            .and_then(|e| e.get("message").or(Some(e)))
            .and_then(|m| m.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| raw.chars().take(160).collect())
    } else {
        raw.chars().take(160).collect()
    };
    if let Some(ra) = retry_after {
        format!("{base} (Retry-After: {ra}s)")
    } else {
        base
    }
}

/// Rewrites SSE stream lines to restore client tool names or remove compatibility placeholders.
fn build_filtered_sse_stream(resp: reqwest::Response, plan: FingerprintPlan) -> ByteStream {
    let s = futures::stream::unfold(
        (resp.bytes_stream(), String::new(), plan),
        |(mut byte_stream, mut buf, plan)| async move {
            loop {
                if let Some(pos) = buf.find('\n') {
                    let line = buf[..=pos].to_string();
                    buf.drain(..=pos);
                    if let Some(rewritten) = rewrite_sse_line(&line, &plan) {
                        return Some((Ok(Bytes::from(rewritten)), (byte_stream, buf, plan)));
                    }
                    continue;
                }
                match byte_stream.next().await {
                    Some(Ok(chunk)) => {
                        buf.push_str(&String::from_utf8_lossy(&chunk));
                    }
                    Some(Err(e)) => {
                        let err = std::io::Error::new(std::io::ErrorKind::Other, e);
                        return Some((Err(err), (byte_stream, buf, plan)));
                    }
                    None => {
                        if !buf.is_empty() {
                            let tail = std::mem::take(&mut buf);
                            if let Some(rewritten) = rewrite_sse_line(&tail, &plan) {
                                return Some((Ok(Bytes::from(rewritten)), (byte_stream, buf, plan)));
                            }
                        }
                        return None;
                    }
                }
            }
        },
    );
    Box::pin(s)
}

fn rewrite_sse_line(line: &str, plan: &FingerprintPlan) -> Option<String> {
    let trimmed = line.trim();
    if !trimmed.starts_with("data:") {
        return Some(line.to_string());
    }
    let payload = trimmed.strip_prefix("data:")?.trim();
    if payload == "[DONE]" || payload.is_empty() {
        return Some(line.to_string());
    }
    let Ok(mut v) = serde_json::from_str::<Value>(payload) else {
        return Some(line.to_string());
    };

    if let Some(choices) = v.get_mut("choices").and_then(|c| c.as_array_mut()) {
        for ch in choices.iter_mut() {
            if let Some(delta) = ch.get_mut("delta").and_then(|d| d.as_object_mut()) {
                if plan.drop_placeholder_tool_calls {
                    delta.remove("tool_calls");
                } else if plan.aliased_client_tools {
                    if let Some(tcs) = delta.get_mut("tool_calls").and_then(|t| t.as_array_mut()) {
                        for tc in tcs.iter_mut() {
                            if let Some(f) = tc.get_mut("function").and_then(|f| f.as_object_mut()) {
                                if let Some(n) = f.get("name").and_then(|n| n.as_str()) {
                                    let orig = unalias_tool_name(n).to_string();
                                    f.insert("name".into(), json!(orig));
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    Some(format!("data: {}\n\n", v))
}

/// Normalizes and aggregates an upstream SSE stream into a standard non-streaming OpenAI `chat.completion` JSON object.
///
/// Preserves the latest non-null token usage block rather than accumulating partial frames.
async fn aggregate_sse_to_json(
    resp: reqwest::Response,
    served_model: &str,
    plan: &FingerprintPlan,
) -> Result<Value, (u16, Value)> {
    let mut byte_stream = resp.bytes_stream();
    let mut buf = String::new();
    let mut content = String::new();
    let mut reasoning = String::new();
    let mut finish_reason = "stop".to_string();
    let mut latest_usage: Option<Value> = None;
    let mut tool_calls_by_idx: std::collections::BTreeMap<usize, (String, String, String)> =
        std::collections::BTreeMap::new();

    while let Some(chunk_res) = byte_stream.next().await {
        let chunk = chunk_res.map_err(|e| {
            (
                502u16,
                json!({"error": {"type": "stream_error", "message": e.to_string()}}),
            )
        })?;
        buf.push_str(&String::from_utf8_lossy(&chunk));
        while let Some(pos) = buf.find('\n') {
            let line = buf[..pos].trim().to_string();
            buf.drain(..=pos);
            let Some(payload) = line.strip_prefix("data:").map(|s| s.trim()) else {
                continue;
            };
            if payload.is_empty() || payload == "[DONE]" {
                continue;
            }
            let Ok(v) = serde_json::from_str::<Value>(payload) else {
                continue;
            };
            if let Some(u) = v.get("usage").filter(|u| !u.is_null()) {
                latest_usage = Some(u.clone());
            }
            if let Some(choices) = v.get("choices").and_then(|c| c.as_array()) {
                for ch in choices {
                    if let Some(fr) = ch.get("finish_reason").and_then(|f| f.as_str()) {
                        if !fr.is_empty() {
                            finish_reason = fr.to_string();
                        }
                    }
                    if let Some(delta) = ch.get("delta") {
                        if let Some(c) = delta.get("content").and_then(|c| c.as_str()) {
                            content.push_str(c);
                        }
                        if let Some(r) = delta
                            .get("reasoning_content")
                            .or_else(|| delta.get("reasoning"))
                            .and_then(|r| r.as_str())
                        {
                            reasoning.push_str(r);
                        }
                        if !plan.drop_placeholder_tool_calls {
                            if let Some(tcs) = delta.get("tool_calls").and_then(|t| t.as_array()) {
                                for (fallback_i, tc) in tcs.iter().enumerate() {
                                    let idx = tc
                                        .get("index")
                                        .and_then(|i| i.as_u64())
                                        .map(|i| i as usize)
                                        .unwrap_or(fallback_i);
                                    let entry = tool_calls_by_idx.entry(idx).or_insert_with(|| {
                                        (format!("call_{idx}"), String::new(), String::new())
                                    });
                                    if let Some(id) = tc.get("id").and_then(|s| s.as_str()) {
                                        if !id.is_empty() {
                                            entry.0 = id.to_string();
                                        }
                                    }
                                    if let Some(f) = tc.get("function") {
                                        if let Some(n) = f.get("name").and_then(|s| s.as_str()) {
                                            if !n.is_empty() {
                                                entry.1 = if plan.aliased_client_tools {
                                                    unalias_tool_name(n).to_string()
                                                } else {
                                                    n.to_string()
                                                };
                                            }
                                        }
                                        if let Some(a) = f.get("arguments").and_then(|s| s.as_str()) {
                                            entry.2.push_str(a);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let mut msg = json!({
        "role": "assistant",
        "content": content,
    });
    if !reasoning.is_empty() {
        msg["reasoning_content"] = json!(reasoning);
    }
    if !tool_calls_by_idx.is_empty() {
        let tcs: Vec<Value> = tool_calls_by_idx
            .into_values()
            .map(|(id, name, arguments)| {
                json!({
                    "id": id,
                    "type": "function",
                    "function": {
                        "name": name,
                        "arguments": arguments
                    }
                })
            })
            .collect();
        msg["tool_calls"] = json!(tcs);
    }

    let mut out = json!({
        "id": format!("chatcmpl-zfr-{now_secs}"),
        "object": "chat.completion",
        "created": now_secs,
        "model": served_model,
        "choices": [{
            "index": 0,
            "message": msg,
            "finish_reason": finish_reason
        }]
    });
    if let Some(u) = latest_usage {
        out["usage"] = u;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fingerprint::FingerprintPlan;

    #[test]
    fn classify_maps_status_and_body() {
        assert!(matches!(
            classify_http_failure(429, ""),
            FailureClass::Quota429
        ));
        assert!(matches!(
            classify_http_failure(200, r#"{"error":{"message":"FreeUsageLimitError"}}"#),
            FailureClass::Quota429
        ));
        assert!(matches!(
            classify_http_failure(403, "FreeTierError: upgrade required"),
            FailureClass::FreeTier403
        ));
        assert!(matches!(
            classify_http_failure(403, "not allowed in your region"),
            FailureClass::RegionBlock
        ));
        assert!(matches!(
            classify_http_failure(502, "bad gateway"),
            FailureClass::Upstream5xx
        ));
    }

    #[test]
    fn extract_error_message_appends_retry_after() {
        let raw = r#"{"error":{"message":"rate limited"}}"#;
        assert_eq!(
            extract_error_message(raw, Some(30)),
            "rate limited (Retry-After: 30s)"
        );
        assert_eq!(extract_error_message("plain boom", None), "plain boom");
    }

    #[test]
    fn rewrite_sse_line_drops_placeholder_tool_calls() {
        let line = r#"data: {"choices":[{"delta":{"tool_calls":[{"id":"c1"}]}}]}"#;
        let plan = FingerprintPlan {
            downstream_wants_stream: true,
            drop_placeholder_tool_calls: true,
            aliased_client_tools: false,
        };
        let out = rewrite_sse_line(line, &plan).unwrap();
        assert!(!out.contains("tool_calls"), "{out}");
    }

    #[test]
    fn rewrite_sse_line_unaliases_client_tools() {
        let line =
            r#"data: {"choices":[{"delta":{"tool_calls":[{"function":{"name":"shell_client","arguments":"{}"}}]}}]}"#;
        let plan = FingerprintPlan {
            downstream_wants_stream: true,
            drop_placeholder_tool_calls: false,
            aliased_client_tools: true,
        };
        let out = rewrite_sse_line(line, &plan).unwrap();
        assert!(out.contains(r#""shell""#), "{out}");
        assert!(!out.contains("shell_client"), "{out}");
    }

    #[test]
    fn rewrite_sse_line_passes_through_plain_lines() {
        let plan = FingerprintPlan {
            downstream_wants_stream: true,
            drop_placeholder_tool_calls: false,
            aliased_client_tools: false,
        };
        assert_eq!(
            rewrite_sse_line("data: [DONE]", &plan).as_deref(),
            Some("data: [DONE]")
        );
        assert_eq!(
            rewrite_sse_line(":keepalive", &plan).as_deref(),
            Some(":keepalive")
        );
    }

    #[test]
    fn relay_envelope_carries_url_headers_and_body() {
        let headers = vec![
            ("Authorization".to_string(), "Bearer anonymous".to_string()),
            ("Content-Type".to_string(), "application/json".to_string()),
        ];
        let v = build_relay_envelope(
            "https://up.example/chat/completions",
            &headers,
            br#"{"a":1}"#,
        );
        assert_eq!(v["method"], "POST");
        assert_eq!(v["url"], "https://up.example/chat/completions");
        assert_eq!(v["headers"]["Authorization"], "Bearer anonymous");
        assert_eq!(v["headers"]["Content-Type"], "application/json");
        assert_eq!(v["body"], r#"{"a":1}"#);
    }
}
