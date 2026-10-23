//! Low-level Multi-Egress IP & Proxy Rotator with Per-(Exit, Model) Quota Ledger.
//!
//! Solves two key failure modes:
//! 1. `429 FreeUsageLimitError`: OpenCode Zen accounts free quota per `(egress_ip, model)`.
//!    Rotating across multiple proxy/interface exits multiplies available daily quota.
//! 2. `403 FreeTierError` on dirty exits: instead of marking the entire model lane dead
//!    (the bug in `zen-gate` v1.7.14 commit `727aa60`), `EgressPool` quarantines only
//!    that `(exit, model)` pair and immediately retries on the next healthy exit.

use crate::fingerprint::SessionLedger;
use reqwest::Client;
use serde::Serialize;
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureClass {
    Quota429,
    FreeTier403,
    RegionBlock,
    Upstream5xx,
    Transport,
}

#[derive(Debug, Clone)]
pub struct EgressNode {
    pub id: String,
    pub spec: String,
    pub client: Client,
}

#[derive(Debug, Clone, Serialize)]
pub struct CooldownEntry {
    pub exit_id: String,
    pub model: String,
    pub class: FailureClass,
    pub remaining_secs: u64,
    pub detail: String,
}

struct BanRecord {
    until: Instant,
    class: FailureClass,
    detail: String,
}

pub struct EgressPool {
    nodes: Vec<EgressNode>,
    cursor: AtomicUsize,
    bans: Mutex<HashMap<(String, String), BanRecord>>,
    pub sessions: SessionLedger,
}

impl Default for EgressPool {
    fn default() -> Self {
        Self::new(&["direct".to_string()])
    }
}

impl EgressPool {
    /// Build an egress pool from specs like:
    /// - `"auto"` (expands to `["ipv6", "ipv4"]` dual-stack exits — ideal for Cloudflare WARP 1.1.1.1)
    /// - `"ipv6"` (forces IPv6 `[::]` socket binding)
    /// - `"ipv4"` (forces IPv4 `0.0.0.0` socket binding)
    /// - `"direct"` (default OS dual-stack route)
    /// - `"http://127.0.0.1:7890"` / `"socks5://127.0.0.1:1080"`
    /// - `"bind://192.168.31.135"` (bind socket to a specific local interface IP)
    pub fn new(specs: &[String]) -> Self {
        let mut expanded = Vec::new();
        for raw in specs {
            let s = raw.trim();
            if s.eq_ignore_ascii_case("auto") {
                expanded.push("ipv6".to_string());
                expanded.push("ipv4".to_string());
            } else if !s.is_empty() {
                expanded.push(s.to_string());
            }
        }
        let mut nodes = Vec::new();
        for (idx, spec) in expanded.iter().enumerate() {
            if let Ok(client) = build_client_for_spec(spec) {
                let id = match spec.as_str() {
                    "direct" | "ipv6" | "ipv4" => spec.clone(),
                    _ => format!("exit-{idx}:{spec}"),
                };
                nodes.push(EgressNode {
                    id,
                    spec: spec.clone(),
                    client,
                });
            }
        }
        if nodes.is_empty() {
            nodes.push(EgressNode {
                id: "direct".to_string(),
                spec: "direct".to_string(),
                client: build_client_for_spec("direct").expect("default reqwest client"),
            });
        }
        Self {
            nodes,
            cursor: AtomicUsize::new(0),
            bans: Mutex::new(HashMap::new()),
            sessions: SessionLedger::default(),
        }
    }

    pub fn nodes(&self) -> &[EgressNode] {
        &self.nodes
    }

