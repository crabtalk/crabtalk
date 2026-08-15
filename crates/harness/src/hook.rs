//! Harness tools, as the runtime sees them.
//!
//! Their tools land in the agent's list under their own names, read from the
//! manifest at load — the per-agent declaration is already the gate, so there
//! is no meta-tool to go through (RFC 0205).
//!
//! Images are keyed by `(agent, harness)` rather than by harness alone,
//! because the grant lives in the declaration: two agents may install the same
//! ELF against different roots, and they must not share a linker.
//!
//! Entering a guest blocks the thread it runs on, and `exec` can hold it for
//! the length of a command, so dispatch hands the invocation to the blocking
//! pool rather than running it on an async worker.

use crate::{Dispatch, Grants, Harness};
use crabllm_core::Tool;
use runtime::Hook;
use rvtime::{Config, Engine};
use std::{
    collections::BTreeMap,
    sync::{Arc, OnceLock, RwLock},
};
use wcore::{
    AgentConfig, HarnessConfig, ToolDispatch, ToolFuture,
    model::{FunctionDef, ToolType},
};

/// Every harness image the daemon has loaded.
pub struct HarnessHook {
    engine: Engine,
    /// `(agent, harness name)` to the image that agent's declaration granted.
    loaded: RwLock<BTreeMap<(String, String), Arc<Harness>>>,
    /// The runtime's own door, connected once the daemon that implements it
    /// exists — which is after these images load, since it is built on them.
    protocol: Arc<OnceLock<Dispatch>>,
}

impl HarnessHook {
    /// An engine whose generated code is cached under the config directory, so
    /// a restart pays ~3ms per image instead of ~15ms.
    /// `protocol` is filled by the daemon once it exists. Until then a granted
    /// protocol capability is present but answers that it is not connected,
    /// which is a clearer failure than a call that waits for one.
    pub fn new(protocol: Arc<OnceLock<Dispatch>>) -> anyhow::Result<Self> {
        let mut config = Config::new();
        config.cache_dir(wcore::paths::CONFIG_DIR.join("cache/harness"));
        Ok(Self {
            engine: Engine::new(&config)?,
            loaded: RwLock::new(BTreeMap::new()),
            protocol,
        })
    }

    /// Load what `agent` declared. Failures are logged rather than fatal: one
    /// unreadable image should cost its own tools, not the daemon's startup.
    pub fn load(&self, agent: &str, declarations: &[HarnessConfig]) {
        for declaration in declarations {
            match self.image(declaration) {
                Ok(harness) => {
                    self.loaded.write().expect("harness registry").insert(
                        (agent.to_owned(), declaration.name.clone()),
                        Arc::new(harness),
                    );
                }
                Err(error) => tracing::warn!(
                    agent,
                    harness = declaration.name,
                    "harness not loaded: {error:#}"
                ),
            }
        }
    }

    /// Read one image and grant it what the declaration says.
    ///
    /// The daemon does not download code: it loads what is present and errors
    /// if it is not. Fetching because a config named something would be the
    /// daemon making a policy decision with a network connection.
    fn image(&self, declaration: &HarnessConfig) -> anyhow::Result<Harness> {
        let path = wcore::paths::HARNESSES_DIR.join(format!("{}.elf", declaration.name));
        let elf = std::fs::read(&path).map_err(|e| {
            anyhow::anyhow!(
                "{}: {e} — `make harness` installs images here",
                path.display()
            )
        })?;

        let granted = |name: &str| declaration.capabilities.iter().any(|c| c == name);
        Harness::load(
            &self.engine,
            &elf,
            &Grants {
                root: declaration.root.clone(),
                fs: granted("fs"),
                exec: granted("exec"),
                protocol_read: granted("protocol:read"),
            },
            self.protocol.clone(),
        )
    }

    /// The image serving `tool` for `agent`.
    fn owner(&self, agent: &str, tool: &str) -> Option<Arc<Harness>> {
        self.loaded
            .read()
            .expect("harness registry")
            .iter()
            .find(|((owner, _), harness)| {
                owner == agent && harness.manifest().tools.iter().any(|t| t.name == tool)
            })
            .map(|(_, harness)| harness.clone())
    }

    /// Tool names an agent's declarations bring.
    fn names(&self, agent: &str) -> Vec<String> {
        self.loaded
            .read()
            .expect("harness registry")
            .iter()
            .filter(|((owner, _), _)| owner == agent)
            .flat_map(|(_, harness)| harness.manifest().tools.iter().map(|t| t.name.clone()))
            .collect()
    }
}

impl Hook for HarnessHook {
    /// Every harness tool, for the schema catalogue. What an agent may
    /// actually call is [`Hook::scoped_tools`].
    fn schema(&self) -> Vec<Tool> {
        self.loaded
            .read()
            .expect("harness registry")
            .values()
            .flat_map(|harness| harness.manifest().tools.clone())
            .map(|tool| Tool {
                kind: ToolType::Function,
                function: FunctionDef {
                    name: tool.name,
                    description: Some(tool.description),
                    parameters: Some(tool.parameters),
                },
                strict: None,
            })
            .collect()
    }

    fn on_register_agent(&self, name: &str, config: &AgentConfig) {
        self.load(name, &config.harnesses);
    }

    fn on_unregister_agent(&self, name: &str) {
        self.loaded
            .write()
            .expect("harness registry")
            .retain(|(agent, _), _| agent != name);
    }

    fn scoped_tools(&self, config: &AgentConfig) -> (Vec<String>, Option<String>) {
        (self.names(&config.name), None)
    }

    fn dispatch<'a>(&'a self, name: &'a str, call: ToolDispatch) -> Option<ToolFuture<'a>> {
        let harness = self.owner(&call.agent, name)?;
        let tool = name.to_owned();
        Some(Box::pin(async move {
            let invocation =
                tokio::task::spawn_blocking(move || harness.call(&tool, call.args.into_bytes()))
                    .await
                    .map_err(|e| format!("harness invocation panicked: {e}"))?;

            // The outer error is the host's — a trap, a missing tool — and
            // reaches the model as something it cannot fix. The inner one is
            // the harness reporting its own failure, which is a tool result.
            invocation.map_err(|e| format!("{e:#}"))?
        }))
    }
}
