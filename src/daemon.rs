//! Foreground-free background execution (daemon mode) for `zfr serve`.
//!
//! `zfr serve --daemon` re-launches itself as a detached process (no console
//! window on Windows), redirects logs to `%LOCALAPPDATA%\zfr\zfr.log` and
//! records the PID in `zfr.pid` so `zfr stop` can terminate it later.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::{Command, Stdio};

/// Everything needed to re-launch `zfr serve` in the background.
pub struct ServeOptions {
    pub bind: SocketAddr,
    pub egress: Vec<String>,
    pub no_failover: bool,
    pub api_key: Option<String>,
    pub probe_interval: u64,
    pub config: Option<PathBuf>,
}

/// Per-user data directory for PID files, logs and user config.
pub fn data_dir() -> PathBuf {
    #[cfg(windows)]
    {
        std::env::var("LOCALAPPDATA")
            .map(|d| PathBuf::from(d).join("zfr"))
            .unwrap_or_else(|_| std::env::temp_dir().join("zfr"))
    }
    #[cfg(not(windows))]
    {
        std::env::var("HOME")
            .map(|h| PathBuf::from(h).join(".local/share/zfr"))
            .unwrap_or_else(|_| std::env::temp_dir().join("zfr"))
    }
}

pub fn pid_file() -> PathBuf {
    data_dir().join("zfr.pid")
}

pub fn log_file() -> PathBuf {
    data_dir().join("zfr.log")
}

fn read_pid() -> Option<u32> {
    std::fs::read_to_string(pid_file())
        .ok()?
        .trim()
        .parse()
        .ok()
}

/// Re-launch `zfr serve ...` as a detached background process and record its PID.
pub fn spawn_detached(opts: &ServeOptions) -> Result<u32, String> {
    let exe = std::env::current_exe().map_err(|e| format!("cannot locate executable: {e}"))?;
    let mut cmd = Command::new(&exe);
    cmd.arg("serve").arg("--bind").arg(opts.bind.to_string());
    for e in &opts.egress {
        cmd.arg("--egress").arg(e);
    }
    if opts.no_failover {
        cmd.arg("--no-failover");
    }
    if let Some(k) = &opts.api_key {
        cmd.arg("--api-key").arg(k);
    }
    cmd.arg("--probe-interval").arg(opts.probe_interval.to_string());
    if let Some(c) = &opts.config {
        cmd.arg("--config").arg(c);
    }

    std::fs::create_dir_all(data_dir())
        .map_err(|e| format!("cannot create {}: {e}", data_dir().display()))?;
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_file())
        .map_err(|e| format!("cannot open log file: {e}"))?;
    let log_err = log
        .try_clone()
        .map_err(|e| format!("cannot duplicate log handle: {e}"))?;
    cmd.stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(log_err));

    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Foundation::{SetHandleInformation, HANDLE_FLAG_INHERIT};

        // The detached child must not inherit the caller's stdio pipe handles,
        // otherwise a capturing parent (shell pipeline, CI runner) waits on the
        // pipe forever because the long-lived child keeps it open.
        let stdin = std::io::stdin();
        let stdout = std::io::stdout();
        let stderr = std::io::stderr();
        unsafe {
            for h in [
                stdin.as_raw_handle(),
                stdout.as_raw_handle(),
                stderr.as_raw_handle(),
            ] {
                if !h.is_null() && h as usize != usize::MAX {
                    SetHandleInformation(h, HANDLE_FLAG_INHERIT, 0);
                }
            }
        }

        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }

    let child = cmd
        .spawn()
        .map_err(|e| format!("failed to spawn background process: {e}"))?;
    let pid = child.id();
    std::fs::write(pid_file(), pid.to_string())
        .map_err(|e| format!("cannot write pid file: {e}"))?;
    Ok(pid)
}

/// Stop a previously spawned background instance via its PID file.
pub fn stop() -> Result<(), String> {
    let pid =
        read_pid().ok_or_else(|| "no PID file found — is the background instance running?".to_string())?;
    #[cfg(windows)]
    let success = Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/F"])
        .status()
        .map_err(|e| format!("failed to run taskkill: {e}"))?
        .success();
    #[cfg(not(windows))]
    let success = Command::new("kill")
        .arg(pid.to_string())
        .status()
        .map_err(|e| format!("failed to run kill: {e}"))?
        .success();

    if success {
        let _ = std::fs::remove_file(pid_file());
        println!("stopped background zfr (pid {pid})");
        Ok(())
    } else {
        Err(format!(
            "failed to terminate pid {pid} (process may already be gone)"
        ))
    }
}
