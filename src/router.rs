//! Core Request Router, Multi-Exit Executor & SSE Aggregator.

use crate::catalog::{
    builtin_catalog, failover_chain, resolve_model, Lane, KILO_GATEWAY_BASE, OPENCODE_ZEN_BASE,
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

    /// Probe a single model on the first healthy exit (rotating on failure).
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

    /// Probe all built-in OpenCode Zen and Kilo Free models concurrently.
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

    /// Execute a chat completion request with multi-exit IP rotation and optional model failover.
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
        };

        let payload_bytes = match lane {
            Lane::OpenCodeZen => encode_v2_chat_body(&body).into_bytes(),
            Lane::KiloFree => serde_json::to_vec(&body).unwrap_or_default(),
        };
        let mut rb = exit.client.post(&url).body(payload_bytes);
        for (k, v) in headers {
            rb = rb.header(k, v);
        }

        let resp = rb.send().await.map_err(|e| {
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

/// Rewrites SSE lines on the fly to unalias `read_client`/`shell_client` or strip inert placeholder tool calls.
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

/// Aggregates an upstream SSE stream into a single non-streaming OpenAI `chat.completion` JSON object.
///
/// Also avoids the `zen-gate` Issue #105 bug where cumulative `usage` chunks were summed together:
/// we overwrite `latest_usage` with the newest non-null usage block rather than adding cumulative frames.
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
