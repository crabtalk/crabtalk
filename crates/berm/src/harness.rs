//! Harness tools, as the runtime sees them.
//!
//! Their tools land in the agent's list under their own names, read from the
//! manifest at load — the per-agent declaration is already the gate, so there
//! is no meta-tool to go through (RFC 0205).
//!
//! An image is keyed by what determines it — the ELF, the grants it runs
//! under, and the scope a granted capability closes over — not by the agent
//! that declared it. The grant still decides: two agents installing the same
//! ELF against different roots hash differently and get two linkers. But two
//! that declare it identically share one image, and a rename changes nothing
//! about the key, because the agent's name was never part of it.
//!
//! Entering a harness blocks the thread it runs on, and `exec` can hold it for
//! the length of a command, so dispatch hands the invocation to the blocking
//! pool rather than running it on an async worker.

use crate::{Dispatch, Scope};
use berm::{Capability, Config, Engine, Grants, Harness};
use crabllm_core::Tool;
use crabllm_core::{FunctionDef, ToolType};
use runtime::{ToolDispatch, ToolFuture};
use sha2::{Digest as _, Sha256};
use std::{
    collections::BTreeMap,
    sync::{Arc, OnceLock, RwLock},
};
use storage::{AgentConfig, AgentId, HarnessConfig};

/// What names an image: a SHA-256 over the ELF and everything the sandbox is
/// built with.
type Digest = [u8; 32];

/// Every harness image the daemon has loaded, and who declared it.
///
/// One lock over both maps: the two are only ever read or written together,
/// and a single guard is one fewer ordering rule to get wrong.
#[derive(Default)]
struct Registry {
    /// Digest to the image it names.
    images: BTreeMap<Digest, Arc<Harness>>,
    /// The images each agent's declarations resolved to, in declaration order.
    agents: BTreeMap<AgentId, Vec<Digest>>,
}

impl Registry {
    /// Drop images no declaration points at any more. Called after every
    /// change to `agents`, so an agent losing a harness loses its tools —
    /// the registry holds what is declared now, not what once was.
    fn sweep(&mut self) {
        let Self { images, agents } = self;
        images.retain(|digest, _| agents.values().flatten().any(|d| d == digest));
    }

    /// The images `agent` declared, in order.
    fn of(&self, agent: &AgentId) -> impl Iterator<Item = &Arc<Harness>> {
        self.agents
            .get(agent)
            .into_iter()
            .flatten()
            .filter_map(|digest| self.images.get(digest))
    }
}

pub struct HarnessHook {
    engine: Engine,
    registry: RwLock<Registry>,
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
        config.cache_dir(crabup::CONFIG_DIR.join("cache/berm"));
        Ok(Self {
            engine: Engine::new(&config)?,
            registry: RwLock::new(Registry::default()),
            protocol,
        })
    }

    /// Load what `agent` declared, replacing whatever it declared before.
    /// Failures are logged rather than fatal: one unreadable image should cost
    /// its own tools, not the daemon's startup.
    ///
    /// The registry is held for the whole pass so two agents registering at
    /// once cannot compile the same image twice.
    pub fn load(&self, agent: &AgentId, config: &AgentConfig) {
        let mut registry = self.registry.write().expect("harness registry");
        let mut declared = Vec::new();
        for declaration in &config.harnesses {
            match self.image(&mut registry, agent, declaration, &config.skills) {
                Ok(digest) => declared.push(digest),
                Err(error) => tracing::warn!(
                    %agent,
                    harness = declaration.name,
                    "harness not loaded: {error:#}"
                ),
            }
        }
        registry.agents.insert(*agent, declared);
        registry.sweep();
    }

    /// Read one image, grant it what the declaration says, and return the
    /// digest it is keyed by. An image already in the registry under that
    /// digest is the same sandbox, so it is reused rather than recompiled.
    ///
    /// The daemon does not download code: it loads what is present and errors
    /// if it is not. Fetching because a config named something would be the
    /// daemon making a policy decision with a network connection.
    fn image(
        &self,
        registry: &mut Registry,
        agent: &AgentId,
        declaration: &HarnessConfig,
        skills: &[String],
    ) -> anyhow::Result<Digest> {
        let path = crate::HARNESSES_DIR.join(format!("{}.elf", declaration.name));
        let elf = std::fs::read(&path).map_err(|e| {
            anyhow::anyhow!(
                "{}: {e} — `make harness` installs images here",
                path.display()
            )
        })?;

        let granted = |name: &str| declaration.capabilities.iter().any(|c| c == name);
        let grants = Grants {
            root: declaration.root.clone(),
            fs: granted("fs"),
            exec: granted("exec"),
        };
        // The runtime is not something berm knows about, so it arrives the way
        // any embedder's capability does. The groups the declaration granted
        // are captured here and checked on decode.
        let read = granted("protocol:read");
        let sessions = granted("protocol:sessions");
        let scope = (read || sessions).then(|| Scope {
            read,
            sessions,
            skills: skills.to_vec(),
            agent: *agent,
        });
        // The hosts are the grant, exactly as the root is: naming the
        // capability without naming where it may go reaches nothing.
        let hosts =
            (granted("http") && !declaration.hosts.is_empty()).then(|| declaration.hosts.clone());

        let digest = digest(&elf, &grants, scope.as_ref(), hosts.as_deref());
        if registry.images.contains_key(&digest) {
            return Ok(digest);
        }

        let mut extra = Vec::new();
        if let Some(scope) = scope {
            let protocol = self.protocol.clone();
            extra.push(Capability {
                name: crate::protocol::CALL.to_owned(),
                call: Arc::new(move |request| crate::protocol::call(&protocol, request, &scope)),
            });
        }
        if let Some(hosts) = hosts {
            extra.push(Capability {
                name: crate::http::FETCH.to_owned(),
                call: Arc::new(move |request| crate::http::call(&hosts, request)),
            });
        }

        let harness = Harness::load(&self.engine, &elf, &grants, &extra)?;
        registry.images.insert(digest, Arc::new(harness));
        Ok(digest)
    }

    /// The image serving `tool` for `agent`.
    fn owner(&self, agent: &AgentId, tool: &str) -> Option<Arc<Harness>> {
        self.registry
            .read()
            .expect("harness registry")
            .of(agent)
            .find(|harness| harness.manifest().tools.iter().any(|t| t.name == tool))
            .cloned()
    }

    /// Tool names an agent's declarations bring.
    fn names(&self, agent: &AgentId) -> Vec<String> {
        self.registry
            .read()
            .expect("harness registry")
            .of(agent)
            .flat_map(|harness| harness.manifest().tools.iter().map(|t| t.name.clone()))
            .collect()
    }
}

