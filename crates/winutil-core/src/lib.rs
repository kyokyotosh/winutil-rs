//! Core logic for WinUtil.
//!
//! This crate has no GUI dependencies so that its logic can be tested without a window.

pub mod config;
pub mod install;
pub mod net;
pub mod tweaks;

/// Display name of the application.
pub const APP_NAME: &str = "WinUtil";
