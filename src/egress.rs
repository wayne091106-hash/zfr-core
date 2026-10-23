//! Multi-Homed IPv6/IPv4 Dual-Stack Transport & Endpoint Health Pool.
//!
//! Provides high-availability egress routing and connection health management:
//! 1. Rate-limit and concurrency distribution: balances traffic across multiple dual-stack
//!    or proxy egress interfaces, isolating rate limit windows per `(egress_node, model)`.
//! 2. Isolated endpoint cooldown: when an egress path experiences rate limits or handshake
//!    errors, `EgressPool` temporarily marks only that specific `(exit, model)` pair for
//!    cooldown and immediately dispatches requests to alternative healthy routes.

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

impl std::fmt::Display for FailureClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FailureClass::Quota429 => write!(f, "quota429"),
            FailureClass::FreeTier403 => write!(f, "free_tier403"),
            FailureClass::RegionBlock => write!(f, "region_block"),
            FailureClass::Upstream5xx => write!(f, "upstream5xx"),
            FailureClass::Transport => write!(f, "transport"),
        }
    }
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
    /// Consecutive failures while already on cooldown; drives exponential backoff.
    attempts: u32,
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
    /// Build an egress pool from transport route specifications:
    /// - `"auto"` (expands to `["ipv6", "ipv4"]` dual-stack sockets — compatible with Cloudflare WARP and multi-stack networks)
    /// - `"ipv6"` (explicit IPv6 `[::]` socket binding)
    /// - `"ipv4"` (explicit IPv4 `0.0.0.0` socket binding)
    /// - `"direct"` (default OS routing table)
    /// - `"http://127.0.0.1:7890"` / `"socks5://127.0.0.1:1080"` (proxy transport endpoints)
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

    /// Record a transport or endpoint failure on `(exit_id, model)` to initiate a cooldown window.
    ///
    /// Repeated failures on an already-cooled pair back off exponentially (up to 1h),
    /// so a persistently broken route stops churning the prober and failover loop.
    /// `Quota429` windows are server-driven and always honor `Retry-After` as-is.
    pub fn report_failure(
        &self,
        exit_id: &str,
        model: &str,
        class: FailureClass,
        retry_after_secs: Option<u64>,
        detail: impl Into<String>,
    ) {
        let base_ttl = match class {
            FailureClass::Quota429 => {
                Duration::from_secs(retry_after_secs.unwrap_or(1800).clamp(30, 86400))
            }
            FailureClass::FreeTier403 | FailureClass::RegionBlock => Duration::from_secs(900),
            FailureClass::Upstream5xx => Duration::from_secs(45),
            FailureClass::Transport => Duration::from_secs(30),
        };
        let mut bans = self.bans.lock().unwrap();
        let now = Instant::now();
        let key = (exit_id.to_string(), model.to_string());
        let attempts = match bans.get(&key) {
            Some(prev) if prev.until > now => prev.attempts.saturating_add(1),
            _ => 0,
        };
        let ttl = match class {
            FailureClass::Quota429 => base_ttl,
            _ => base_ttl
                .mul_f64(2f64.powi(attempts.min(8) as i32))
                .min(Duration::from_secs(3600)),
        };
        bans.insert(
            key,
            BanRecord {
                until: now + ttl,
                class,
                detail: detail.into(),
                attempts,
            },
        );
    }

    /// Clear cooldown status on `(exit_id, model)` upon receiving a successful 200 OK response.
    pub fn revive(&self, exit_id: &str, model: &str) {
        let mut bans = self.bans.lock().unwrap();
        bans.remove(&(exit_id.to_string(), model.to_string()));
    }

    /// True when at least one exit for `model` is not on cooldown.
    ///
    /// Used by the background prober to skip models whose quota is exhausted on
    /// every exit, instead of burning upstream requests to renew long bans.
    pub fn model_has_healthy_exit(&self, model: &str) -> bool {
        let now = Instant::now();
        let bans = self.bans.lock().unwrap();
        self.nodes.iter().any(|n| {
            match bans.get(&(n.id.clone(), model.to_string())) {
                Some(ban) => ban.until <= now,
                None => true,
            }
        })
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

impl CooldownEntry {
    /// Stable snake_case label for logs and reports.
    pub fn class_str(&self) -> &'static str {
        match self.class {
            FailureClass::Quota429 => "quota429",
            FailureClass::FreeTier403 => "free_tier403",
            FailureClass::RegionBlock => "region_block",
            FailureClass::Upstream5xx => "upstream5xx",
            FailureClass::Transport => "transport",
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_expands_to_dual_stack() {
        let pool = EgressPool::new(&["auto".to_string()]);
        let ids: Vec<&str> = pool.nodes().iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ids, vec!["ipv6", "ipv4"]);
    }

    #[test]
    fn empty_specs_fall_back_to_direct() {
        let pool = EgressPool::new(&[]);
        assert_eq!(pool.nodes().len(), 1);
        assert_eq!(pool.nodes()[0].id, "direct");
    }

    #[test]
    fn failure_cooldowns_are_per_exit_model_pair() {
        let pool = EgressPool::new(&["direct".to_string()]);
        pool.report_failure("direct", "beam-free", FailureClass::Upstream5xx, None, "boom");
        assert_eq!(pool.active_cooldowns().len(), 1);
        assert_eq!(pool.active_cooldowns()[0].model, "beam-free");
        // The banned pair still gets a single fallback probe attempt...
        assert_eq!(pool.candidates_for("", "beam-free", 3).len(), 1);
        // ...but an unrelated model on the same exit is unaffected.
        assert_eq!(pool.candidates_for("", "cat-jam-free", 3).len(), 1);
        pool.revive("direct", "beam-free");
        assert_eq!(pool.active_cooldowns().len(), 0);
    }

    #[test]
    fn candidates_skip_banned_exit() {
        let pool = EgressPool::new(&["ipv6".to_string(), "ipv4".to_string()]);
        pool.report_failure("ipv6", "beam-free", FailureClass::Transport, None, "down");
        let candidates = pool.candidates_for("", "beam-free", 3);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].id, "ipv4");
    }

    #[test]
    fn proxy_and_bind_specs_build_clients() {
        let pool = EgressPool::new(&[
            "socks5://127.0.0.1:1080".to_string(),
            "bind://192.168.1.10".to_string(),
        ]);
        assert_eq!(pool.nodes().len(), 2);
        assert!(pool.nodes()[0].id.starts_with("exit-"));
    }

    #[test]
    fn repeat_failures_back_off_exponentially() {
        let pool = EgressPool::new(&["direct".to_string()]);
        pool.report_failure("direct", "m", FailureClass::Transport, None, "x");
        let c0 = pool.active_cooldowns()[0].remaining_secs;
        assert!((25..=30).contains(&c0), "first failure: {c0}s");
        pool.report_failure("direct", "m", FailureClass::Transport, None, "x");
        let c1 = pool.active_cooldowns()[0].remaining_secs;
        assert!(c1 > c0 && c1 <= 60, "second failure should double: {c1}s");
        pool.report_failure("direct", "m", FailureClass::Transport, None, "x");
        let c2 = pool.active_cooldowns()[0].remaining_secs;
        assert!(c2 > c1 && c2 <= 120, "third failure should double again: {c2}s");
    }

    #[test]
    fn quota_cooldowns_honor_retry_after_without_backoff() {
        let pool = EgressPool::new(&["direct".to_string()]);
        pool.report_failure("direct", "q", FailureClass::Quota429, Some(100), "quota");
        pool.report_failure("direct", "q", FailureClass::Quota429, Some(100), "quota");
        let remaining = pool.active_cooldowns()[0].remaining_secs;
        assert!(
            (95..=100).contains(&remaining),
            "quota window must follow Retry-After, not double: {remaining}s"
        );
    }

    #[test]
    fn model_has_healthy_exit_reflects_cooldowns() {
        let pool = EgressPool::new(&["ipv6".to_string(), "ipv4".to_string()]);
        assert!(pool.model_has_healthy_exit("m"));
        pool.report_failure("ipv6", "m", FailureClass::Transport, None, "x");
        assert!(pool.model_has_healthy_exit("m"), "one healthy exit left");
        pool.report_failure("ipv4", "m", FailureClass::Transport, None, "x");
        assert!(!pool.model_has_healthy_exit("m"), "all exits cooled down");
    }
}
