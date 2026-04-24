//! Fallback for unsupported platforms.

use anyhow::Result;

pub fn is_installed(_label: &str) -> bool {
    false
}

pub fn install(_rendered: &str, _label: &str) -> Result<()> {
    anyhow::bail!("service install is only supported on macOS, Linux, and Windows")
}

pub fn uninstall(_label: &str) -> Result<()> {
    anyhow::bail!("service uninstall is only supported on macOS, Linux, and Windows")
}
