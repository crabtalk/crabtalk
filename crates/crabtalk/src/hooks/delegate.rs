//! Delegate tool — as a Hook implementation.

use crate::system::SharedRuntime;
use crabllm_core::Provider;
use runtime::Hook;
use serde::Deserialize;
use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicU64, Ordering},
};
use wcore::{ToolDispatch, ToolFuture, agent::AsTool};

/// Delegate tasks to other agents. Runs all tasks in parallel.
///
/// Set `background=true` to return immediately with task IDs — results
/// arrive via agent completion events.
#[derive(Deserialize, schemars::JsonSchema)]
pub struct Delegate {
    /// List of tasks to run in parallel. Each task has an agent name and a message.
    pub tasks: Vec<DelegateTask>,
    /// If true, return immediately with task IDs instead of waiting for completion.
    #[serde(default)]
    pub background: bool,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct DelegateTask {
    /// Target agent name. Auto-generated if empty and system_prompt is set.
    #[serde(default)]
    pub agent: String,
    /// Message/instruction for the target agent.
    pub message: String,
    /// System prompt for an ephemeral agent.
    #[serde(default)]
    pub system_prompt: Option<String>,
}

/// Delegate subsystem: dispatch tasks to other agents.
pub struct DelegateHook<P: Provider + 'static> {
    runtime: Arc<OnceLock<SharedRuntime<P>>>,
}

impl<P: Provider + 'static> DelegateHook<P> {
    pub fn new(runtime: Arc<OnceLock<SharedRuntime<P>>>) -> Self {
        Self { runtime }
    }
}

impl<P: Provider + 'static> Hook for DelegateHook<P> {
    fn schema(&self) -> Vec<wcore::model::Tool> {
        vec![Delegate::as_tool()]
    }

    fn dispatch<'a>(&'a self, name: &'a str, call: ToolDispatch) -> Option<ToolFuture<'a>> {
        if name != "delegate" {
            return None;
        }
        Some(Box::pin(async move {
            let input: Delegate =
                serde_json::from_str(&call.args).map_err(|e| format!("invalid arguments: {e}"))?;
            if input.tasks.is_empty() {
                return Err("no tasks provided".to_owned());
            }
            let shared = self
                .runtime
                .get()
                .ok_or_else(|| "delegate: runtime not initialized".to_owned())?;
            dispatch_delegate(input, shared).await
        }))
    }
}

async fn dispatch_delegate<P: Provider + 'static>(
    input: Delegate,
    shared: &SharedRuntime<P>,
) -> Result<String, String> {
    let mut ephemeral_names = Vec::new();
    let mut tasks = Vec::with_capacity(input.tasks.len());
    for task in input.tasks {
        let agent_name = if let Some(prompt) = task.system_prompt {
            let name = if task.agent.is_empty() {
                ephemeral_agent_name()
            } else {
                task.agent
            };
            let mut config = wcore::AgentConfig::new(&name);
            config.system_prompt = prompt;
            let rt = shared.read().await.clone();
            rt.add_ephemeral(config).await;
            ephemeral_names.push(name.clone());
            name
        } else {
            task.agent
        };

        let sender = delegate_sender();
        let handle = spawn_agent_task(
            shared.clone(),
            agent_name.clone(),
            task.message,
            sender.clone(),
        );
        tasks.push((agent_name, sender, handle));
    }

    if input.background {
        let mut json_results = Vec::with_capacity(tasks.len());
        let mut handles = Vec::with_capacity(tasks.len());
        for (agent, sender, handle) in tasks {
            json_results.push(serde_json::json!({ "agent": agent, "task_id": sender }));
            handles.push(handle);
        }
        if !ephemeral_names.is_empty() {
            let shared = shared.clone();
            tokio::spawn(async move {
                for h in handles {
                    let _ = h.await;
                }
                let rt = shared.read().await.clone();
                for name in ephemeral_names {
                    rt.remove_ephemeral(&name).await;
                }
            });
        }
        return serde_json::to_string(&json_results)
            .map_err(|e| format!("serialization error: {e}"));
    }

    let mut results = Vec::with_capacity(tasks.len());
    for (agent_name, _sender, handle) in tasks {
        let (result, error) = match handle.await {
            Ok((r, e)) => (r, e),
            Err(e) => (None, Some(format!("task panicked: {e}"))),
        };
        results.push(serde_json::json!({
            "agent": agent_name,
            "result": result,
            "error": error,
        }));
    }

    if !ephemeral_names.is_empty() {
        let rt = shared.read().await.clone();
        for name in ephemeral_names {
            rt.remove_ephemeral(&name).await;
        }
    }

    serde_json::to_string(&results).map_err(|e| format!("serialization error: {e}"))
}

fn delegate_sender() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("delegate:{id}")
}

fn ephemeral_agent_name() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("_ephemeral:{id}")
}

fn spawn_agent_task<P: Provider + 'static>(
    shared: SharedRuntime<P>,
    agent: String,
    message: String,
    delegate_sender: String,
) -> tokio::task::JoinHandle<(Option<String>, Option<String>)> {
    tokio::spawn(async move {
        let rt = shared.read().await.clone();
        let conversation_id = match rt
            .get_or_create_conversation(&agent, &delegate_sender)
            .await
        {
            Ok(id) => id,
            Err(e) => return (None, Some(e.to_string())),
        };

        let (result_content, error_msg) = match rt
            .send_to(conversation_id, &message, &delegate_sender, None)
            .await
        {
            Ok(response) => (response.final_response, None),
            Err(e) => (None, Some(e.to_string())),
        };

        rt.close(conversation_id).await;

        (result_content, error_msg)
    })
}
