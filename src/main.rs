use clap::{Parser, Subcommand};
use futures::StreamExt;
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::sync::Arc;
use zfr_core::{
    catalog::Lane,
    egress::EgressPool,
    router::{RoutedResponse, Router},
    server::build_app,
};

#[derive(Parser)]
#[command(
    name = "zfr",
    version,
    about = "ZFR Core — OpenCode Zen & Kilo Free Model Router with Multi-Egress IP Rotation"
)]
struct Cli {
    /// Comma-separated or repeated egress specs: `auto` (IPv6+IPv4 dual-stack), `direct`, `ipv6`, `ipv4`, `http://host:port`, `socks5://host:port`, or `bind://192.168.x.x`
    #[arg(long = "egress", short = 'e', global = true, value_delimiter = ',', default_value = "auto")]
    egress: Vec<String>,

    /// Disable cross-model automatic failover when the requested model is exhausted on all exits
    #[arg(long = "no-failover", global = true)]
    no_failover: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Probe all OpenCode Zen & Kilo free models concurrently across configured exits
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
        #[arg(short = 'b', long = "bind", default_value = "127.0.0.1:18788")]
        bind: SocketAddr,
    },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();
    let pool = Arc::new(EgressPool::new(&cli.egress));
    let router = Arc::new(Router::new(pool, !cli.no_failover));

    match cli.command {
        Commands::Probe => {
            println!("=== ZFR Core Live Probe (exits: {}) ===", router.egress.nodes().len());
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
        Commands::Run { model, prompt } => {
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
                        let chunk = chunk_res?;
                        buf.push_str(&String::from_utf8_lossy(&chunk));
                        while let Some(pos) = buf.find('\n') {
                            let line = buf[..pos].trim().to_string();
                            buf.drain(..=pos);
                            if let Some(payload) = line.strip_prefix("data:").map(|s| s.trim()) {
                                if payload == "[DONE]" || payload.is_empty() {
                                    continue;
                                }
                                if let Ok(v) = serde_json::from_str::<Value>(payload) {
                                    if let Some(delta) = v
                                        .get("choices")
                                        .and_then(|c| c.get(0))
                                        .and_then(|c| c.get("delta"))
                                    {
                                        if let Some(c) = delta.get("content").and_then(|c| c.as_str()) {
                                            print!("{c}");
                                        }
                                    }
                                }
                            }
                        }
                    }
                    println!();
                }
                Ok(RoutedResponse::Json { body, .. }) => {
                    println!("{}", serde_json::to_string_pretty(&body)?);
                }
                Err((status, err)) => {
                    eprintln!("HTTP {status}: {err}");
                    std::process::exit(1);
                }
            }
        }
        Commands::Serve { bind } => {
            println!("ZFR Core listening on http://{bind}/v1 (exits: {})", router.egress.nodes().len());
            let app = build_app(router);
            let listener = tokio::net::TcpListener::bind(bind).await?;
            axum::serve(listener, app).await?;
        }
    }

    Ok(())
}
