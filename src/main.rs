use clap::{Parser, Subcommand};
use futures::StreamExt;
use serde_json::{json, Value};
use std::io::Write as _;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use zfr_core::{
    catalog::Lane,
    config::Config,
    daemon,
    egress::EgressPool,
    health::HealthTracker,
    router::{RoutedResponse, Router},
    server::{build_app, AppState},
};

const DEFAULT_BIND: &str = "127.0.0.1:18788";
const DEFAULT_PROBE_INTERVAL: u64 = 300;

#[derive(Parser)]
#[command(
    name = "zfr",
    version,
    about = "ZFR Core — High-Performance Model Router & Multi-Homed Gateway"
)]
struct Cli {
    /// Comma-separated or repeated egress specs: `auto` (IPv6+IPv4 dual-stack), `direct`,
    /// `ipv6`, `ipv4`, `http://host:port`, `socks5://host:port`, or `bind://192.168.x.x`
    #[arg(long = "egress", short = 'e', global = true, value_delimiter = ',', env = "ZFR_EGRESS")]
    egress: Option<Vec<String>>,

    /// Require gateway clients to present `Authorization: Bearer <key>` (env: ZFR_API_KEY)
    #[arg(long = "api-key", global = true, env = "ZFR_API_KEY")]
    api_key: Option<String>,

    /// Disable cross-model automatic failover when the requested model is unavailable on all exits
    #[arg(long = "no-failover", global = true)]
    no_failover: bool,

    /// Optional JSON config file (env: ZFR_CONFIG); CLI flags override its values
    #[arg(long = "config", global = true, env = "ZFR_CONFIG")]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Probe all catalog models concurrently across configured egress paths
    Probe,
    /// Run a single prompt from the CLI
    Run {
        #[arg(short = 'm', long = "model", default_value = "beam-free")]
        model: String,
        /// Prompt text
        prompt: String,
    },
    /// Start the local OpenAI-compatible HTTP gateway server
    Serve {
        #[arg(short = 'b', long = "bind", env = "ZFR_BIND",
              help = "Listen address [default: 127.0.0.1:18788]")]
        bind: Option<SocketAddr>,
        /// Run detached in the background (log: %LOCALAPPDATA%\\zfr\\zfr.log)
        #[arg(long = "daemon")]
        daemon: bool,
        /// Background health prober interval in seconds, 0 disables (env: ZFR_PROBE_INTERVAL)
        #[arg(long = "probe-interval", env = "ZFR_PROBE_INTERVAL")]
        probe_interval: Option<u64>,
    },
    /// Stop a background instance started with `serve --daemon`
    Stop,
    /// Manage the Windows service installation (requires Administrator)
    Service {
        #[command(subcommand)]
        action: ServiceAction,
    },
    /// (internal) Service Control Manager entry point
    #[command(hide = true)]
    ServiceRun,
}

#[derive(Subcommand)]
#[allow(dead_code)]
enum ServiceAction {
    /// Install the Windows service (auto-start at boot, restarts on failure)
    Install {
        #[arg(short = 'b', long = "bind",
              help = "Listen address [default: 127.0.0.1:18788]")]
        bind: Option<SocketAddr>,
        /// Background prober interval in seconds, 0 disables
        #[arg(long = "probe-interval")]
        probe_interval: Option<u64>,
    },
    Uninstall,
    Start,
    Stop,
    /// Show the current service state
    Status,
}

struct Settings {
    bind: SocketAddr,
    egress: Vec<String>,
    allow_failover: bool,
    api_key: Option<String>,
    probe_interval: u64,
}

/// Merge CLI flags > env vars (already applied by clap) > config file > defaults.
fn resolve_settings(
    egress: &Option<Vec<String>>,
    api_key: &Option<String>,
    no_failover: bool,
    config: Option<&PathBuf>,
    bind_override: Option<SocketAddr>,
    interval_override: Option<u64>,
) -> Result<Settings, String> {
    let (cfg, _path) = Config::load(config.map(|p| p.as_path()))?;
    Ok(Settings {
        bind: bind_override
            .or(cfg.bind)
            .unwrap_or_else(|| DEFAULT_BIND.parse().expect("valid default bind")),
        egress: egress
            .clone()
            .or(cfg.egress)
            .unwrap_or_else(|| vec!["auto".to_string()]),
        allow_failover: !(no_failover || cfg.no_failover.unwrap_or(false)),
        api_key: api_key.clone().or(cfg.api_key).filter(|s| !s.is_empty()),
        probe_interval: interval_override.or(cfg.probe_interval_secs).unwrap_or(DEFAULT_PROBE_INTERVAL),
    })
}

