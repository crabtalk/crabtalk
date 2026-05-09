//! Service lifecycle ops for a registry [`Entry`].

use anyhow::{Context, Result, bail};

use crate::registry::Entry;

/// [`command::Service`] adapter for a registry entry whose `label` is `Some`.
/// Non-serviceable entries never reach here.
struct Spec<'a> {
    entry: &'a Entry,
    label: &'a str,
}

impl command::Service for Spec<'_> {
    fn name(&self) -> &str {
        self.entry.short
    }
    fn description(&self) -> &str {
        self.entry.description
    }
    fn label(&self) -> &str {
        self.label
    }
}

impl Entry {
    fn spec(&self) -> Result<Spec<'_>> {
        let label = self
            .label
            .with_context(|| format!("{} is not a service", self.short))?;
        Ok(Spec { entry: self, label })
    }

    fn require_binary(&self) -> Result<std::path::PathBuf> {
        self.binary_path().with_context(|| {
            format!(
                "{} not installed — run `crabup pull {}` first",
                self.krate, self.short
            )
        })
    }

    pub fn start(&self, force: bool) -> Result<()> {
        let spec = self.spec()?;
        if !force && command::is_installed(spec.label) {
            println!("{} is already running", self.short);
            return Ok(());
        }
        if self.short == "daemon" {
            ensure_daemon_configured()?;
        }
        let binary = self.require_binary()?;
        let rendered = command::render_service_template(&spec, &binary);
        command::install(&rendered, spec.label)?;
        println!("started {}", self.short);
        Ok(())
    }

    pub fn stop(&self) -> Result<()> {
        let spec = self.spec()?;
        if !command::is_installed(spec.label) {
            println!("{} is not running", self.short);
            return Ok(());
        }
        command::uninstall(spec.label)?;
        let _ = std::fs::remove_file(wcore::paths::service_port_file(self.short));
        println!("stopped {}", self.short);
        Ok(())
    }

    pub fn restart(&self) -> Result<()> {
        let spec = self.spec()?;
        if self.short == "daemon" {
            ensure_daemon_configured()?;
        }
        if command::is_installed(spec.label) {
            command::uninstall(spec.label)?;
        }
        let binary = self.require_binary()?;
        let rendered = command::render_service_template(&spec, &binary);
        command::install(&rendered, spec.label)?;
        println!("restarted {}", self.short);
        Ok(())
    }

    pub fn logs(&self, tail_args: &[String]) -> Result<()> {
        if self.label.is_none() {
            bail!("{} is not a service", self.short);
        }
        command::view_logs(self.short, tail_args)
    }
}

/// Refuse to start the daemon until an LLM endpoint is configured. The daemon
/// itself only logs a warning at startup; the service unit would happily run
/// with no model list, which is rarely what the user wants. Failing here gives
/// them a single clear next step.
fn ensure_daemon_configured() -> Result<()> {
    let config_path = wcore::paths::CONFIG_DIR.join(wcore::paths::CONFIG_FILE);
    let base_url = std::fs::read_to_string(&config_path)
        .ok()
        .and_then(|raw| raw.parse::<toml::Value>().ok())
        .and_then(|doc| {
            doc.get("llm")
                .and_then(|llm| llm.get("base_url"))
                .and_then(|v| v.as_str())
                .map(str::to_owned)
        })
        .unwrap_or_default();
    if base_url.trim().is_empty() {
        bail!(
            "no LLM endpoint configured — run `crabup login` or edit {}",
            config_path.display()
        );
    }
    Ok(())
}
