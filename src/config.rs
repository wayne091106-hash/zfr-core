//! Optional JSON configuration file support.
//!
//! Search order (first match wins):
//! 1. Explicit `--config <path>` (error if missing)
//! 2. `$ZFR_CONFIG`
//! 3. `./zfr.json` (current working directory)
//! 4. Machine-wide config: `%PROGRAMDATA%\zfr\config.json` (Windows) / `~/.config/zfr/config.json` (unix)
//! 5. Per-user config: `%LOCALAPPDATA%\zfr\config.json` (Windows)
//!
//! CLI flags always override config file values. Missing files simply yield defaults.

use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Listen address for the gateway, e.g. `"127.0.0.1:18788"`.
    pub bind: Option<SocketAddr>,
    /// Egress specs: `auto`, `direct`, `ipv6`, `ipv4`, `http://...`, `socks5://...`, `bind://...`,
    /// `relay://https://relay-host#key=...` (serverless fresh-IP egress, see modal/relay.py).
    pub egress: Option<Vec<String>>,
    /// Disable cross-model automatic failover.
    pub no_failover: Option<bool>,
    /// Require clients to present `Authorization: Bearer <key>`.
    pub api_key: Option<String>,
    /// Background health prober interval in seconds (0 disables the prober).
    pub probe_interval_secs: Option<u64>,
}

impl Config {
    /// Load the effective configuration file, if any. Returns `(config, path)`.
    pub fn load(explicit: Option<&Path>) -> Result<(Config, Option<PathBuf>), String> {
        let path = match explicit {
            Some(p) => {
                if !p.exists() {
                    return Err(format!("config file not found: {}", p.display()));
                }
                Some(p.to_path_buf())
            }
            None => Self::discover(),
        };
        let Some(path) = path else {
            return Ok((Config::default(), None));
        };
        let raw = std::fs::read_to_string(&path)
            .map_err(|e| format!("failed to read config {}: {e}", path.display()))?;
        let cfg: Config = serde_json::from_str(&raw)
            .map_err(|e| format!("failed to parse config {}: {e}", path.display()))?;
        Ok((cfg, Some(path)))
    }

    /// Persist this config as JSON, creating parent directories as needed.
    pub fn save_to(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
        }
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(path, json).map_err(|e| format!("cannot write {}: {e}", path.display()))
    }

    fn discover() -> Option<PathBuf> {
        if let Ok(p) = std::env::var("ZFR_CONFIG") {
            let p = PathBuf::from(p);
            if p.exists() {
                return Some(p);
            }
        }
        let cwd = PathBuf::from("zfr.json");
        if cwd.exists() {
            return Some(cwd);
        }
        candidate_paths().into_iter().find(|p| p.exists())
    }
}

fn candidate_paths() -> Vec<PathBuf> {
    let mut out = Vec::new();
    #[cfg(windows)]
    {
        if let Ok(pd) = std::env::var("PROGRAMDATA") {
            out.push(PathBuf::from(pd).join("zfr").join("config.json"));
        }
        if let Ok(la) = std::env::var("LOCALAPPDATA") {
            out.push(PathBuf::from(la).join("zfr").join("config.json"));
        }
    }
    #[cfg(not(windows))]
    {
        if let Ok(h) = std::env::var("HOME") {
            out.push(PathBuf::from(h).join(".config/zfr/config.json"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_explicit_file_is_an_error() {
        assert!(Config::load(Some(Path::new("definitely-missing-zfr.json"))).is_err());
    }

    #[test]
    fn roundtrip_save_and_load() {
        let dir = std::env::temp_dir().join(format!("zfr-cfg-test-{}", std::process::id()));
        let path = dir.join("config.json");
        let cfg = Config {
            bind: Some("127.0.0.1:19999".parse().unwrap()),
            egress: Some(vec!["ipv4".into(), "socks5://127.0.0.1:1080".into()]),
            no_failover: Some(true),
            api_key: Some("secret".into()),
            probe_interval_secs: Some(60),
        };
        cfg.save_to(&path).expect("save");
        let (loaded, _) = Config::load(Some(&path)).expect("load");
        assert_eq!(loaded.bind, cfg.bind);
        assert_eq!(loaded.egress, cfg.egress);
        assert_eq!(loaded.no_failover, cfg.no_failover);
        assert_eq!(loaded.api_key, cfg.api_key);
        assert_eq!(loaded.probe_interval_secs, cfg.probe_interval_secs);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn partial_file_fills_defaults() {
        let dir = std::env::temp_dir().join(format!("zfr-cfg-test2-{}", std::process::id()));
        let path = dir.join("config.json");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, r#"{"bind": "0.0.0.0:8080"}"#).unwrap();
        let (loaded, _) = Config::load(Some(&path)).expect("load");
        assert_eq!(loaded.bind, Some("0.0.0.0:8080".parse().unwrap()));
        assert!(loaded.egress.is_none());
        assert_eq!(loaded.probe_interval_secs, None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
