//! Model Catalog & Route Lane Definitions for OpenCode Zen (built-in native wire engine),
//! Kilo Free Pool, Cline, Groq Free, NVIDIA NIM Free, and OpenRouter Free Endpoints.

use serde::{Deserialize, Serialize};

pub const OPENCODE_ZEN_BASE: &str = "https://opencode.ai/inference/openai/v1";
pub const OPENCODE_LEGACY_BASE: &str = "https://opencode.ai/inference/v1";
pub const KILO_GATEWAY_BASE: &str = "https://api.kilo.ai/api/gateway/v1";
pub const CLINE_API_BASE: &str = "https://api.cline.bot/v1";
pub const GROQ_API_BASE: &str = "https://api.groq.com/openai/v1";
pub const NVIDIA_API_BASE: &str = "https://integrate.api.nvidia.com/v1";
pub const OPENROUTER_API_BASE: &str = "https://openrouter.ai/api/v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lane {
    OpenCodeZen,
    KiloFree,
    Cline,
    GroqFree,
    NvidiaFree,
    OpenRouterFree,
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
        // Cline Lane (api.cline.bot — OpenAI-compatible; free models carry the
        // `cline-free/` prefix or a partner `:free` id; per-model limits rotate).
        // Context windows below are best-effort from Cline's own model cache.
        ModelEntry {
            id: "z-ai/glm-5.3-flash".into(),
            lane: Lane::Cline,
            context_window: 204_800,
            max_output: 131_072,
            vision: false,
            reasoning: true,
        },
        ModelEntry {
            id: "cline-free/deepseek-v4.1-flash".into(),
            lane: Lane::Cline,
            context_window: 1_000_000,
            max_output: 65_536,
            vision: false,
            reasoning: true,
        },
        ModelEntry {
            id: "cline-free/mimo-v2.6-flash".into(),
            lane: Lane::Cline,
            context_window: 262_144,
            max_output: 65_536,
            vision: false,
            reasoning: true,
        },
        ModelEntry {
            id: "cline-free/muse-spark-1.3-contributor".into(),
            lane: Lane::Cline,
            context_window: 262_144,
            max_output: 65_536,
            vision: true,
            reasoning: true,
        },
        ModelEntry {
            id: "cline-free/solar-pro4".into(),
            lane: Lane::Cline,
            context_window: 131_072,
            max_output: 32_768,
            vision: false,
            reasoning: true,
        },
        ModelEntry {
            id: "poolside/laguna-s-2.1:free".into(),
            lane: Lane::Cline,
            context_window: 262_144,
            max_output: 65_536,
            vision: false,
            reasoning: true,
        },
    ]
}

/// Resolve a model string (including optional `opencode/`, `kilo/` or `cline/` prefix or role alias)
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
    if let Some(rest) = base.strip_prefix("cline/") {
        return (Lane::Cline, rest.to_string());
    }
    if let Some(rest) = base.strip_prefix("groq/") {
        return (Lane::GroqFree, rest.to_string());
    }
    if let Some(rest) = base.strip_prefix("nvidia/") {
        if rest.ends_with(":free") {
            return (Lane::KiloFree, format!("nvidia/{rest}"));
        }
        return (Lane::NvidiaFree, rest.to_string());
    }
    if let Some(rest) = base.strip_prefix("openrouter/") {
        if rest == "free" {
            return (Lane::KiloFree, "openrouter/free".to_string());
        }
        return (Lane::OpenRouterFree, rest.to_string());
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

/// Direct Kilo Gateway equivalent for an OpenCode Zen model ID, when the same model
/// (or model family) is hosted on both lanes.
pub fn kilo_equivalent(opencode_id: &str) -> Option<&'static str> {
    match opencode_id {
        "beam-free" => Some("reflection/beam-free"),
        "nemotron-3-ultra-free" => Some("nvidia/nemotron-3-ultra-550b-a55b:free"),
        "nemotron-3.5-lightning-free" => Some("nvidia/nemotron-3.5-lightning:free"),
        "nemotron-3.5-super-vl-free" => Some("nvidia/nemotron-3.5-super-vl:free"),
        "nemotron-3-super-free" => Some("nvidia/nemotron-3-super-120b-a12b:free"),
        "mimo-v2.6-flash-free" => Some("xiaomi/mimo-v2.6-flash:free"),
        "mimo-v2-pro-free" => Some("xiaomi/mimo-v2-pro:free"),
        "mimo-v2-omni-free" => Some("xiaomi/mimo-v2-omni:free"),
        "minimax-m3.1-flash-free" => Some("minimax/minimax-m3.1-flash:free"),
        "minimax-m2.5-free" => Some("minimax/minimax-m2.5:free"),
        "north-mini-code-free" => Some("cohere/north-mini-code:free"),
        "qwen3.6-plus-free" => Some("qwen/qwen3.6-plus:free"),
        _ => None,
    }
}

