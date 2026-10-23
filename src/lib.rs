//! ZFR Core — High-Performance Model Router & Transport Gateway.

pub mod catalog;
pub mod config;
pub mod daemon;
pub mod egress;
pub mod fingerprint;
pub mod health;
pub mod router;
#[cfg(windows)]
pub mod service;
pub mod server;
