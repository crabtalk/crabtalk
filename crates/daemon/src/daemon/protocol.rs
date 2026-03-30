//! Server trait implementation for the Daemon.

use crate::{cron::CronEntry, daemon::Daemon};
use anyhow::{Context, Result};
use futures_util::{StreamExt, pin_mut};
use runtime::backend::Backend;
use std::sync::Arc;
use std::{
    collections::VecDeque,
    io::{BufRead, BufReader},
};
use wcore::protocol::{
    api::Server,
    message::{
        AgentEventMsg, AgentInfo, AskOption, AskQuestion, AskUserEvent, CreateAgentMsg,
        CreateCronMsg, CronInfo, CronList, DaemonStats, InstallPackageMsg, PackageInfo,
        ProviderInfo, SendMsg, SendResponse, SessionInfo, StreamChunk, StreamEnd, StreamEvent,
        StreamMsg, StreamStart, StreamThinking, TokenUsage, ToolCallInfo, ToolResultEvent,
        ToolStartEvent, ToolsCompleteEvent, UpdateAgentMsg, stream_event,
    },
};
use wcore::{AgentEvent, AgentStep};

impl<B: Backend + 'static> Server for Daemon<B> {
    async fn send(&self, req: SendMsg) -> Result<SendResponse> {
        let rt: Arc<_> = self.runtime.read().await.clone();
        let sender = req.sender.as_deref().unwrap_or("");
        let created_by = if sender.is_empty() { "user" } else { sender };
        let cwd = req.cwd.map(std::path::PathBuf::from);
        let session_id = match req.session {
            Some(id) => id,
            None => {
                let id = if let Some(ref file) = req.resume_file {
                    rt.load_specific_session(std::path::Path::new(file)).await?
                } else if req.new_chat {
                    rt.create_session(&req.agent, created_by).await?
                } else {
                    rt.get_or_create_session(&req.agent, created_by).await?
                };
                if let Some(ref cwd) = cwd {
                    rt.hook.bridge.set_session_cwd(id, cwd.clone()).await;
                }
                id
            }
        };
        let response = rt.send_to(session_id, &req.content, sender).await?;
        let provider = rt
            .model
            .provider_name_for(&response.model)
            .unwrap_or_default();
        Ok(SendResponse {
            agent: req.agent,
            content: response.final_response.unwrap_or_default(),
            session: session_id,
            provider,
            model: response.model,
            usage: Some(sum_usage(&response.steps)),
        })
    }

    fn stream(
        &self,
        req: StreamMsg,
    ) -> impl futures_core::Stream<Item = Result<StreamEvent>> + Send {
        let runtime = self.runtime.clone();
        let agent = req.agent;
        let content = req.content;
        let req_session = req.session;
        let sender = req.sender.unwrap_or_default();
        let cwd = req.cwd.map(std::path::PathBuf::from);
        let new_chat = req.new_chat;
        let resume_file = req.resume_file;
        async_stream::try_stream! {
            let rt: Arc<_> = runtime.read().await.clone();
            let created_by = if sender.is_empty() { "user".into() } else { sender.clone() };
            let session_id = match req_session {
                Some(id) => id,
                None => {
                    let id = if let Some(ref file) = resume_file {
                        rt.load_specific_session(std::path::Path::new(file)).await?
                    } else if new_chat {
                        rt.create_session(&agent, created_by.as_str()).await?
                    } else {
                        rt.get_or_create_session(&agent, created_by.as_str()).await?
                    };
                    if let Some(ref cwd) = cwd {
                        rt.hook.bridge.set_session_cwd(id, cwd.clone()).await;
                    }
                    id
                }
            };

            yield StreamEvent { event: Some(stream_event::Event::Start(StreamStart { agent: agent.clone(), session: session_id })) };

            let stream = rt.stream_to(session_id, &content, &sender);
            pin_mut!(stream);
            while let Some(event) = stream.next().await {
                match event {
                    AgentEvent::TextDelta(text) => {
                        yield StreamEvent { event: Some(stream_event::Event::Chunk(StreamChunk { content: text })) };
                    }
                    AgentEvent::ThinkingDelta(text) => {
                        yield StreamEvent { event: Some(stream_event::Event::Thinking(StreamThinking { content: text })) };
                    }
                    AgentEvent::ToolCallsBegin(calls) => {
                        yield StreamEvent { event: Some(stream_event::Event::ToolStart(ToolStartEvent {
                            calls: calls.into_iter().map(|c| ToolCallInfo {
                                name: c.function.name.to_string(),
                                arguments: String::new(),
                            }).collect(),
                        })) };
                    }
                    AgentEvent::ToolCallsStart(calls) => {
                        // Extract structured questions from ask_user calls.
                        let ask_questions: Vec<AskQuestion> = calls
                            .iter()
                            .filter(|c| c.function.name == "ask_user")
                            .filter_map(|c| {
                                serde_json::from_str::<runtime::ask_user::AskUser>(&c.function.arguments)
                                    .ok()
                            })
                            .flat_map(|a| a.questions)
                            .map(|q| AskQuestion {
                                question: q.question,
                                header: q.header,
                                options: q.options.into_iter().map(|o| AskOption {
                                    label: o.label,
                                    description: o.description,
                                }).collect(),
                                multi_select: q.multi_select,
                            })
                            .collect();

                        yield StreamEvent { event: Some(stream_event::Event::ToolStart(ToolStartEvent {
                            calls: calls.into_iter().map(|c| ToolCallInfo {
                                name: c.function.name.to_string(),
                                arguments: c.function.arguments,
                            }).collect(),
                        })) };

                        if !ask_questions.is_empty() {
                            yield StreamEvent { event: Some(stream_event::Event::AskUser(AskUserEvent { questions: ask_questions })) };
                        }
                    }
                    AgentEvent::ToolResult { call_id, output, duration_ms } => {
                        yield StreamEvent { event: Some(stream_event::Event::ToolResult(ToolResultEvent { call_id: call_id.to_string(), output, duration_ms })) };
                    }
                    AgentEvent::ToolCallsComplete => {
                        yield StreamEvent { event: Some(stream_event::Event::ToolsComplete(ToolsCompleteEvent {})) };
                    }
                    AgentEvent::Compact { .. } => {
                    }
                    AgentEvent::Done(resp) => {
                        let error = if let wcore::AgentStopReason::Error(ref e) = resp.stop_reason {
                            e.clone()
                        } else {
                            String::new()
                        };
                        let provider = rt
                            .model
                            .provider_name_for(&resp.model)
                            .unwrap_or_default();
                        yield StreamEvent { event: Some(stream_event::Event::End(StreamEnd {
                            agent: agent.clone(),
                            error,
                            provider,
                            model: resp.model,
                            usage: Some(sum_usage(&resp.steps)),
                        })) };
                        return;
                    }
                }
            }
            yield StreamEvent { event: Some(stream_event::Event::End(StreamEnd {
                agent: agent.clone(),
                error: String::new(),
                provider: String::new(),
                model: String::new(),
                usage: None,
            })) };
        }
    }

    async fn compact_session(&self, session: u64) -> Result<String> {
        let rt = self.runtime.read().await.clone();
        rt.compact_session(session)
            .await
            .ok_or_else(|| anyhow::anyhow!("compact failed for session {session}"))
    }

    async fn ping(&self) -> Result<()> {
        Ok(())
    }

    async fn list_sessions(&self) -> Result<Vec<SessionInfo>> {
        let rt = self.runtime.read().await.clone();
        let sessions = rt.sessions().await;
        let mut infos = Vec::with_capacity(sessions.len());
        for s in sessions {
            let s = s.lock().await;
            let active = rt.is_active(s.id).await;
            infos.push(SessionInfo {
                id: s.id,
                agent: s.agent.to_string(),
                created_by: s.created_by.to_string(),
                message_count: s.history.len() as u64,
                alive_secs: s.uptime_secs,
                active,
                title: s.title.clone(),
            });
        }
        Ok(infos)
    }

    async fn kill_session(&self, session: u64) -> Result<bool> {
        let rt = self.runtime.read().await.clone();
        rt.hook.bridge.clear_session_state(session).await;
        Ok(rt.close_session(session).await)
    }

    fn subscribe_events(&self) -> impl futures_core::Stream<Item = Result<AgentEventMsg>> + Send {
        let runtime = self.runtime.clone();
        async_stream::try_stream! {
            let rt = runtime.read().await.clone();
            let Some(mut rx) = rt.hook.bridge.subscribe_events() else {
                return;
            };
            loop {
                match rx.recv().await {
                    Ok(event) => yield event,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                }
            }
        }
    }

    async fn get_config(&self) -> Result<String> {
        let config = self.load_config()?;
        serde_json::to_string(&config).context("failed to serialize config")
    }

    async fn set_config(&self, config: String) -> Result<()> {
        let parsed: crate::DaemonConfig =
            serde_json::from_str(&config).context("invalid DaemonConfig JSON")?;
        let toml_str =
            toml::to_string_pretty(&parsed).context("failed to serialize config to TOML")?;
        let config_path = self.config_dir.join(wcore::paths::CONFIG_FILE);
        std::fs::write(&config_path, toml_str)
            .with_context(|| format!("failed to write {}", config_path.display()))?;
        self.reload().await
    }

    async fn reload(&self) -> Result<()> {
        self.reload().await
    }

    async fn get_stats(&self) -> Result<DaemonStats> {
        let rt = self.runtime.read().await.clone();
        let active = rt.active_session_count().await;
        let agents = rt.agents().len() as u32;
        let uptime = self.started_at.elapsed().as_secs();
        Ok(DaemonStats {
            uptime_secs: uptime,
            active_sessions: active as u32,
            registered_agents: agents,
        })
    }

    async fn create_cron(&self, req: CreateCronMsg) -> Result<CronInfo> {
        // Validate the target session exists.
        let rt = self.runtime.read().await.clone();
        if rt.session(req.session).await.is_none() {
            anyhow::bail!("session {} not found", req.session);
        }
        let entry = CronEntry {
            id: 0, // assigned by store
            schedule: req.schedule,
            skill: req.skill,
            session: req.session,
            quiet_start: req.quiet_start,
            quiet_end: req.quiet_end,
            once: req.once,
        };
        // Schedule validation happens inside CronStore::create.
        let created = self
            .crons
            .lock()
            .await
            .create(entry, self.crons.clone())
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        Ok(cron_entry_to_info(&created))
    }

    async fn delete_cron(&self, id: u64) -> Result<bool> {
        Ok(self.crons.lock().await.delete(id))
    }

    async fn list_crons(&self) -> Result<CronList> {
        let entries = self.crons.lock().await.list();
        Ok(CronList {
            crons: entries.iter().map(cron_entry_to_info).collect(),
        })
    }

    async fn reply_to_ask(&self, session: u64, content: String) -> Result<()> {
        let rt = self.runtime.read().await.clone();
        if rt.hook.bridge.reply_to_ask(session, content).await? {
            return Ok(());
        }
        anyhow::bail!("no pending ask_user for session {session}")
    }

    async fn list_agents(&self) -> Result<Vec<AgentInfo>> {
        let rt = self.runtime.read().await.clone();
        let agents = rt.agents();
        agents
            .into_iter()
            .map(|config| {
                let json =
                    serde_json::to_string(&config).context("failed to serialize agent config")?;
                Ok(AgentInfo {
                    name: config.name,
                    description: config.description,
                    config: json,
                })
            })
            .collect()
    }

    async fn get_agent(&self, name: String) -> Result<AgentInfo> {
        let rt = self.runtime.read().await.clone();
        let config = rt
            .agent(&name)
            .ok_or_else(|| anyhow::anyhow!("agent '{name}' not found"))?;
        let json = serde_json::to_string(&config).context("failed to serialize agent config")?;
        Ok(AgentInfo {
            name: config.name,
            description: config.description,
            config: json,
        })
    }

    async fn create_agent(&self, req: CreateAgentMsg) -> Result<AgentInfo> {
        self.write_agent_to_manifest(&req.name, &req.config, true)?;
        self.reload().await?;
        self.get_agent(req.name).await
    }

    async fn update_agent(&self, req: UpdateAgentMsg) -> Result<AgentInfo> {
        self.write_agent_to_manifest(&req.name, &req.config, false)?;
        self.reload().await?;
        self.get_agent(req.name).await
    }

    async fn delete_agent(&self, name: String) -> Result<bool> {
        use toml_edit::DocumentMut;

        let manifest_path = self
            .config_dir
            .join(wcore::paths::LOCAL_DIR)
            .join("CrabTalk.toml");
        if !manifest_path.exists() {
            return Ok(false);
        }
        let content =
            std::fs::read_to_string(&manifest_path).context("failed to read local manifest")?;
        let mut doc: DocumentMut = content.parse().context("failed to parse local manifest")?;

        let removed = doc
            .get_mut("agents")
            .and_then(|v| v.as_table_like_mut())
            .and_then(|t| t.remove(&name))
            .is_some();
        if removed {
            std::fs::write(&manifest_path, doc.to_string())
                .context("failed to write local manifest")?;
            self.reload().await?;
        }
        Ok(removed)
    }

    async fn list_providers(&self) -> Result<Vec<ProviderInfo>> {
        let rt = self.runtime.read().await.clone();
        let entries = rt.model.list()?;
        Ok(entries
            .into_iter()
            .map(|e| ProviderInfo {
                name: e.name,
                active: e.active,
            })
            .collect())
    }

    async fn install_package(&self, req: InstallPackageMsg) -> Result<()> {
        let branch = if req.branch.is_empty() {
            None
        } else {
            Some(req.branch.as_str())
        };
        let path = if req.path.is_empty() {
            None
        } else {
            Some(std::path::Path::new(&req.path))
        };
        crabhub::package::install(&req.package, branch, path, req.force, |msg| {
            tracing::info!("{msg}");
        })
        .await?;
        self.reload().await
    }

    async fn uninstall_package(&self, package: String) -> Result<()> {
        crabhub::package::uninstall(&package, |msg| {
            tracing::info!("{msg}");
        })
        .await?;
        self.reload().await
    }

    async fn list_packages(&self) -> Result<Vec<PackageInfo>> {
        let mut result: Vec<PackageInfo> = scan_package_manifests(&self.config_dir)
            .into_iter()
            .map(|(name, manifest)| PackageInfo {
                name,
                description: manifest.package.description,
            })
            .collect();
        result.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(result)
    }

    async fn start_service(&self, name: String, force: bool) -> Result<()> {
        let cmd = self.find_command_service(&name)?;
        let label = format!("ai.crabtalk.{name}");
        if !force && crabtalk_command::service::is_installed(&label) {
            anyhow::bail!("service '{name}' is already running, use force to restart");
        }
        let binary = find_binary(&cmd.krate)?;
        let rendered = crabtalk_command::service::render_service_template(
            &CommandService {
                name: name.clone(),
                description: cmd.description.clone(),
                label: label.clone(),
            },
            &binary,
        );
        crabtalk_command::service::install(&rendered, &label)
    }

    async fn stop_service(&self, name: String) -> Result<()> {
        let label = format!("ai.crabtalk.{name}");
        crabtalk_command::service::uninstall(&label)?;
        let _ = std::fs::remove_file(wcore::paths::service_port_file(&name));
        Ok(())
    }

    async fn service_logs(&self, name: String, lines: u32) -> Result<String> {
        let path = wcore::paths::service_log_path(&name);
        if !path.exists() {
            return Ok(format!("no logs yet: {}", path.display()));
        }
        let file =
            std::fs::File::open(&path).context(format!("failed to open {}", path.display()))?;
        let n = if lines == 0 { 50 } else { lines as usize };
        let mut tail: VecDeque<String> = VecDeque::with_capacity(n);
        for line in BufReader::new(file).lines() {
            let line = line?;
            if tail.len() == n {
                tail.pop_front();
            }
            tail.push_back(line);
        }
        Ok(tail.into_iter().collect::<Vec<_>>().join("\n"))
    }
}

