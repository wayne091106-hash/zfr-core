//! Background health prober: periodically probes every catalog model across all
//! configured egress paths and keeps the latest snapshot for `/v1/status`.
//!
//! The prober runs as an independent tokio task so the gateway stays responsive
//! while probing; probe results also seed the `(exit, model)` cooldown state in
//! `EgressPool`, so failures observed in the background immediately divert live
//! traffic to healthy routes.

use crate::router::{ProbeReport, Router};
use serde_json::json;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;

#[derive(Debug, Clone)]
pub struct HealthTracker {
    interval: Duration,
    inner: Arc<RwLock<Snapshot>>,
}

#[derive(Debug, Clone, Default)]
struct Snapshot {
    updated_at: u64,
    reports: Vec<ProbeReport>,
}

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

impl HealthTracker {
    /// Spawn a background task that probes all models every `interval`.
    /// Returns `None` when the interval is zero (prober disabled).
    pub fn spawn(router: Arc<Router>, interval: Duration) -> Option<Self> {
        if interval.is_zero() {
            return None;
        }
        let inner = Arc::new(RwLock::new(Snapshot::default()));
        let tracker = Self {
            interval,
            inner: Arc::clone(&inner),
        };
        tokio::spawn(async move {
            loop {
                let reports = router.probe_available_models().await;
                *inner.write().await = Snapshot {
                    updated_at: now_secs(),
                    reports,
                };
                tokio::time::sleep(interval).await;
            }
        });
        Some(tracker)
    }

    pub fn interval_secs(&self) -> u64 {
        self.interval.as_secs()
    }

    pub async fn snapshot(&self) -> (u64, Vec<ProbeReport>) {
        let snap = self.inner.read().await.clone();
        (snap.updated_at, snap.reports)
    }

    pub async fn summary_json(&self) -> serde_json::Value {
        let (updated_at, reports) = self.snapshot().await;
        let total = reports.len();
        let ok = reports.iter().filter(|r| r.ok).count();
        let rows: Vec<serde_json::Value> = reports
            .iter()
            .map(|r| {
                json!({
                    "model": r.model,
                    "ok": r.ok,
                    "status": r.status,
                    "latency_ms": r.latency_ms,
                    "exit_id": r.exit_id,
                    "detail": r.detail,
                })
            })
            .collect();
        json!({
            "enabled": true,
            "interval_secs": self.interval_secs(),
            "last_run_at": updated_at,
            "models_ok": ok,
            "models_total": total,
            "reports": rows,
        })
    }
}
