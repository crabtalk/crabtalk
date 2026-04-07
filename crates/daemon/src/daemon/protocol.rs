//! Server trait implementation for the Daemon.

use crate::{cron::CronEntry, daemon::Daemon, event_bus::EventSubscription};
use anyhow::{Context, Result};
use crabllm_core::Provider;
use futures_util::{StreamExt, pin_mut};
use runtime::host::Host;
use std::sync::Arc;
use std::{
    collections::VecDeque,
    io::{BufRead, BufReader},
};
use wcore::protocol::{
    api::Server,
    message::{
        ActiveConversationInfo, AgentEventMsg, AgentInfo, AskOption, AskQuestion, AskUserEvent,
        ConversationHistory, ConversationInfo, ConversationMessage, CreateAgentMsg, CreateCronMsg,
        CronInfo, CronList, DaemonStats, InstallPluginMsg, McpInfo, McpStatus, ModelInfo,
        PluginDone, PluginEvent, PluginInfo, PluginSetupOutput, PluginStep, PluginWarning,
        ProtoProviderKind, ProviderInfo, ProviderPresetInfo, PublishEventMsg, ResourceKind,
        SendMsg, SendResponse, SkillInfo, SourceKind, SteerSessionMsg, StreamChunk, StreamEnd,
        StreamEvent, StreamMsg, StreamStart, StreamThinking, SubscribeEventMsg, SubscriptionInfo,
        SubscriptionList, TextEndEvent, TextStartEvent, ThinkingEndEvent, ThinkingStartEvent,
        TokenUsage, ToolCallInfo, ToolResultEvent, ToolStartEvent, ToolsCompleteEvent,
        UpdateAgentMsg, UserSteeredEvent, plugin_event, stream_event,
    },
};
use wcore::{AgentEvent, AgentStep};