/// Service metadata for render_service_template.
struct CommandService {
    name: String,
    description: String,
    label: String,
}

impl crabtalk_command::service::Service for CommandService {
    fn name(&self) -> &str {
        &self.name
    }
    fn description(&self) -> &str {
        &self.description
    }
    fn label(&self) -> &str {
        &self.label
    }
}

/// Scan `packages/` for installed hub manifests, returning `(scope/name, Manifest)` pairs.
fn scan_package_manifests(
    config_dir: &std::path::Path,
) -> Vec<(String, crabhub::manifest::Manifest)> {
    let packages_dir = config_dir.join(wcore::paths::PACKAGES_DIR);
    let mut result = Vec::new();
    let scopes = match std::fs::read_dir(&packages_dir) {
        Ok(entries) => entries,
        Err(_) => return result,
    };
    for scope_entry in scopes.flatten() {
        let scope_path = scope_entry.path();
        if !scope_path.is_dir() {
            continue;
        }
        let scope = scope_entry.file_name().to_string_lossy().to_string();
        let manifests = match std::fs::read_dir(&scope_path) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for manifest_entry in manifests.flatten() {
            let path = manifest_entry.path();
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
            match toml::from_str::<crabhub::manifest::Manifest>(&content) {
                Ok(manifest) => result.push((format!("{scope}/{name}"), manifest)),
                Err(e) => {
                    tracing::warn!("failed to parse manifest {}: {e}", path.display());
                }
            }
        }
    }
    result
}

