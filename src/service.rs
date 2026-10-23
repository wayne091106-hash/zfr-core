//! Windows Service integration for the gateway (`zfr service ...`).
//!
//! Installs `zfr-core` as an auto-start Windows Service with restart-on-failure
//! recovery, running `zfr service-run` under the Service Control Manager.
//! Install/uninstall/start/stop require an elevated (Administrator) shell.

use crate::config::Config;
use crate::daemon::data_dir;
use crate::egress::EgressPool;
use crate::health::HealthTracker;
use crate::router::Router;
use crate::server::{build_app, AppState};
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};
use windows_service::{
    define_windows_service,
    service::{
        ServiceAccess, ServiceControl, ServiceControlAccept, ServiceErrorControl,
        ServiceExitCode, ServiceInfo, ServiceStartType, ServiceState, ServiceStatus, ServiceType,
    },
    service_control_handler::{self, ServiceControlHandlerResult},
    service_dispatcher,
    service_manager::{ServiceManager, ServiceManagerAccess},
};

pub const SERVICE_NAME: &str = "zfr-core";

/// Inputs captured from CLI flags at install time; persisted to the machine
/// config so the service (running as LocalSystem) can reload them at boot.
pub struct InstallOptions {
    pub bind: Option<std::net::SocketAddr>,
    pub egress: Option<Vec<String>>,
    pub no_failover: bool,
    pub api_key: Option<String>,
    pub probe_interval: Option<u64>,
}

fn machine_config_path() -> PathBuf {
    std::env::var("PROGRAMDATA")
        .map(|p| PathBuf::from(p).join("zfr").join("config.json"))
        .unwrap_or_else(|_| data_dir().join("config.json"))
}

fn manager(access: ServiceManagerAccess) -> Result<ServiceManager, String> {
    ServiceManager::local_computer(None::<&str>, access)
        .map_err(|e| format!("opening the service manager failed (run as Administrator?): {e}"))
}

pub fn install(opts: InstallOptions) -> Result<(), String> {
    let cfg = Config {
        bind: opts.bind,
        egress: opts.egress,
        no_failover: Some(opts.no_failover),
        api_key: opts.api_key.filter(|s| !s.is_empty()),
        probe_interval_secs: opts.probe_interval,
    };
    let path = machine_config_path();
    cfg.save_to(&path)?;

    let exe = std::env::current_exe().map_err(|e| format!("cannot locate executable: {e}"))?;
    let mgr = manager(ServiceManagerAccess::CREATE_SERVICE)?;
    let info = ServiceInfo {
        name: OsString::from(SERVICE_NAME),
        display_name: OsString::from("ZFR Core Gateway"),
        service_type: ServiceType::OWN_PROCESS,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path: exe,
        launch_arguments: vec![
            OsString::from("--config"),
            path.clone().into_os_string(),
            OsString::from("service-run"),
        ],
        dependencies: Vec::new(),
        account_name: None,
        account_password: None,
    };
    let service = mgr
        .create_service(&info, ServiceAccess::CHANGE_CONFIG)
        .map_err(|e| format!("creating service failed (already installed?): {e}"))?;
    let _ = service.set_description(
        "OpenAI-compatible local gateway with multi-homed routing (zfr-core)",
    );
    // Restart the service 10s after an unexpected exit (windows-service 0.7 has no
    // setter, so configure recovery through `sc.exe` — best effort).
    let _ = Command::new("sc")
        .args(["failure", SERVICE_NAME, "reset=", "0", "actions=", "restart/10000"])
        .output();

    println!("service '{SERVICE_NAME}' installed");
    println!("  config : {}", path.display());
    println!("  startup: automatic (restarts 10s after failure)");
    println!("start it with: zfr service start");
    Ok(())
}

pub fn uninstall() -> Result<(), String> {
    let mgr = manager(ServiceManagerAccess::CONNECT)?;
    let service = mgr
        .open_service(SERVICE_NAME, ServiceAccess::DELETE | ServiceAccess::STOP)
        .map_err(|e| format!("service '{SERVICE_NAME}' is not installed ({e})"))?;
    let _ = service.stop();
    service
        .delete()
        .map_err(|e| format!("deleting service failed: {e}"))?;
    println!("service '{SERVICE_NAME}' removed");
    Ok(())
}

pub fn start() -> Result<(), String> {
    let mgr = manager(ServiceManagerAccess::CONNECT)?;
    let service = mgr
        .open_service(SERVICE_NAME, ServiceAccess::START | ServiceAccess::QUERY_STATUS)
        .map_err(|e| format!("service '{SERVICE_NAME}' is not installed ({e})"))?;
    service
        .start(&[] as &[&str])
        .map_err(|e| format!("starting service failed: {e}"))?;
    println!("service '{SERVICE_NAME}' started");
    Ok(())
}