impl<P: Provider + 'static, H: Host + 'static> Server for Daemon<P, H> {
    async fn send(&self, req: SendMsg) -> Result<SendResponse> {
        let rt: Arc<_> = self.runtime.read().await.clone();
        let sender = req.sender.as_deref().unwrap_or("");
        let created_by = if sender.is_empty() { "user" } else { sender };
        let cwd = req.cwd.map(std::path::PathBuf::from);
        let conversation_id = rt
            .get_or_create_conversation(&req.agent, created_by)
            .await?;
        if let Some(ref cwd) = cwd {
            rt.hook
                .host
                .set_conversation_cwd(conversation_id, cwd.clone())
                .await;
        }
        let tool_choice = req
            .tool_choice
            .map(|s| wcore::model::ToolChoice::from(s.as_str()));
        let response = rt
            .send_to(conversation_id, &req.content, sender, tool_choice)
            .await?;
        let provider = self.provider_name_for_model(&response.model);
        Ok(SendResponse {
            agent: req.agent,
            content: response.final_response.unwrap_or_default(),
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
        let sender = req.sender.unwrap_or_default();
        let cwd = req.cwd.map(std::path::PathBuf::from);
        let guest = req.guest.unwrap_or_default();
        let tool_choice = req
            .tool_choice
            .map(|s| wcore::model::ToolChoice::from(s.as_str()));
        async_stream::try_stream! {
            let rt: Arc<_> = runtime.read().await.clone();
            let created_by = if sender.is_empty() { "user".into() } else { sender.clone() };
            let conversation_id = rt.get_or_create_conversation(&agent, created_by.as_str()).await?;
            if let Some(ref cwd) = cwd {
                rt.hook.host.set_conversation_cwd(conversation_id, cwd.clone()).await;
            }

            let responding_agent = if guest.is_empty() { agent.clone() } else { guest.clone() };
            yield StreamEvent { event: Some(stream_event::Event::Start(StreamStart { agent: responding_agent.clone() })) };

            let stream: std::pin::Pin<Box<dyn futures_core::Stream<Item = wcore::AgentEvent> + Send + '_>> = if guest.is_empty() {
                Box::pin(rt.stream_to(conversation_id, &content, &sender, tool_choice))
            } else {
                Box::pin(rt.guest_stream_to(conversation_id, &content, &sender, &guest))
            };
            pin_mut!(stream);
            while let Some(event) = stream.next().await {
                match event {
                    AgentEvent::TextStart => {
                        yield StreamEvent { event: Some(stream_event::Event::TextStart(TextStartEvent { agent: responding_agent.clone() })) };
                    }
                    AgentEvent::TextDelta(text) => {
                        yield StreamEvent { event: Some(stream_event::Event::Chunk(StreamChunk { content: text })) };
                    }
                    AgentEvent::TextEnd => {
                        yield StreamEvent { event: Some(stream_event::Event::TextEnd(TextEndEvent { agent: responding_agent.clone() })) };
                    }
                    AgentEvent::ThinkingStart => {
                        yield StreamEvent { event: Some(stream_event::Event::ThinkingStart(ThinkingStartEvent { agent: responding_agent.clone() })) };
                    }
                    AgentEvent::ThinkingDelta(text) => {
                        yield StreamEvent { event: Some(stream_event::Event::Thinking(StreamThinking { content: text })) };
                    }
                    AgentEvent::ThinkingEnd => {
                        yield StreamEvent { event: Some(stream_event::Event::ThinkingEnd(ThinkingEndEvent { agent: responding_agent.clone() })) };
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
                    AgentEvent::Compact { .. } => {}
                    AgentEvent::UserSteered { ref content } => {
                        yield StreamEvent { event: Some(stream_event::Event::UserSteered(UserSteeredEvent { content: content.clone() })) };
                    }
                    AgentEvent::Done(resp) => {
                        let error = if let wcore::AgentStopReason::Error(ref e) = resp.stop_reason {
                            e.clone()
                        } else {
                            String::new()
                        };
                        let provider = self.provider_name_for_model(&resp.model);
                        yield StreamEvent { event: Some(stream_event::Event::End(StreamEnd {
                            agent: responding_agent.clone(),
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
                agent: responding_agent.clone(),
                error: String::new(),
                provider: String::new(),
                model: String::new(),
                usage: None,
            })) };
        }
    }

    async fn compact_conversation(&self, agent: String, sender: String) -> Result<String> {
        let rt = self.runtime.read().await.clone();
        let conversation_id = rt
            .find_conversation_id(&agent, &sender)
            .await
            .ok_or_else(|| {
                anyhow::anyhow!("conversation not found for agent='{agent}' sender='{sender}'")
            })?;
        rt.compact_conversation(conversation_id)
            .await
            .ok_or_else(|| anyhow::anyhow!("compact failed for agent='{agent}' sender='{sender}'"))
    }

    async fn ping(&self) -> Result<()> {
        Ok(())
    }

    async fn list_conversations_active(&self) -> Result<Vec<ActiveConversationInfo>> {
        let rt = self.runtime.read().await.clone();
        let conversations = rt.conversations().await;
        let mut infos = Vec::with_capacity(conversations.len());
        for c in conversations {
            let c = c.lock().await;
            infos.push(ActiveConversationInfo {
                agent: c.agent.to_string(),
                sender: c.created_by.to_string(),
                message_count: c.history.len() as u64,
                alive_secs: c.uptime_secs,
                title: c.title.clone(),
            });
        }
        Ok(infos)
    }

    async fn kill_conversation(&self, agent: String, sender: String) -> Result<bool> {
        let rt = self.runtime.read().await.clone();
        let Some(conversation_id) = rt.find_conversation_id(&agent, &sender).await else {
            return Ok(false);
        };
        rt.hook.host.clear_conversation_state(conversation_id).await;
        Ok(rt.close_conversation(conversation_id).await)
    }

    fn subscribe_events(&self) -> impl futures_core::Stream<Item = Result<AgentEventMsg>> + Send {
        let runtime = self.runtime.clone();
        async_stream::try_stream! {
            let rt = runtime.read().await.clone();
            let Some(mut rx) = rt.hook.host.subscribe_events() else {
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

    async fn reload(&self) -> Result<()> {
        self.reload().await
    }

    async fn get_stats(&self) -> Result<DaemonStats> {
        let rt = self.runtime.read().await.clone();
        let active = rt.conversation_count().await;
        let agents = rt.agents().len() as u32;
        let uptime = self.started_at.elapsed().as_secs();
        let active_model = self
            .load_config()
            .ok()
            .and_then(|c| c.system.crab.model)
            .unwrap_or_default();
        Ok(DaemonStats {
            uptime_secs: uptime,
            active_conversations: active as u32,
            registered_agents: agents,
            active_model,
        })
    }

    async fn create_cron(&self, req: CreateCronMsg) -> Result<CronInfo> {
        // Validate the target agent exists.
        let rt = self.runtime.read().await.clone();
        if rt.agent(&req.agent).is_none() {
            anyhow::bail!("agent '{}' not found", req.agent);
        }
        let entry = CronEntry {
            id: 0, // assigned by store
            schedule: req.schedule,
            skill: req.skill,
            agent: req.agent,
            sender: req.sender,
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

    async fn subscribe_event(&self, req: SubscribeEventMsg) -> Result<SubscriptionInfo> {
        let rt = self.runtime.read().await.clone();
        if rt.agent(&req.target_agent).is_none() {
            anyhow::bail!("agent '{}' not found", req.target_agent);
        }
        let sub = EventSubscription {
            id: 0,
            source: req.source,
            target_agent: req.target_agent,
            once: req.once,
        };
        let created = self.events.lock().await.subscribe(sub);
        Ok(subscription_to_info(&created))
    }

    async fn unsubscribe_event(&self, id: u64) -> Result<bool> {
        Ok(self.events.lock().await.unsubscribe(id))
    }

    async fn list_subscriptions(&self) -> Result<SubscriptionList> {
        let subs = self.events.lock().await.list();
        Ok(SubscriptionList {
            subscriptions: subs.iter().map(subscription_to_info).collect(),
        })
    }

    async fn publish_event(&self, req: PublishEventMsg) -> Result<()> {
        let _ = self.event_tx.send(crate::DaemonEvent::PublishEvent {
            source: req.source,
            payload: req.payload,
        });
        Ok(())
    }

    async fn reply_to_ask(&self, agent: String, sender: String, content: String) -> Result<()> {
        let rt = self.runtime.read().await.clone();
        let conversation_id = rt
            .find_conversation_id(&agent, &sender)
            .await
            .ok_or_else(|| {
                anyhow::anyhow!("conversation not found for agent='{agent}' sender='{sender}'")
            })?;
        if rt.hook.host.reply_to_ask(conversation_id, content).await? {
            return Ok(());
        }
        anyhow::bail!("no pending ask_user for agent='{agent}' sender='{sender}'")
    }

    async fn steer_session(&self, req: SteerSessionMsg) -> Result<()> {
        let rt = self.runtime.read().await.clone();
        let sender = if req.sender.is_empty() {
            "user"
        } else {
            &req.sender
        };
        let conversation_id = rt
            .find_conversation_id(&req.agent, sender)
            .await
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "conversation not found for agent='{}' sender='{sender}'",
                    req.agent
                )
            })?;
        rt.steer(conversation_id, req.content).await
    }

    async fn list_agents(&self) -> Result<Vec<AgentInfo>> {
        let rt = self.runtime.read().await.clone();
        Ok(rt
            .agents()
            .into_iter()
            .map(|c| agent_config_to_info(&c))
            .collect())
    }

    async fn get_agent(&self, name: String) -> Result<AgentInfo> {
        let rt = self.runtime.read().await.clone();
        let config = rt
            .agent(&name)
            .ok_or_else(|| anyhow::anyhow!("agent '{name}' not found"))?;
        Ok(agent_config_to_info(&config))
    }

    async fn create_agent(&self, req: CreateAgentMsg) -> Result<AgentInfo> {
        validate_agent_name(&req.name)?;
        self.write_agent_to_manifest(&req.name, &req.config, true)?;
        self.write_agent_prompt(&req.name, &req.prompt)?;
        self.reload().await?;
        self.get_agent(req.name).await
    }

    async fn update_agent(&self, req: UpdateAgentMsg) -> Result<AgentInfo> {
        validate_agent_name(&req.name)?;
        if req.name == wcore::paths::DEFAULT_AGENT {
            self.write_system_crab_config(&req.config)?;
        } else {
            self.write_agent_to_manifest(&req.name, &req.config, false)?;
        }
        if !req.prompt.is_empty() {
            self.write_agent_prompt(&req.name, &req.prompt)?;
        }
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
            let prompt_file = self
                .config_dir
                .join(wcore::paths::AGENTS_DIR)
                .join(format!("{name}.md"));
            if prompt_file.exists() {
                std::fs::remove_file(&prompt_file).context("failed to remove agent prompt file")?;
            }
            self.reload().await?;
        }
        Ok(removed)
    }

    async fn list_providers(&self) -> Result<Vec<ProviderInfo>> {
        let config = self.load_config()?;
        let (manifest, _) = self.resolve_manifests()?;
        let active_model = config.system.crab.model.clone().unwrap_or_default();
        Ok(config
            .provider
            .iter()
            .map(|(name, def)| {
                let cfg_json = serde_json::to_string(def).unwrap_or_default();
                let active = !active_model.is_empty() && def.models.contains(&active_model);
                let enabled = !manifest.disabled.providers.contains(name);
                ProviderInfo {
                    name: name.clone(),
                    active,
                    config: cfg_json,
                    enabled,
                }
            })
            .collect())
    }

    fn install_plugin(
        &self,
        req: InstallPluginMsg,
    ) -> impl futures_core::Stream<Item = Result<PluginEvent>> + Send {
        let daemon = self.clone();
        async_stream::try_stream! {
            let plugin = req.plugin;
            let branch = req.branch;
            let path = req.path;
            let force = req.force;

            // Channel bridge: sync callbacks → async stream.
            // false = step message, true = script output.
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(bool, String)>();
            let handle = tokio::spawn({
                let branch = branch.clone();
                let path = path.clone();
                let plugin = plugin.clone();
                let tx2 = tx.clone();
                async move {
                    let branch = if branch.is_empty() { None } else { Some(branch.as_str()) };
                    let path = if path.is_empty() { None } else { Some(std::path::Path::new(&path)) };
                    crabtalk_plugins::plugin::install(
                        &plugin, branch, path, force,
                        |msg| { let _ = tx.send((false, msg.to_string())); },
                        |msg| { let _ = tx2.send((true, msg.to_string())); },
                    )
                    .await
                }
            });

            // Drain progress messages while install runs.
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
                                // Sender dropped — task finished, await it.
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

            // Reload daemon to pick up new components.
            yield plugin_step("reloading daemon…");
            daemon.reload().await?;

            // Conflict and auth warnings.
            let (manifest, mut warnings) = daemon.resolve_manifests()?;
            warnings.extend(wcore::check_skill_conflicts(&manifest.skill_dirs));
            for w in &warnings {
                yield plugin_warning(w);
            }
            for (name, mcp) in &manifest.mcps {
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

    fn uninstall_plugin(
        &self,
        plugin: String,
    ) -> impl futures_core::Stream<Item = Result<PluginEvent>> + Send {
        let daemon = self.clone();
        async_stream::try_stream! {
            // Channel bridge for on_step callback.
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
            let name = plugin.clone();
            let handle = tokio::spawn(async move {
                crabtalk_plugins::plugin::uninstall(&name, |msg| {
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
            daemon.reload().await?;
            yield plugin_done("");
        }
    }

    async fn list_conversations(
        &self,
        agent: String,
        sender: String,
    ) -> Result<Vec<ConversationInfo>> {
        let sessions_dir = self.config_dir.join("sessions");
        tokio::task::spawn_blocking(move || scan_conversations_all(&sessions_dir, &agent, &sender))
            .await
            .context("conversation scan task panicked")
    }

    async fn get_conversation_history(&self, file_path: String) -> Result<ConversationHistory> {
        let path = std::path::PathBuf::from(&file_path);
        anyhow::ensure!(path.exists(), "conversation file not found: {file_path}");
        let (meta, messages) =
            tokio::task::spawn_blocking(move || wcore::Conversation::load_context(&path))
                .await
                .context("load_context task panicked")??;
        Ok(ConversationHistory {
            title: meta.title,
            agent: meta.agent,
            messages: messages
                .into_iter()
                .filter(|m| {
                    !matches!(
                        m.role,
                        wcore::model::Role::System | wcore::model::Role::Tool
                    )
                })
                .map(|m| ConversationMessage {
                    role: serde_json::to_value(&m.role)
                        .ok()
                        .and_then(|v| v.as_str().map(String::from))
                        .unwrap_or_default(),
                    content: m.content,
                })
                .collect(),
        })
    }

    async fn delete_conversation(&self, file_path: String) -> Result<()> {
        let path = std::path::Path::new(&file_path);
        anyhow::ensure!(path.exists(), "conversation file not found: {file_path}");
        std::fs::remove_file(path).with_context(|| format!("failed to delete {file_path}"))?;
        Ok(())
    }

    async fn list_mcps(&self) -> Result<Vec<McpInfo>> {
        let config = self.load_config()?;
        let rt = self.runtime.read().await.clone();
        let connected: std::collections::BTreeMap<String, usize> = rt
            .hook
            .mcp_servers()
            .into_iter()
            .map(|(name, tools)| (name, tools.len()))
            .collect();

        let mut mcps = Vec::new();

        // Local MCPs from CrabTalk.toml.
        let manifest_path = self
            .config_dir
            .join(wcore::paths::LOCAL_DIR)
            .join("CrabTalk.toml");
        if let Ok(Some(local)) = wcore::ManifestConfig::load(&manifest_path) {
            for (name, cfg) in &local.mcps {
                let enabled = !config.disabled.mcps.contains(name);
                let (status, tool_count) = mcp_status(&connected, name, enabled);
                mcps.push(mcp_to_info(
                    name,
                    cfg,
                    "local",
                    SourceKind::Local,
                    enabled,
                    status,
                    tool_count,
                ));
            }
        }

        // Plugin-installed MCPs.
        for (plugin_name, plugin_manifest) in scan_plugin_manifests(&self.config_dir) {
            for (name, mcp_res) in &plugin_manifest.mcps {
                if mcps.iter().any(|m| m.name == *name) {
                    continue; // local wins
                }
                let enabled = !config.disabled.mcps.contains(name);
                let (status, tool_count) = mcp_status(&connected, name, enabled);
                let cfg = mcp_res.to_server_config();
                mcps.push(mcp_to_info(
                    name,
                    &cfg,
                    &plugin_name,
                    SourceKind::Plugin,
                    enabled,
                    status,
                    tool_count,
                ));
            }
        }

        Ok(mcps)
    }

    async fn set_local_mcps(&self, mcps: Vec<McpInfo>) -> Result<()> {
        use toml_edit::{Array, DocumentMut, Item, Table, value};

        let manifest_path = self
            .config_dir
            .join(wcore::paths::LOCAL_DIR)
            .join("CrabTalk.toml");
        let local_dir = self.config_dir.join(wcore::paths::LOCAL_DIR);
        std::fs::create_dir_all(&local_dir)
            .with_context(|| format!("cannot create {}", local_dir.display()))?;

        let content = if manifest_path.exists() {
            std::fs::read_to_string(&manifest_path)
                .with_context(|| format!("cannot read {}", manifest_path.display()))?
        } else {
            String::new()
        };
        let mut doc: DocumentMut = content
            .parse()
            .with_context(|| format!("invalid TOML in {}", manifest_path.display()))?;

        doc.remove("mcps");
        if !mcps.is_empty() {
            let mut mcps_table = Table::new();
            for mcp in &mcps {
                let mut tbl = Table::new();
                if !mcp.url.is_empty() {
                    tbl.insert("url", value(&mcp.url));
                } else {
                    if !mcp.command.is_empty() {
                        tbl.insert("command", value(&mcp.command));
                    }
                    if !mcp.args.is_empty() {
                        let mut arr = Array::new();
                        for a in &mcp.args {
                            arr.push(a.as_str());
                        }
                        tbl.insert("args", Item::Value(arr.into()));
                    }
                }
                if mcp.auth {
                    tbl.insert("auth", value(true));
                }
                if mcp.auto_restart {
                    tbl.insert("auto_restart", value(true));
                }
                if !mcp.env.is_empty() {
                    let mut env_tbl = Table::new();
                    for (k, v) in &mcp.env {
                        env_tbl.insert(k, value(v));
                    }
                    tbl.insert("env", Item::Table(env_tbl));
                }
                mcps_table.insert(&mcp.name, Item::Table(tbl));
            }
            doc.insert("mcps", Item::Table(mcps_table));
        }

        std::fs::write(&manifest_path, doc.to_string())
            .with_context(|| format!("failed to write {}", manifest_path.display()))?;
        self.reload().await
    }

    async fn set_provider(&self, name: String, config: String) -> Result<ProviderInfo> {
        use toml_edit::DocumentMut;

        let def: wcore::ProviderDef =
            serde_json::from_str(&config).context("invalid ProviderDef JSON")?;

        // Validate before writing: merge with existing providers and check.
        let daemon_config = self.load_config()?;
        let mut all_providers = daemon_config.provider;
        all_providers.insert(name.clone(), def.clone());
        crate::config::validate_providers(&all_providers)?;

        let toml_value = toml::to_string(&def).context("failed to serialize provider to TOML")?;
        let provider_doc: DocumentMut = toml_value
            .parse()
            .context("failed to parse provider TOML")?;

        let config_path = self.config_dir.join(wcore::paths::CONFIG_FILE);
        let content = std::fs::read_to_string(&config_path)
            .with_context(|| format!("cannot read {}", config_path.display()))?;
        let mut doc: DocumentMut = content
            .parse()
            .with_context(|| format!("invalid TOML in {}", config_path.display()))?;

        if doc.get("provider").is_none() {
            doc.insert("provider", toml_edit::Item::Table(toml_edit::Table::new()));
        }
        let provider_table = doc["provider"]
            .as_table_mut()
            .context("[provider] is not a table")?;

        let mut entry = toml_edit::Table::new();
        for (key, value) in provider_doc.as_table().iter() {
            entry.insert(key, value.clone());
        }
        provider_table.insert(&name, toml_edit::Item::Table(entry));

        std::fs::write(&config_path, doc.to_string())
            .with_context(|| format!("failed to write {}", config_path.display()))?;
        self.reload().await?;

        // Return the config as actually loaded by the daemon, not the input.
        let loaded_config = self.load_config()?;
        let loaded_json = loaded_config
            .provider
            .get(&name)
            .and_then(|def| serde_json::to_string(def).ok())
            .unwrap_or_default();
        let active_model = loaded_config.system.crab.model.clone().unwrap_or_default();
        let active = loaded_config
            .provider
            .get(&name)
            .is_some_and(|def| !active_model.is_empty() && def.models.contains(&active_model));
        let enabled = !loaded_config.disabled.providers.contains(&name);
        Ok(ProviderInfo {
            name,
            active,
            config: loaded_json,
            enabled,
        })
    }

    async fn delete_provider(&self, name: String) -> Result<()> {
        use toml_edit::DocumentMut;

        let config_path = self.config_dir.join(wcore::paths::CONFIG_FILE);
        let content = std::fs::read_to_string(&config_path)
            .with_context(|| format!("cannot read {}", config_path.display()))?;
        let mut doc: DocumentMut = content
            .parse()
            .with_context(|| format!("invalid TOML in {}", config_path.display()))?;

        let removed = doc
            .get_mut("provider")
            .and_then(|v| v.as_table_mut())
            .and_then(|t| t.remove(&name))
            .is_some();
        if !removed {
            anyhow::bail!("provider '{name}' not found");
        }

        std::fs::write(&config_path, doc.to_string())
            .with_context(|| format!("failed to write {}", config_path.display()))?;
        self.reload().await
    }

    async fn set_active_model(&self, model: String) -> Result<()> {
        use toml_edit::{DocumentMut, Item, Table, value};

        // Validate model exists in some provider.
        let daemon_config = self.load_config()?;
        let model_exists = daemon_config
            .provider
            .values()
            .any(|def| def.models.iter().any(|m| m == &model));
        if !model_exists {
            anyhow::bail!("model '{model}' not found in any provider");
        }

        let config_path = self.config_dir.join(wcore::paths::CONFIG_FILE);
        let content = std::fs::read_to_string(&config_path)
            .with_context(|| format!("cannot read {}", config_path.display()))?;
        let mut doc: DocumentMut = content
            .parse()
            .with_context(|| format!("invalid TOML in {}", config_path.display()))?;

        if doc.get("system").is_none() {
            doc.insert("system", Item::Table(Table::new()));
        }
        if let Some(system) = doc.get_mut("system").and_then(|s| s.as_table_mut()) {
            if system.get("crab").is_none() {
                system.insert("crab", Item::Table(Table::new()));
            }
            if let Some(crab) = system.get_mut("crab").and_then(|w| w.as_table_mut()) {
                crab.insert("model", value(&model));
            }
        }

        std::fs::write(&config_path, doc.to_string())
            .with_context(|| format!("failed to write {}", config_path.display()))?;
        self.reload().await
    }

    async fn list_provider_presets(&self) -> Result<Vec<ProviderPresetInfo>> {
        Ok(wcore::config::PROVIDER_PRESETS
            .iter()
            .map(|p| ProviderPresetInfo {
                name: p.name.to_string(),
                kind: wcore::protocol::message::ProtoProviderKind::from(p.kind).into(),
                base_url: p.base_url.to_string(),
                fixed_base_url: p.fixed_base_url.to_string(),
                default_model: p.default_model.to_string(),
            })
            .collect())
    }

    async fn list_skills(&self) -> Result<Vec<SkillInfo>> {
        let (manifest, _) = self.resolve_manifests()?;
        let local_skills_dir = self.config_dir.join(wcore::paths::SKILLS_DIR);

        // Reverse-lookup: dir path → package id.
        let dir_to_pkg: std::collections::BTreeMap<_, _> = manifest
            .plugin_skill_dirs
            .iter()
            .map(|(id, dir)| (dir.clone(), id.clone()))
            .collect();

        let mut seen = std::collections::BTreeSet::new();
        let mut skills = Vec::new();

        for dir in &manifest.skill_dirs {
            let (source, source_kind) = if *dir == local_skills_dir {
                ("local".to_string(), SourceKind::Local)
            } else if let Some(pkg_id) = dir_to_pkg.get(dir) {
                (pkg_id.clone(), SourceKind::Plugin)
            } else {
                let name = wcore::external_source_name(dir).unwrap_or("external");
                (name.to_string(), SourceKind::External)
            };

            for name in wcore::scan_skill_names(dir) {
                if !seen.insert(name.clone()) {
                    continue;
                }
                let enabled = !manifest.disabled.skills.contains(&name)
                    && (source_kind != SourceKind::External
                        || !manifest.disabled.external.contains(&source));
                skills.push(SkillInfo {
                    name,
                    enabled,
                    source: source.clone(),
                    source_kind: source_kind as i32,
                });
            }
        }
        Ok(skills)
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        let config = self.load_config()?;
        let active_model = config.system.crab.model.clone().unwrap_or_default();

        let mut models = Vec::new();
        for (provider_name, def) in &config.provider {
            let enabled = !config.disabled.providers.contains(provider_name);
            let kind: i32 = ProtoProviderKind::from(def.kind).into();
            for model_name in &def.models {
                models.push(ModelInfo {
                    name: model_name.clone(),
                    provider: provider_name.clone(),
                    active: *model_name == active_model,
                    enabled,
                    kind,
                });
            }
        }
        Ok(models)
    }

    async fn set_enabled(&self, kind: ResourceKind, name: String, enabled: bool) -> Result<()> {
        use toml_edit::DocumentMut;

        // Refuse to disable the active model's provider.
        if !enabled && kind == ResourceKind::Provider {
            let config = self.load_config()?;
            let active_model = config.system.crab.model.clone().unwrap_or_default();
            if !active_model.is_empty()
                && config
                    .provider
                    .get(&name)
                    .is_some_and(|def| def.models.contains(&active_model))
            {
                anyhow::bail!(
                    "cannot disable provider '{name}' — it serves the active model '{active_model}'"
                );
            }
        }

        let config_path = self.config_dir.join(wcore::paths::CONFIG_FILE);
        let content = std::fs::read_to_string(&config_path)
            .with_context(|| format!("cannot read {}", config_path.display()))?;
        let mut doc: DocumentMut = content
            .parse()
            .with_context(|| format!("invalid TOML in {}", config_path.display()))?;

        if doc.get("disabled").is_none() {
            doc.insert("disabled", toml_edit::Item::Table(toml_edit::Table::new()));
        }
        let disabled = doc["disabled"]
            .as_table_mut()
            .context("[disabled] is not a table")?;

        let key = match kind {
            ResourceKind::Provider => "providers",
            ResourceKind::Mcp => "mcps",
            ResourceKind::Skill => "skills",
            ResourceKind::ExternalSource => "external",
            ResourceKind::Unknown => anyhow::bail!("unknown resource kind"),
        };
        if disabled.get(key).is_none() {
            disabled.insert(key, toml_edit::Item::Value(toml_edit::Array::new().into()));
        }
        let arr = disabled[key]
            .as_array_mut()
            .context("disabled list is not an array")?;

        if enabled {
            let idx = arr.iter().position(|v| v.as_str() == Some(&name));
            if let Some(idx) = idx {
                arr.remove(idx);
            }
        } else if !arr.iter().any(|v| v.as_str() == Some(&name)) {
            arr.push(&name);
        }

        std::fs::write(&config_path, doc.to_string())
            .with_context(|| format!("failed to write {}", config_path.display()))?;
        self.reload().await
    }

    async fn list_plugins(&self) -> Result<Vec<PluginInfo>> {
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
        Ok(result)
    }

    async fn search_plugins(&self, query: String) -> Result<Vec<PluginInfo>> {
        let entries = crabtalk_plugins::plugin::search(&query).await?;
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

/// Scan `plugins/` for installed plugin manifests, returning `(name, Manifest)` pairs.
fn scan_plugin_manifests(
    config_dir: &std::path::Path,
) -> Vec<(String, crabtalk_plugins::manifest::Manifest)> {
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
        match toml::from_str::<crabtalk_plugins::manifest::Manifest>(&content) {
            Ok(manifest) => result.push((name.to_string(), manifest)),
            Err(e) => {
                tracing::warn!("failed to parse manifest {}: {e}", path.display());
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

impl<P: Provider + 'static, H: Host + 'static> Daemon<P, H> {
    /// Load the current `DaemonConfig` from disk.
    fn load_config(&self) -> Result<crate::DaemonConfig> {
        crate::DaemonConfig::load(&self.config_dir.join(wcore::paths::CONFIG_FILE))
    }

    /// Look up which provider name serves the given model name, by reading
    /// the on-disk config. Returns an empty string if not found or on error.
    /// Used by `send` / `stream_to` to attribute responses to a provider.
    fn provider_name_for_model(&self, model: &str) -> String {
        self.load_config()
            .ok()
            .and_then(|c| {
                c.provider
                    .iter()
                    .find(|(_, def)| def.models.iter().any(|m| m == model))
                    .map(|(name, _)| name.clone())
            })
            .unwrap_or_default()
    }

    /// Resolve manifests and apply disabled items from config.toml.
    fn resolve_manifests(&self) -> Result<(wcore::ResolvedManifest, Vec<String>)> {
        let config = self.load_config()?;
        let (mut manifest, warnings) = wcore::resolve_manifests(&self.config_dir);
        manifest.disabled = config.disabled;
        wcore::filter_disabled_external(&mut manifest.skill_dirs, &manifest.disabled.external);
        Ok((manifest, warnings))
    }

    /// Look up a command service by name from installed plugin manifests.
    fn find_command_service(
        &self,
        name: &str,
    ) -> Result<crabtalk_plugins::manifest::CommandConfig> {
        for (_, manifest) in scan_plugin_manifests(&self.config_dir) {
            if let Some(cmd) = manifest.commands.get(name) {
                return Ok(cmd.clone());
            }
        }
        anyhow::bail!("command service '{name}' not found in installed plugins")
    }

    /// Write an agent config into the local manifest `[agents.<name>]`.
    ///
    /// If `expect_new` is true, fails when the agent already exists in the
    /// manifest. If false, upserts (creates or overwrites).
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

        if expect_new && agents.contains_key(name) {
            anyhow::bail!("agent '{name}' already exists in local manifest");
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

    /// Write an agent's system prompt to `local/agents/{name}.md`.
    fn write_agent_prompt(&self, name: &str, prompt: &str) -> Result<()> {
        let agents_dir = self.config_dir.join(wcore::paths::AGENTS_DIR);
        std::fs::create_dir_all(&agents_dir).context("failed to create agents directory")?;
        std::fs::write(agents_dir.join(format!("{name}.md")), prompt)
            .context("failed to write agent prompt file")?;
        Ok(())
    }

    /// Write config into `[system.crab]` in `config.toml`.
    fn write_system_crab_config(&self, config_json: &str) -> Result<()> {
        use toml_edit::DocumentMut;

        let config: wcore::AgentConfig =
            serde_json::from_str(config_json).context("invalid AgentConfig JSON")?;
        let toml_value = toml::to_string(&config).context("failed to serialize agent to TOML")?;
        let agent_doc: DocumentMut = toml_value.parse().context("failed to parse agent TOML")?;

        let config_path = self.config_dir.join(wcore::paths::CONFIG_FILE);
        let mut doc: DocumentMut = if config_path.exists() {
            std::fs::read_to_string(&config_path)
                .context("failed to read config.toml")?
                .parse()
                .context("failed to parse config.toml")?
        } else {
            DocumentMut::default()
        };

        // Ensure [system] table exists.
        if doc.get("system").is_none() {
            doc.insert("system", toml_edit::Item::Table(toml_edit::Table::new()));
        }
        let system = doc["system"]
            .as_table_mut()
            .context("[system] is not a table")?;

        // Build and insert [system.crab] sub-table.
        let mut crab_table = toml_edit::Table::new();
        for (key, value) in agent_doc.as_table().iter() {
            crab_table.insert(key, value.clone());
        }
        system.insert("crab", toml_edit::Item::Table(crab_table));

        std::fs::write(&config_path, doc.to_string()).context("failed to write config.toml")?;
        Ok(())
    }
}

/// Reject agent names that could escape the agents directory.
fn validate_agent_name(name: &str) -> Result<()> {
    anyhow::ensure!(!name.is_empty(), "agent name cannot be empty");
    anyhow::ensure!(
        !name.contains('/') && !name.contains('\\') && !name.contains(".."),
        "agent name '{name}' contains invalid characters"
    );
    Ok(())
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

/// Scan session files and return conversation info.
///
/// If `agent` and `sender` are both empty, returns all conversations.
/// Otherwise, filters to the given identity.
fn scan_conversations_all(
    sessions_dir: &std::path::Path,
    agent: &str,
    sender: &str,
) -> Vec<ConversationInfo> {
    let Ok(entries) = std::fs::read_dir(sessions_dir) else {
        return Vec::new();
    };

    let filter_prefix = if !agent.is_empty() && !sender.is_empty() {
        Some(format!("{}_{}_", agent, wcore::sender_slug(sender)))
    } else {
        None
    };

    let today = chrono::Local::now().date_naive();
    let mut results = Vec::new();

    for file in entries.flatten() {
        let path = file.path();
        if path.is_dir() {
            continue;
        }
        let name = file.file_name();
        let Some(name) = name.to_str() else { continue };
        if !name.ends_with(".jsonl") {
            continue;
        }

        if let Some(ref prefix) = filter_prefix
            && !name.starts_with(prefix)
        {
            continue;
        }

        let Some((file_agent, file_sender, seq, title)) = parse_session_filename(name) else {
            continue;
        };

        if filter_prefix.is_none() && !agent.is_empty() && file_agent != agent {
            continue;
        }

        let mtime = file
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        let (alive_secs, message_count) = read_session_file_stats(&path);
        let date = mtime_to_label(mtime, today);

        results.push((
            mtime,
            ConversationInfo {
                agent: file_agent,
                sender: file_sender,
                seq,
                title,
                file_path: path.to_string_lossy().into_owned(),
                message_count,
                alive_secs,
                date,
            },
        ));
    }

    // Sort by mtime descending (most recently active first).
    results.sort_by(|a, b| b.0.cmp(&a.0));
    results.into_iter().map(|(_, info)| info).collect()
}

/// Parse a session filename into (agent, sender, seq, title).
///
/// Format: `{agent}_{sender}_{seq}[_{title}].jsonl`
fn parse_session_filename(name: &str) -> Option<(String, String, u32, String)> {
    let stem = name.strip_suffix(".jsonl")?;
    let parts: Vec<&str> = stem.split('_').collect();
    if parts.len() < 3 {
        return None;
    }
    // Find the first numeric part after position 1 (that's the seq).
    for i in 2..parts.len() {
        if !parts[i].is_empty() && parts[i].chars().all(|c| c.is_ascii_digit()) {
            let agent = parts[0].to_string();
            let sender = parts[1..i].join("_");
            let seq: u32 = parts[i].parse().ok()?;
            let title = if i + 1 < parts.len() {
                parts[i + 1..].join("_")
            } else {
                String::new()
            };
            return Some((agent, sender, seq, title));
        }
    }
    None
}

/// Read uptime_secs from meta line and count message lines.
fn read_session_file_stats(path: &std::path::Path) -> (u64, u64) {
    use std::io::{BufRead, BufReader};

    let Ok(file) = std::fs::File::open(path) else {
        return (0, 0);
    };
    let reader = BufReader::new(file);
    let mut lines = reader.lines();

    let uptime = lines
        .next()
        .and_then(|l| l.ok())
        .and_then(|l| {
            let v: serde_json::Value = serde_json::from_str(&l).ok()?;
            v.get("uptime_secs")?.as_u64()
        })
        .unwrap_or(0);

    let msg_count = lines
        .map_while(|l| l.ok())
        .filter(|l| !l.trim().is_empty() && !l.contains("\"compact\""))
        .count() as u64;

    (uptime, msg_count)
}

/// Convert a file mtime to a human-readable date label.
fn mtime_to_label(mtime: std::time::SystemTime, today: chrono::NaiveDate) -> String {
    let date = chrono::DateTime::<chrono::Local>::from(mtime).date_naive();
    if date == today {
        "Today".to_string()
    } else if date == today - chrono::Duration::days(1) {
        "Yesterday".to_string()
    } else {
        date.format("%Y-%m-%d").to_string()
    }
}

fn mcp_status(
    connected: &std::collections::BTreeMap<String, usize>,
    name: &str,
    enabled: bool,
) -> (McpStatus, u32) {
    if !enabled {
        return (McpStatus::Disconnected, 0);
    }
    match connected.get(name) {
        Some(&count) => (McpStatus::Connected, count as u32),
        None => (McpStatus::Failed, 0),
    }
}

fn mcp_to_info(
    name: &str,
    cfg: &wcore::McpServerConfig,
    source: &str,
    source_kind: SourceKind,
    enabled: bool,
    status: McpStatus,
    tool_count: u32,
) -> McpInfo {
    McpInfo {
        name: name.to_string(),
        command: cfg.command.clone(),
        args: cfg.args.clone(),
        env: cfg
            .env
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
        url: cfg.url.clone().unwrap_or_default(),
        auth: cfg.auth,
        source: source.to_string(),
        auto_restart: cfg.auto_restart,
        enabled,
        source_kind: source_kind.into(),
        status: status.into(),
        error: String::new(),
        tool_count,
    }
}

fn agent_config_to_info(config: &wcore::AgentConfig) -> AgentInfo {
    AgentInfo {
        name: config.name.clone(),
        description: config.description.clone(),
        config: String::new(),
        model: config.model.clone(),
        max_iterations: config.max_iterations as u32,
        thinking: config.thinking,
        members: config.members.clone(),
        skills: config.skills.clone(),
        mcps: config.mcps.clone(),
        compact_threshold: config.compact_threshold.map(|t| t as u32),
        compact_tool_max_len: config.compact_tool_max_len as u32,
    }
}

fn subscription_to_info(sub: &EventSubscription) -> SubscriptionInfo {
    SubscriptionInfo {
        id: sub.id,
        source: sub.source.clone(),
        target_agent: sub.target_agent.clone(),
        once: sub.once,
    }
}

fn cron_entry_to_info(e: &CronEntry) -> CronInfo {
    CronInfo {
        id: e.id,
        schedule: e.schedule.clone(),
        skill: e.skill.clone(),
        agent: e.agent.clone(),
        quiet_start: e.quiet_start.clone().unwrap_or_default(),
        quiet_end: e.quiet_end.clone().unwrap_or_default(),
        once: e.once,
        sender: e.sender.clone(),
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