fn build_router(s: &Settings) -> Arc<Router> {
    let pool = Arc::new(EgressPool::new(&s.egress));
    Arc::new(Router::new(pool, s.allow_failover))
}

fn init_tracing() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    eprintln!("\n[zfr] shutdown signal received, stopping gateway...");
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let Cli {
        egress,
        api_key,
        no_failover,
        config,
        command,
    } = Cli::parse();

    match command {
        Commands::ServiceRun => {
            #[cfg(windows)]
            {
                zfr_core::service::dispatch().unwrap_or_else(|e| {
                    eprintln!("{e}");
                    std::process::exit(1);
                });
                return Ok(());
            }
            #[cfg(not(windows))]
            {
                eprintln!("service commands are only supported on Windows");
                std::process::exit(1);
            }
        }
        Commands::Service { action } => {
            init_tracing();
            handle_service(action, &egress, &api_key, no_failover, config.as_ref())?;
        }
        Commands::Stop => {
            init_tracing();
            if let Err(e) = daemon::stop() {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
        Commands::Probe => {
            init_tracing();
            let s = resolve_settings(&egress, &api_key, no_failover, config.as_ref(), None, None)
                .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
            let router = build_router(&s);
            run_probe(&router).await;
        }
        Commands::Run { model, prompt } => {
            init_tracing();
            let s = resolve_settings(&egress, &api_key, no_failover, config.as_ref(), None, None)
                .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
            let router = build_router(&s);
            run_prompt(router, model, prompt).await;
        }
        Commands::Serve {
            bind,
            daemon: daemon_mode,
            probe_interval,
        } => {
            init_tracing();
            let s = resolve_settings(
                &egress,
                &api_key,
                no_failover,
                config.as_ref(),
                bind,
                probe_interval,
            )
            .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
            let router = build_router(&s);

            if daemon_mode {
                let opts = daemon::ServeOptions {
                    bind: s.bind,
                    egress: s.egress,
                    no_failover: !s.allow_failover,
                    api_key: s.api_key.clone(),
                    probe_interval: s.probe_interval,
                    config: config.clone(),
                };
                match daemon::spawn_detached(&opts) {
                    Ok(pid) => {
                        println!("zfr-core daemon started (pid {pid})");
                        println!("  endpoint: http://{}/v1", s.bind);
                        println!("  log     : {}", daemon::log_file().display());
                        println!("  stop    : zfr stop");
                    }
                    Err(e) => {
                        eprintln!("{e}");
                        std::process::exit(1);
                    }
                }
                return Ok(());
            }

            let exits = router.egress.nodes().len();
            let health =
                HealthTracker::spawn(Arc::clone(&router), Duration::from_secs(s.probe_interval));
            let state = AppState {
                router,
                health,
                api_key: s.api_key.clone(),
                started_at: Instant::now(),
            };
            let app = build_app(state);
            let listener = tokio::net::TcpListener::bind(s.bind).await?;
            println!(
                "ZFR Core v{} listening on http://{}/v1 (exits: {exits})",
                env!("CARGO_PKG_VERSION"),
                s.bind
            );
            println!(
                "  prober  : {}",
                if s.probe_interval > 0 {
                    format!("every {}s", s.probe_interval)
                } else {
                    "disabled".to_string()
                }
            );
            println!(
                "  api-key : {}",
                if s.api_key.is_some() { "required" } else { "none" }
            );
            println!(
                "  failover: {}",
                if s.allow_failover { "on" } else { "off" }
            );
            axum::serve(listener, app)
                .with_graceful_shutdown(shutdown_signal())
                .await?;
        }
    }

    Ok(())
}

fn handle_service(
    action: ServiceAction,
    egress: &Option<Vec<String>>,
    api_key: &Option<String>,
    no_failover: bool,
    config: Option<&PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(windows)]
    {
        return service_dispatch(action, egress, api_key, no_failover, config);
    }
    #[cfg(not(windows))]
    {
        let _ = (action, egress, api_key, no_failover, config);
        eprintln!("service commands are only supported on Windows");
        std::process::exit(1);
    }
}