pub fn stop() -> Result<(), String> {
    let mgr = manager(ServiceManagerAccess::CONNECT)?;
    let service = mgr
        .open_service(SERVICE_NAME, ServiceAccess::STOP | ServiceAccess::QUERY_STATUS)
        .map_err(|e| format!("service '{SERVICE_NAME}' is not installed ({e})"))?;
    service
        .stop()
        .map_err(|e| format!("stopping service failed: {e}"))?;
    println!("service '{SERVICE_NAME}' stop requested");
    Ok(())
}

pub fn status() -> Result<(), String> {
    let mgr = manager(ServiceManagerAccess::CONNECT)?;
    let service = mgr
        .open_service(SERVICE_NAME, ServiceAccess::QUERY_STATUS)
        .map_err(|e| format!("service '{SERVICE_NAME}' is not installed ({e})"))?;
    let st = service
        .query_status()
        .map_err(|e| format!("querying status failed: {e}"))?;
    let state = match st.current_state {
        ServiceState::Stopped => "stopped",
        ServiceState::StartPending => "start pending",
        ServiceState::StopPending => "stop pending",
        ServiceState::Running => "running",
        ServiceState::ContinuePending => "continue pending",
        ServiceState::PausePending => "pause pending",
        ServiceState::Paused => "paused",
    };
    println!("service '{SERVICE_NAME}': {state}");
    Ok(())
}

// ---------------------------------------------------------------------------
// Service runtime (invoked by the Service Control Manager)
// ---------------------------------------------------------------------------

define_windows_service!(ffi_service_main, service_main);

/// Entry point when launched by the Service Control Manager (`zfr service-run`).
pub fn dispatch() -> Result<(), String> {
    service_dispatcher::start(SERVICE_NAME, ffi_service_main).map_err(|e| {
        format!(
            "failed to connect to the Service Control Manager (launch `service-run` via SCM only): {e}"
        )
    })
}

fn service_main(args: Vec<OsString>) {
    // Parse the optional `--config <path>` forwarded in the service command line.
    let mut config_path: Option<PathBuf> = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "--config" {
            if let Some(v) = it.next() {
                config_path = Some(PathBuf::from(v));
            }
        }
    }
    if let Err(e) = run_service(config_path) {
        eprintln!("zfr-core service error: {e:?}");
    }
}

fn run_service(config_path: Option<PathBuf>) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let (shutdown_tx, shutdown_rx) = tokio::sync::mpsc::unbounded_channel();
    let handler = move |control: ServiceControl| match control {
        ServiceControl::Stop | ServiceControl::Shutdown => {
            let _ = shutdown_tx.send(());
            ServiceControlHandlerResult::NoError
        }
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        _ => ServiceControlHandlerResult::NotImplemented,
    };
    let status_handle = service_control_handler::register(SERVICE_NAME, handler)
        .map_err(|e| format!("registering service control handler failed: {e}"))?;

    let set_state = |state: ServiceState, accept: ServiceControlAccept| {
        status_handle
            .set_service_status(ServiceStatus {
                service_type: ServiceType::OWN_PROCESS,
                current_state: state,
                controls_accepted: accept,
                exit_code: ServiceExitCode::Win32(0),
                checkpoint: 0,
                wait_hint: Duration::from_secs(3),
                process_id: None,
            })
            .map_err(|e| format!("setting service status failed: {e}"))
    };

    set_state(ServiceState::StartPending, ServiceControlAccept::empty())?;
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("tokio runtime failed: {e}"))?;
    let result = rt.block_on(run_gateway(config_path, shutdown_rx));
    let _ = set_state(ServiceState::Stopped, ServiceControlAccept::empty());
    result
}

async fn run_gateway(
    config_path: Option<PathBuf>,
    mut shutdown: tokio::sync::mpsc::UnboundedReceiver<()>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Log to a file — the service has no console.
    let log_dir = data_dir();
    std::fs::create_dir_all(&log_dir).map_err(|e| format!("cannot create {}: {e}", log_dir.display()))?;
    let writer = tracing_appender::rolling::never(&log_dir, "zfr-service.log");
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(writer)
        .with_ansi(false)
        .init();

    let (cfg, _) = Config::load(config_path.as_deref())?;
    let egress = cfg
        .egress
        .unwrap_or_else(|| vec!["auto".to_string()]);
    let bind = cfg
        .bind
        .unwrap_or_else(|| "127.0.0.1:18788".parse().expect("valid default bind"));
    let pool = Arc::new(EgressPool::new(&egress));
    let router = Arc::new(Router::new(pool, !cfg.no_failover.unwrap_or(false)));
    let interval = Duration::from_secs(cfg.probe_interval_secs.unwrap_or(300));
    let health = HealthTracker::spawn(Arc::clone(&router), interval);
    let state = AppState {
        router,
        health,
        api_key: cfg.api_key.filter(|s| !s.is_empty()),
        started_at: Instant::now(),
    };
    let app = build_app(state);
    let listener = tokio::net::TcpListener::bind(bind).await?;
    tracing::info!("zfr-core service gateway listening on {bind}");

    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let _ = shutdown.recv().await;
            tracing::info!("service stop requested, shutting down gateway");
        })
        .await?;
    Ok(())
}