/// Find a binary on PATH or in ~/.cargo/bin.
fn find_binary(name: &str) -> Result<std::path::PathBuf> {
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let candidate = dir.join(name);
            if candidate.exists() {
                return Ok(candidate);
            }
        }
    }
    let cargo_bin = wcore::paths::CONFIG_DIR
        .parent()
        .unwrap_or(std::path::Path::new("/"))
        .join(".cargo/bin")
        .join(name);
    if cargo_bin.exists() {
        return Ok(cargo_bin);
    }
    anyhow::bail!("binary '{name}' not found in PATH or ~/.cargo/bin")
}

impl<B: Backend + 'static> Daemon<B> {
    /// Load the current `DaemonConfig` from disk.
    fn load_config(&self) -> Result<crate::DaemonConfig> {
        crate::DaemonConfig::load(&self.config_dir.join(wcore::paths::CONFIG_FILE))
    }

    /// Look up a command service by name from installed package manifests.
    fn find_command_service(&self, name: &str) -> Result<crabhub::manifest::CommandConfig> {
        for (_, manifest) in scan_package_manifests(&self.config_dir) {
            if let Some(cmd) = manifest.commands.get(name) {
                return Ok(cmd.clone());
            }
        }
        anyhow::bail!("command service '{name}' not found in installed packages")
    }

    /// Write an agent config into the local manifest `[agents.<name>]`.
    ///
    /// If `expect_new` is true, fails when the agent already exists in the
    /// manifest. If false, fails when it does not exist. The check and write
    /// happen in the same synchronous block with no yield points.
    fn write_agent_to_manifest(
        &self,
        name: &str,
        config_json: &str,
        expect_new: bool,
    ) -> Result<()> {
        use toml_edit::DocumentMut;

        // Parse incoming JSON to validate it and convert to TOML value.
        let config: wcore::AgentConfig =
            serde_json::from_str(config_json).context("invalid AgentConfig JSON")?;
        let toml_value = toml::to_string(&config).context("failed to serialize agent to TOML")?;
        let agent_doc: DocumentMut = toml_value.parse().context("failed to parse agent TOML")?;

        let manifest_path = self
            .config_dir
            .join(wcore::paths::LOCAL_DIR)
            .join("CrabTalk.toml");
        let mut doc: DocumentMut = if manifest_path.exists() {
            std::fs::read_to_string(&manifest_path)
                .context("failed to read local manifest")?
                .parse()
                .context("failed to parse local manifest")?
        } else {
            DocumentMut::default()
        };

        // Ensure [agents] table exists.
        if doc.get("agents").is_none() {
            doc.insert("agents", toml_edit::Item::Table(toml_edit::Table::new()));
        }
        let agents = doc["agents"]
            .as_table_mut()
            .context("[agents] is not a table")?;

        let exists = agents.contains_key(name);
        if expect_new && exists {
            anyhow::bail!("agent '{name}' already exists in local manifest");
        }
        if !expect_new && !exists {
            anyhow::bail!("agent '{name}' not found in local manifest");
        }

        // Insert the agent as a sub-table.
        let mut agent_table = toml_edit::Table::new();
        for (key, value) in agent_doc.as_table().iter() {
            agent_table.insert(key, value.clone());
        }
        agents.insert(name, toml_edit::Item::Table(agent_table));

        std::fs::create_dir_all(manifest_path.parent().context("no parent dir")?)
            .context("failed to create local dir")?;
        std::fs::write(&manifest_path, doc.to_string())
            .context("failed to write local manifest")?;
        Ok(())
    }
}