/// Direct OpenCode Zen equivalent for a Kilo Gateway model ID, when hosted on both keyless lanes.
pub fn opencode_equivalent(kilo_id: &str) -> Option<&'static str> {
    match kilo_id {
        "reflection/beam-free" => Some("beam-free"),
        "nvidia/nemotron-3-ultra-550b-a55b:free" => Some("nemotron-3-ultra-free"),
        "nvidia/nemotron-3.5-lightning:free" => Some("nemotron-3.5-lightning-free"),
        "nvidia/nemotron-3.5-super-vl:free" => Some("nemotron-3.5-super-vl-free"),
        "xiaomi/mimo-v2.6-flash:free" => Some("mimo-v2.6-flash-free"),
        _ => None,
    }
}

fn has_env_key(name: &str) -> bool {
    std::env::var(name)
        .map(|v| !v.trim().is_empty())
        .unwrap_or(false)
}

/// Ordered failover chain when all egress paths for `primary` are unavailable.
/// Bundles keyless OpenCode Zen + Kilo Free Pool first, plus optional Groq / NVIDIA / OpenRouter
/// free channels when their keys are present in the environment.
pub fn failover_chain(primary: &str) -> Vec<(Lane, String)> {
    let (primary_lane, primary_id) = resolve_model(primary);
    let mut chain = vec![(primary_lane, primary_id.clone())];

    if primary_lane == Lane::OpenCodeZen {
        if let Some(twin) = kilo_equivalent(&primary_id) {
            chain.push((Lane::KiloFree, twin.to_string()));
        }
    } else if primary_lane == Lane::KiloFree {
        if let Some(twin) = opencode_equivalent(&primary_id) {
            chain.push((Lane::OpenCodeZen, twin.to_string()));
        }
    }

    let defaults = [
        (Lane::KiloFree, "kilo-auto/free"),
        (Lane::KiloFree, "reflection/beam-free"),
        (Lane::KiloFree, "nvidia/nemotron-3.5-super-vl:free"),
        (Lane::OpenCodeZen, "beam-free"),
        (Lane::OpenCodeZen, "nemotron-3-ultra-free"),
        (Lane::OpenCodeZen, "deepseek-v4.1-flash-free"),
        (Lane::OpenCodeZen, "glm-5.3-flash-free"),
        (Lane::OpenCodeZen, "cat-jam-free"),
        (Lane::OpenCodeZen, "nemotron-3.5-lightning-free"),
    ];

    for (lane, id) in defaults {
        if !chain.iter().any(|(l, m)| *l == lane && m == id) {
            chain.push((lane, id.to_string()));
        }
    }

    if has_env_key("NVIDIA_API_KEY") {
        for id in ["z-ai/glm-5.3-flash", "deepseek-ai/deepseek-v4.1-flash"] {
            if !chain.iter().any(|(l, m)| *l == Lane::NvidiaFree && m == id) {
                chain.push((Lane::NvidiaFree, id.to_string()));
            }
        }
    }
    if has_env_key("GROQ_API_KEY") {
        for id in ["openai/gpt-oss-120b", "qwen/qwen3.8-27b"] {
            if !chain.iter().any(|(l, m)| *l == Lane::GroqFree && m == id) {
                chain.push((Lane::GroqFree, id.to_string()));
            }
        }
    }
    if has_env_key("OPENROUTER_API_KEY") {
        for id in ["minimax/minimax-m3.1-flash:free", "cohere/north-mini-code:free"] {
            if !chain.iter().any(|(l, m)| *l == Lane::OpenRouterFree && m == id) {
                chain.push((Lane::OpenRouterFree, id.to_string()));
            }
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
    fn resolve_model_cline_prefix() {
        assert_eq!(
            resolve_model("cline/z-ai/glm-5.3-flash"),
            (Lane::Cline, "z-ai/glm-5.3-flash".to_string())
        );
        assert_eq!(
            resolve_model("cline/cline-free/mimo-v2.6-flash"),
            (Lane::Cline, "cline-free/mimo-v2.6-flash".to_string())
        );
        // Without the `cline/` prefix, `provider/model` ids still resolve to the Kilo lane.
        assert_eq!(
            resolve_model("z-ai/glm-5.3-flash"),
            (Lane::KiloFree, "z-ai/glm-5.3-flash".to_string())
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
