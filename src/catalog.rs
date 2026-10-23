//! Model Catalog & Route Lane Definitions for OpenCode and Kilo Endpoints.

use serde::{Deserialize, Serialize};

pub const OPENCODE_ZEN_BASE: &str = "https://opencode.ai/inference/openai/v1";
pub const OPENCODE_LEGACY_BASE: &str = "https://opencode.ai/inference/v1";
pub const KILO_GATEWAY_BASE: &str = "https://api.kilo.ai/api/gateway/v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lane {
    OpenCodeZen,
    KiloFree,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelEntry {
    pub id: String,
    pub lane: Lane,
    pub context_window: u64,
    pub max_output: u64,
    pub vision: bool,
    pub reasoning: bool,
}

pub fn builtin_catalog() -> Vec<ModelEntry> {
    vec![
        // OpenCode Lane
        ModelEntry {
            id: "beam-free".into(),
            lane: Lane::OpenCodeZen,
            context_window: 1_048_576,
            max_output: 131_072,
            vision: false,
            reasoning: true,
        },
        ModelEntry {
            id: "cat-jam-free".into(),
            lane: Lane::OpenCodeZen,
            context_window: 1_000_000,
            max_output: 131_072,
            vision: false,
            reasoning: true,
        },
        ModelEntry {
            id: "nemotron-3-ultra-free".into(),
            lane: Lane::OpenCodeZen,
            context_window: 1_000_000,
            max_output: 131_072,
            vision: false,
            reasoning: true,
        },
        ModelEntry {
            id: "nemotron-3.5-lightning-free".into(),
            lane: Lane::OpenCodeZen,
            context_window: 262_144,
            max_output: 262_144,
            vision: false,
            reasoning: true,
        },
        ModelEntry {
            id: "flint-40b-free".into(),
            lane: Lane::OpenCodeZen,
            context_window: 262_144,
            max_output: 65_536,
            vision: false,
            reasoning: true,
        },
        ModelEntry {
            id: "deepseek-v4.1-flash-free".into(),
            lane: Lane::OpenCodeZen,
            context_window: 262_144,
            max_output: 65_536,
            vision: false,
            reasoning: true,
        },
        ModelEntry {
            id: "glm-5.3-flash-free".into(),
            lane: Lane::OpenCodeZen,
            context_window: 204_800,
            max_output: 131_072,
            vision: false,
            reasoning: true,
        },
        ModelEntry {
            id: "mimo-v2.6-flash-free".into(),
            lane: Lane::OpenCodeZen,
            context_window: 262_144,
            max_output: 65_536,
            vision: false,
            reasoning: true,
        },
        ModelEntry {
            id: "nemotron-3.5-super-vl-free".into(),
            lane: Lane::OpenCodeZen,
            context_window: 262_144,
            max_output: 65_536,
            vision: true,
            reasoning: true,
        },
        ModelEntry {
            id: "big-pickle".into(),
            lane: Lane::OpenCodeZen,
            context_window: 200_000,
            max_output: 32_000,
            vision: false,
            reasoning: true,
        },
        // Kilo Lane
        ModelEntry {
            id: "kilo-auto/free".into(),
            lane: Lane::KiloFree,
            context_window: 256_000,
            max_output: 65_536,
            vision: true,
            reasoning: true,
        },
        ModelEntry {
            id: "reflection/beam-free".into(),
            lane: Lane::KiloFree,
            context_window: 1_048_576,
            max_output: 131_072,
            vision: false,
            reasoning: true,
        },
        ModelEntry {
            id: "nvidia/nemotron-3.5-super-vl:free".into(),
            lane: Lane::KiloFree,
            context_window: 1_000_000,
            max_output: 65_536,
            vision: true,
            reasoning: true,
        },
        ModelEntry {
            id: "nvidia/nemotron-3-ultra-550b-a55b:free".into(),
            lane: Lane::KiloFree,
            context_window: 1_000_000,
            max_output: 65_536,
            vision: false,
            reasoning: true,
        },
        ModelEntry {
            id: "nvidia/nemotron-3-super-120b-a12b:free".into(),
            lane: Lane::KiloFree,
            context_window: 262_144,
            max_output: 65_536,
            vision: false,
            reasoning: true,
        },
        ModelEntry {
            id: "nvidia/nemotron-3.5-lightning:free".into(),
            lane: Lane::KiloFree,
            context_window: 1_000_000,
            max_output: 65_536,
            vision: false,
            reasoning: true,
        },
        ModelEntry {
            id: "cohere/north-mini-code:free".into(),
            lane: Lane::KiloFree,
            context_window: 256_000,
            max_output: 65_536,
            vision: false,
            reasoning: true,
        },
        ModelEntry {
            id: "poolside/laguna-s-2.1:free".into(),
            lane: Lane::KiloFree,
            context_window: 262_144,
            max_output: 65_536,
            vision: false,
            reasoning: true,
        },
        ModelEntry {
            id: "dots-studio/dots-3-note-preview:free".into(),
            lane: Lane::KiloFree,
            context_window: 512_000,
            max_output: 65_536,
            vision: true,
            reasoning: true,
        },
        ModelEntry {
            id: "recursive/flint-v1.0:free".into(),
            lane: Lane::KiloFree,
            context_window: 262_144,
            max_output: 65_536,
            vision: false,
            reasoning: true,
        },
        ModelEntry {
            id: "xingchen-agi/xing4.0-29b-a4b:free".into(),
            lane: Lane::KiloFree,
            context_window: 262_144,
            max_output: 65_536,
            vision: false,
            reasoning: true,
        },
        ModelEntry {
            id: "stealth/glyph-cluster".into(),
            lane: Lane::KiloFree,
            context_window: 256_000,
            max_output: 65_536,
            vision: false,
            reasoning: true,
        },
        ModelEntry {
            id: "openrouter/free".into(),
            lane: Lane::KiloFree,
            context_window: 200_000,
            max_output: 65_536,
            vision: false,
            reasoning: true,
        },
    ]
}