fn cron_entry_to_info(e: &CronEntry) -> CronInfo {
    CronInfo {
        id: e.id,
        schedule: e.schedule.clone(),
        skill: e.skill.clone(),
        session: e.session,
        quiet_start: e.quiet_start.clone().unwrap_or_default(),
        quiet_end: e.quiet_end.clone().unwrap_or_default(),
        once: e.once,
    }
}

fn sum_usage(steps: &[AgentStep]) -> TokenUsage {
    let mut prompt = 0u32;
    let mut completion = 0u32;
    let mut total = 0u32;
    let mut cache_hit = 0u32;
    let mut cache_miss = 0u32;
    let mut reasoning = 0u32;
    let mut has_cache_hit = false;
    let mut has_cache_miss = false;
    let mut has_reasoning = false;

    for step in steps {
        let u = &step.response.usage;
        prompt += u.prompt_tokens;
        completion += u.completion_tokens;
        total += u.total_tokens;
        if let Some(v) = u.prompt_cache_hit_tokens {
            cache_hit += v;
            has_cache_hit = true;
        }
        if let Some(v) = u.prompt_cache_miss_tokens {
            cache_miss += v;
            has_cache_miss = true;
        }
        if let Some(ref d) = u.completion_tokens_details
            && let Some(v) = d.reasoning_tokens
        {
            reasoning += v;
            has_reasoning = true;
        }
    }

    TokenUsage {
        prompt_tokens: prompt,
        completion_tokens: completion,
        total_tokens: total,
        cache_hit_tokens: has_cache_hit.then_some(cache_hit),
        cache_miss_tokens: has_cache_miss.then_some(cache_miss),
        reasoning_tokens: has_reasoning.then_some(reasoning),
    }
}