/// The digest that names an image: the ELF, the grants it is instantiated
/// with, and the scope a granted capability closes over. Everything that
/// changes what the sandbox *is* is in here; nothing else is, so a rename or
/// a second agent declaring the same thing is not a new image.
///
/// `skills` reaches the harness through `scope`, so it must be hashed — two
/// agents sharing a harness but not a skill list are not the same sandbox.
/// `hosts` is here for the same reason, and matters more: it is the whole of
/// what bounds `http`.
fn digest(elf: &[u8], grants: &Grants, scope: Option<&Scope>, hosts: Option<&[String]>) -> Digest {
    let mut hasher = Sha256::new();
    hasher.update(elf);
    hasher.update([grants.fs as u8, grants.exec as u8]);
    if let Some(root) = &grants.root {
        hasher.update(root.as_os_str().as_encoded_bytes());
    }
    hasher.update([0]);
    if let Some(scope) = scope {
        hasher.update([1, scope.read as u8, scope.sessions as u8]);
        // Narrowing is per-agent, so two agents declaring the same session
        // harness are deliberately two images: sharing one would be sharing
        // the narrowing.
        hasher.update(scope.agent.to_string().as_bytes());
        hasher.update([0]);
        for skill in &scope.skills {
            hasher.update(skill.as_bytes());
            hasher.update([0]);
        }
    }
    hasher.update([0]);
    for host in hosts.unwrap_or_default() {
        hasher.update(host.as_bytes());
        hasher.update([0]);
    }
    hasher.finalize().into()
}

impl runtime::Harness for HarnessHook {
    /// Every harness tool, for the schema catalogue. What an agent may
    /// actually call is [`runtime::Harness::scoped_tools`].
    fn schema(&self) -> Vec<Tool> {
        self.registry
            .read()
            .expect("harness registry")
            .images
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
                cache_control: None,
            })
            .collect()
    }

    /// Append the usage each declared harness carries.
    ///
    /// Per-agent rather than through [`runtime::Harness::usage`], which has no agent in
    /// its signature and would put every harness's text in front of every
    /// agent. The declaration is the gate here as everywhere else.
    ///
    /// Read straight off the ELF, because this runs *before*
    /// `on_register_agent` and nothing is compiled yet. That is what the
    /// manifest being a section rather than an export buys: the text is
    /// available without instantiating anything.
    fn on_build_agent(&self, mut config: AgentConfig) -> AgentConfig {
        for declaration in &config.harnesses {
            let path = crate::HARNESSES_DIR.join(format!("{}.elf", declaration.name));
            let usage = std::fs::read(&path)
                .ok()
                .and_then(|elf| berm::manifest(&elf).ok())
                .map(|manifest| manifest.usage)
                .unwrap_or_default();
            if !usage.is_empty() {
                config.description.push_str("\n\n");
                config.description.push_str(usage.trim_end());
            }
        }
        config
    }

    fn on_register_agent(&self, id: &AgentId, config: &AgentConfig) {
        self.load(id, config);
    }

    fn on_unregister_agent(&self, id: &AgentId) {
        let mut registry = self.registry.write().expect("harness registry");
        registry.agents.remove(id);
        registry.sweep();
    }

    fn scoped_tools(&self, config: &AgentConfig) -> (Vec<String>, Option<String>) {
        (self.names(&config.id), None)
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