/// Resolve a model string (including optional `opencode/` or `kilo/` prefix or role alias)
/// into `(Lane, canonical_model_id)`.
pub fn resolve_model(raw: &str) -> (Lane, String) {
    let trimmed = raw.trim();
    let base = if let Some(idx) = trimmed.rfind('(') {
        if trimmed.ends_with(')') {
            trimmed[..idx].trim()
        } else {
            trimmed
        }
    } else {
        trimmed
    };

    if let Some(rest) = base.strip_prefix("opencode/") {
        return (Lane::OpenCodeZen, rest.to_string());
    }
    if let Some(rest) = base.strip_prefix("kilo/") {
        return (Lane::KiloFree, rest.to_string());
    }

    match base {
        "auto" | "main" | "fast" => (Lane::OpenCodeZen, "beam-free".to_string()),
        "deep" => (Lane::OpenCodeZen, "nemotron-3-ultra-free".to_string()),
        "code" => (Lane::OpenCodeZen, "deepseek-v4.1-flash-free".to_string()),
        "vision" | "vl" => (Lane::KiloFree, "nvidia/nemotron-3.5-super-vl:free".to_string()),
        other if other.contains('/') || other.ends_with(":free") => {
            (Lane::KiloFree, other.to_string())
        }
        other => (Lane::OpenCodeZen, other.to_string()),
    }
}

/// Ordered failover chain when all egress paths for `primary` are unavailable.
pub fn failover_chain(primary: &str) -> Vec<(Lane, String)> {
    let (primary_lane, primary_id) = resolve_model(primary);
    let mut chain = vec![(primary_lane, primary_id.clone())];

    let defaults = [
        (Lane::OpenCodeZen, "beam-free"),
        (Lane::OpenCodeZen, "nemotron-3-ultra-free"),
        (Lane::OpenCodeZen, "cat-jam-free"),
        (Lane::OpenCodeZen, "nemotron-3.5-lightning-free"),
        (Lane::KiloFree, "kilo-auto/free"),
        (Lane::KiloFree, "reflection/beam-free"),
        (Lane::KiloFree, "nvidia/nemotron-3.5-super-vl:free"),
    ];

    for (lane, id) in defaults {
        if !(lane == primary_lane && id == primary_id) {
            chain.push((lane, id.to_string()));
        }
    }
    chain
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_model_prefixes_and_aliases() {
        assert_eq!(
            resolve_model("opencode/beam-free"),
            (Lane::OpenCodeZen, "beam-free".to_string())
        );
        assert_eq!(
            resolve_model("kilo/kilo-auto/free"),
            (Lane::KiloFree, "kilo-auto/free".to_string())
        );
        assert_eq!(
            resolve_model("nvidia/nemotron-3.5-lightning:free"),
            (
                Lane::KiloFree,
                "nvidia/nemotron-3.5-lightning:free".to_string()
            )
        );
        assert_eq!(
            resolve_model("auto"),
            (Lane::OpenCodeZen, "beam-free".to_string())
        );
        assert_eq!(
            resolve_model("code"),
            (Lane::OpenCodeZen, "deepseek-v4.1-flash-free".to_string())
        );
        assert_eq!(
            resolve_model("vision"),
            (
                Lane::KiloFree,
                "nvidia/nemotron-3.5-super-vl:free".to_string()
            )
        );
        assert_eq!(
            resolve_model("beam-free"),
            (Lane::OpenCodeZen, "beam-free".to_string())
        );
    }

    #[test]
    fn resolve_model_strips_role_suffix() {
        assert_eq!(
            resolve_model("beam-free(fast)"),
            (Lane::OpenCodeZen, "beam-free".to_string())
        );
    }

    #[test]
    fn failover_chain_starts_with_primary_and_no_duplicates() {
        let chain = failover_chain("deepseek-v4.1-flash-free");
        assert_eq!(
            chain[0],
            (Lane::OpenCodeZen, "deepseek-v4.1-flash-free".to_string())
        );
        let unique: std::collections::HashSet<_> = chain
            .iter()
            .map(|(l, m)| (format!("{l:?}"), m.clone()))
            .collect();
        assert_eq!(unique.len(), chain.len(), "chain entries must be unique");
        assert!(chain.len() > 5, "failover chain should offer fallbacks");
    }

    #[test]
    fn failover_chain_primary_not_repeated() {
        let chain = failover_chain("beam-free");
        let count = chain
            .iter()
            .filter(|(l, m)| *l == Lane::OpenCodeZen && m == "beam-free")
            .count();
        assert_eq!(count, 1);
    }
}