#[cfg(windows)]
fn service_dispatch(
    action: ServiceAction,
    egress: &Option<Vec<String>>,
    api_key: &Option<String>,
    no_failover: bool,
    config: Option<&PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    use zfr_core::service::{self, InstallOptions};
    match action {
        ServiceAction::Install {
            bind,
            probe_interval,
        } => {
            let (cfg, _) = Config::load(config.map(|p| p.as_path()))?;
            service::install(InstallOptions {
                bind: bind.or(cfg.bind),
                egress: egress.clone().or(cfg.egress),
                no_failover: no_failover || cfg.no_failover.unwrap_or(false),
                api_key: api_key.clone().or(cfg.api_key),
                probe_interval: probe_interval.or(cfg.probe_interval_secs),
            })
            .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
        }
        ServiceAction::Uninstall => {
            service::uninstall().map_err(|e| -> Box<dyn std::error::Error> { e.into() })?
        }
        ServiceAction::Start => {
            service::start().map_err(|e| -> Box<dyn std::error::Error> { e.into() })?
        }
        ServiceAction::Stop => {
            service::stop().map_err(|e| -> Box<dyn std::error::Error> { e.into() })?
        }
        ServiceAction::Status => {
            service::status().map_err(|e| -> Box<dyn std::error::Error> { e.into() })?
        }
    }
    Ok(())
}

async fn run_probe(router: &Arc<Router>) {
    println!(
        "=== ZFR Core Live Probe (exits: {}) ===",
        router.egress.nodes().len()
    );
    let reports = router.probe_all().await;
    for r in reports {
        let tag = if r.ok { "OK " } else { "ERR" };
        let lane_str = match r.lane {
            Lane::OpenCodeZen => "opencode-zen",
            Lane::KiloFree => "kilo-free   ",
        };
        println!(
            "  [{tag}] {lane_str} | {:42} | HTTP {:3} ({:4}ms) via {:12} | {}",
            r.model, r.status, r.latency_ms, r.exit_id, r.detail
        );
    }
}

async fn run_prompt(router: Arc<Router>, model: String, prompt: String) {
    let body = json!({
        "model": model,
        "messages": [{"role": "user", "content": prompt}],
        "stream": true
    });
    match router.execute_chat(body).await {
        Ok(RoutedResponse::Stream {
            served_model,
            exit_id,
            mut stream,
        }) => {
            eprintln!("[zfr] served by {served_model} via {exit_id}");
            let mut buf = String::new();
            while let Some(chunk_res) = stream.next().await {
                let chunk = match chunk_res {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("stream error: {e}");
                        std::process::exit(1);
                    }
                };
                buf.push_str(&String::from_utf8_lossy(&chunk));
                while let Some(pos) = buf.find('\n') {
                    let line = buf[..pos].trim().to_string();
                    buf.drain(..=pos);
                    if let Some(payload) = line.strip_prefix("data:").map(|s| s.trim()) {
                        if payload == "[DONE]" || payload.is_empty() {
                            continue;
                        }
                        if let Ok(v) = serde_json::from_str::<Value>(payload) {
                            if let Some(c) = v
                                .get("choices")
                                .and_then(|c| c.get(0))
                                .and_then(|c| c.get("delta"))
                                .and_then(|d| d.get("content"))
                                .and_then(|c| c.as_str())
                            {
                                print!("{c}");
                                let _ = std::io::stdout().flush();
                            }
                        }
                    }
                }
            }
            println!();
        }
        Ok(RoutedResponse::Json { body, .. }) => {
            println!("{}", serde_json::to_string_pretty(&body).unwrap_or_default());
        }
        Err((status, err)) => {
            eprintln!("HTTP {status}: {err}");
            std::process::exit(1);
        }
    }
}