    /// Pick up to `max_candidates` healthy exits for `model`, preferring a stable exit
    /// for `conv_seed` first (to maximize prompt cache hits) and rotating through the rest.
    pub fn candidates_for(&self, conv_seed: &str, model: &str, max_candidates: usize) -> Vec<EgressNode> {
        let n = self.nodes.len();
        if n == 0 {
            return vec![];
        }
        let start = if !conv_seed.is_empty() {
            let hash = conv_seed
                .bytes()
                .fold(0usize, |acc, b| acc.wrapping_mul(31).wrapping_add(b as usize));
            hash % n
        } else {
            self.cursor.fetch_add(1, Ordering::Relaxed) % n
        };

        let now = Instant::now();
        let bans = self.bans.lock().unwrap();
        let mut healthy = Vec::with_capacity(n);
        let mut fallback = Vec::new();

        for offset in 0..n {
            let node = &self.nodes[(start + offset) % n];
            let key = (node.id.clone(), model.to_string());
            if let Some(ban) = bans.get(&key) {
                if ban.until > now {
                    fallback.push(node.clone());
                    continue;
                }
            }
            healthy.push(node.clone());
            if healthy.len() >= max_candidates {
                break;
            }
        }

        // If all exits are currently on cooldown, still allow one probe attempt on the earliest one
        if healthy.is_empty() && !fallback.is_empty() {
            healthy.push(fallback.remove(0));
        }
        healthy
    }

    /// Record a failure on `(exit_id, model)` so subsequent turns rotate away from it.
    pub fn report_failure(
        &self,
        exit_id: &str,
        model: &str,
        class: FailureClass,
        retry_after_secs: Option<u64>,
        detail: impl Into<String>,
    ) {
        let ttl = match class {
            FailureClass::Quota429 => {
                Duration::from_secs(retry_after_secs.unwrap_or(1800).clamp(30, 86400))
            }
            FailureClass::FreeTier403 | FailureClass::RegionBlock => Duration::from_secs(900),
            FailureClass::Upstream5xx => Duration::from_secs(45),
            FailureClass::Transport => Duration::from_secs(30),
        };
        let mut bans = self.bans.lock().unwrap();
        bans.insert(
            (exit_id.to_string(), model.to_string()),
            BanRecord {
                until: Instant::now() + ttl,
                class,
                detail: detail.into(),
            },
        );
    }

    /// Clear any failure record on `(exit_id, model)` after a live 200 OK.
    pub fn revive(&self, exit_id: &str, model: &str) {
        let mut bans = self.bans.lock().unwrap();
        bans.remove(&(exit_id.to_string(), model.to_string()));
    }

    /// Snapshot active cooldowns for `/v1/egress` diagnostics.
    pub fn active_cooldowns(&self) -> Vec<CooldownEntry> {
        let now = Instant::now();
        let mut bans = self.bans.lock().unwrap();
        bans.retain(|_, v| v.until > now);
        bans.iter()
            .map(|((exit_id, model), rec)| CooldownEntry {
                exit_id: exit_id.clone(),
                model: model.clone(),
                class: rec.class,
                remaining_secs: rec.until.saturating_duration_since(now).as_secs(),
                detail: rec.detail.clone(),
            })
            .collect()
    }
}

fn build_client_for_spec(spec: &str) -> Result<Client, reqwest::Error> {
    let mut builder = Client::builder()
        .connect_timeout(Duration::from_secs(20))
        .pool_idle_timeout(Duration::from_secs(60))
        .tcp_keepalive(Duration::from_secs(30));

    if spec.eq_ignore_ascii_case("ipv6") {
        builder = builder.local_address(IpAddr::V6(Ipv6Addr::UNSPECIFIED));
    } else if spec.eq_ignore_ascii_case("ipv4") {
        builder = builder.local_address(IpAddr::V4(Ipv4Addr::UNSPECIFIED));
    } else if let Some(ip_str) = spec
        .strip_prefix("bind://")
        .or_else(|| spec.strip_prefix("interface://"))
    {
        if let Ok(ip) = ip_str.trim().parse::<IpAddr>() {
            builder = builder.local_address(ip);
        }
    } else if spec != "direct" {
        let clean_proxy = spec.trim_end_matches("#pool");
        let proxy = reqwest::Proxy::all(clean_proxy)?;
        // Disable keep-alive on rotating proxy tunnels so rotating proxies don't pin a single exit
        builder = builder.proxy(proxy).pool_max_idle_per_host(0);
    }

    builder.build()
}
