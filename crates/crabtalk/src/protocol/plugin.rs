//! Daemon-level plugin lifecycle: install, uninstall, list.

use crate::daemon::Daemon;
use anyhow::{Context, Result};
use crabllm_core::Provider;
use wcore::protocol::message::*;
use wcore::storage::Storage;

impl<P: Provider + 'static> Daemon<P> {
    pub(crate) fn install_plugin<'a>(
        &'a self,
        req: InstallPluginMsg,
    ) -> impl futures_core::Stream<Item = Result<PluginEvent>> + Send + 'a {
        async_stream::try_stream! {
            let plugin = req.plugin;
            let branch = req.branch;
            let path = req.path;
            let force = req.force;

            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(bool, String)>();
            let handle = tokio::spawn({
                let branch = branch.clone();
                let path = path.clone();
                let plugin = plugin.clone();
                let tx2 = tx.clone();
                async move {
                    let branch = if branch.is_empty() { None } else { Some(branch.as_str()) };
                    let path = if path.is_empty() { None } else { Some(std::path::Path::new(&path)) };
                    plugin::plugin::install(
                        &plugin, branch, path, force,
                        |msg| { let _ = tx.send((false, msg.to_string())); },
                        |msg| { let _ = tx2.send((true, msg.to_string())); },
                    )
                    .await
                }
            });

            tokio::pin!(handle);
            let task_result;
            loop {
                tokio::select! {
                    msg = rx.recv() => {
                        match msg {
                            Some((is_output, m)) => {
                                if is_output {
                                    yield plugin_output(&m);
                                } else {
                                    yield plugin_step(&m);
                                }
                            }
                            None => {
                                task_result = handle.await;
                                break;
                            }
                        }
                    }
                    result = &mut handle => {
                        rx.close();
                        while let Some((is_output, m)) = rx.recv().await {
                            if is_output {
                                yield plugin_output(&m);
                            } else {
                                yield plugin_step(&m);
                            }
                        }
                        task_result = result;
                        break;
                    }
                }
            }
            task_result.context("install task panicked")??;

            yield plugin_step("reloading daemon…");
            self.reload().await?;

            let dirs = wcore::resolve_dirs(&self.config_dir);
            let warnings = wcore::check_skill_conflicts(&dirs.skill_dirs);
            for w in &warnings {
                yield plugin_warning(w);
            }
            let storage_mcps = {
                let rt = self.runtime.read().await.clone();
                rt.storage().list_mcps().await?
            };
            for (name, mcp) in &storage_mcps {
                if mcp.auth
                    && !wcore::paths::TOKENS_DIR.join(format!("{name}.json")).exists()
                {
                    yield plugin_warning(&format!("MCP '{name}' requires authentication"));
                }
            }

            yield plugin_step("configure env vars in config.toml [env] section if needed");
            yield plugin_done("");
        }
    }

    pub(crate) fn uninstall_plugin<'a>(
        &'a self,
        plugin: String,
    ) -> impl futures_core::Stream<Item = Result<PluginEvent>> + Send + 'a {
        async_stream::try_stream! {
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
            let name = plugin.clone();
            let handle = tokio::spawn(async move {
                plugin::plugin::uninstall(&name, |msg| {
                    let _ = tx.send(msg.to_string());
                })
                .await
            });

            tokio::pin!(handle);
            let task_result;
            loop {
                tokio::select! {
                    msg = rx.recv() => {
                        match msg {
                            Some(m) => yield plugin_step(&m),
                            None => {
                                task_result = handle.await;
                                break;
                            }
                        }
                    }
                    result = &mut handle => {
                        rx.close();
                        while let Some(m) = rx.recv().await {
                            yield plugin_step(&m);
                        }
                        task_result = result;
                        break;
                    }
                }
            }
            task_result.context("uninstall task panicked")??;

            yield plugin_step("reloading daemon…");
            self.reload().await?;
            yield plugin_done("");
        }
    }

    pub(crate) fn list_plugins(&self) -> Vec<PluginInfo> {
        let mut result: Vec<PluginInfo> = scan_plugin_manifests(&self.config_dir)
            .into_iter()
            .map(|(name, manifest)| PluginInfo {
                name,
                description: manifest.package.description,
                installed: true,
                ..Default::default()
            })
            .collect();
        result.sort_by(|a, b| a.name.cmp(&b.name));
        result
    }
}

pub(super) async fn search(query: &str) -> Result<Vec<PluginInfo>> {
    let entries = plugin::plugin::search(query).await?;
    Ok(entries
        .into_iter()
        .map(|e| PluginInfo {
            name: e.name,
            description: e.description,
            skill_count: e.skill_count,
            mcp_count: e.mcp_count,
            installed: e.installed,
            repository: e.repository,
        })
        .collect())
}

pub(super) fn scan_plugin_manifests(
    config_dir: &std::path::Path,
) -> Vec<(String, plugin::manifest::Manifest)> {
    let plugins_dir = config_dir.join(wcore::paths::PLUGINS_DIR);
    let mut result = Vec::new();
    let entries = match std::fs::read_dir(&plugins_dir) {
        Ok(entries) => entries,
        Err(_) => return result,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("toml") {
            continue;
        }
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        match toml::from_str::<plugin::manifest::Manifest>(&content) {
            Ok(manifest) => result.push((name.to_string(), manifest)),
            Err(e) => {
                tracing::warn!("failed to parse manifest {}: {e}", path.display());
            }
        }
    }
    result
}

fn plugin_step(message: &str) -> PluginEvent {
    PluginEvent {
        event: Some(plugin_event::Event::Step(PluginStep {
            message: message.to_string(),
        })),
    }
}

fn plugin_warning(message: &str) -> PluginEvent {
    PluginEvent {
        event: Some(plugin_event::Event::Warning(PluginWarning {
            message: message.to_string(),
        })),
    }
}

fn plugin_done(error: &str) -> PluginEvent {
    PluginEvent {
        event: Some(plugin_event::Event::Done(PluginDone {
            error: error.to_string(),
        })),
    }
}

fn plugin_output(content: &str) -> PluginEvent {
    PluginEvent {
        event: Some(plugin_event::Event::SetupOutput(PluginSetupOutput {
            content: content.to_string(),
        })),
    }
}
