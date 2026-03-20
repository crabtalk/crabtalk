//! System service management for the search MCP server — re-exports from wcore.

pub use wcore::service::{ServiceParams, install, render_template, uninstall};

#[cfg(target_os = "macos")]
const LAUNCHD_TEMPLATE: &str = include_str!("launchd.plist");
#[cfg(target_os = "linux")]
const SYSTEMD_TEMPLATE: &str = include_str!("systemd.service");

/// Install the search service using the platform-specific template.
#[cfg(target_os = "macos")]
pub fn install_search(params: &ServiceParams<'_>) -> anyhow::Result<()> {
    install(LAUNCHD_TEMPLATE, params)
}

/// Install the search service using the platform-specific template.
#[cfg(target_os = "linux")]
pub fn install_search(params: &ServiceParams<'_>) -> anyhow::Result<()> {
    install(SYSTEMD_TEMPLATE, params)
}

/// Install the search service using the platform-specific template.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn install_search(params: &ServiceParams<'_>) -> anyhow::Result<()> {
    install("", params)
}
