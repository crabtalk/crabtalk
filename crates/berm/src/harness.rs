//! Harness tools, as the runtime sees them.
//!
//! Their tools land in the agent's list under their own names, read from the
//! manifest at load — the per-agent declaration is already the gate, so there
//! is no meta-tool to go through (RFC 0205).
//!
//! An image is keyed by what determines it — the ELF, the grants it runs
//! under, and the scope a granted system harness closes over — not by the agent
//! that declared it. The grant still decides: two agents installing the same
//! ELF against different roots hash differently and get two linkers. But two
//! that declare it identically share one image, and a rename changes nothing
//! about the key, because the agent's name was never part of it.
//!
//! Entering a harness blocks the thread it runs on, and `exec` can hold it for
//! the length of a command, so dispatch hands the invocation to the blocking
//! pool rather than running it on an async worker.

use crate::{Dispatch, Http, Protocol, Scope, exec, fs};
use anyhow::Context as _;
use berm::{Berm, Config, Engine, Manifest};
use crabllm_core::{FunctionDef, Tool, ToolType};
use runtime::{ToolDispatch, ToolFuture};
use sha2::{Digest as _, Sha256};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, OnceLock, RwLock},
};
use store::{AgentConfig, AgentId, HarnessConfig, Root};
use tokio::runtime::Handle;

/// What names an image: a SHA-256 over the ELF and everything the sandbox is
/// built with.
type Digest = [u8; 32];

/// What an agent declared, kept because a `Root::Session` declaration cannot
/// be built until a session names its root — which is after the agent
/// resolved, and every time a new root turns up.
struct Declared {
    harnesses: Vec<HarnessConfig>,
    skills: Vec<String>,
}

/// Every harness image the daemon has loaded, and who declared it.
///
/// One lock over all three maps: they are only ever read or written together,
/// and a single guard is one fewer ordering rule to get wrong.
#[derive(Default)]
struct Registry {
    /// Digest to the image it names.
    images: BTreeMap<Digest, Arc<Berm>>,
    /// What each agent asked for, as written.
    declared: BTreeMap<AgentId, Declared>,
    /// The images an agent's declarations resolved to, in declaration order,
    /// under one session root. `None` is the resolution a session that named
    /// no root gets, and the one `scoped_tools` reads — tool *names* do not
    /// vary with the root, only the subtree they reach.
    agents: BTreeMap<(AgentId, Option<PathBuf>), Vec<Digest>>,
}

impl Registry {
    /// Drop images no declaration points at any more. Called after every
    /// change to `agents`, so an agent losing a harness loses its tools —
    /// the registry holds what is declared now, not what once was.
    fn sweep(&mut self) {
        let Self { images, agents, .. } = self;
        images.retain(|digest, _| agents.values().flatten().any(|d| d == digest));
    }

    /// The images `agent` resolved to under `root`, in order.
    fn of(&self, agent: &AgentId, root: Option<&Path>) -> impl Iterator<Item = &Arc<Berm>> {
        self.agents
            .get(&(*agent, root.map(Path::to_path_buf)))
            .into_iter()
            .flatten()
            .filter_map(|digest| self.images.get(digest))
    }

    /// Forget every resolution of `agent`, whatever root it was against.
    fn clear(&mut self, agent: &AgentId) {
        self.agents.retain(|(id, _), _| id != agent);
    }
}

pub struct BermHarness {
    engine: Engine,
    registry: RwLock<Registry>,
    /// Where images live, one `{name}.elf` each.
    images: PathBuf,
    /// The runtime's own door, connected once the daemon that implements it
    /// exists — which is after these images load, since it is built on them.
    protocol: Arc<OnceLock<Dispatch>>,
    /// What bridges a sync sandbox to an async runtime, taken once here rather
    /// than hunted for inside each call: whether a reactor exists is a fact
    /// about the embedder, so it is answered when the embedder is built and
    /// never again on a model's turn.
    reactor: Handle,
}

impl BermHarness {
    /// Both directories come from the embedder, which is the only thing that
    /// knows its own install: `images` is read from, and `cache` holds the
    /// engine's generated code so a restart pays ~3ms per image instead of
    /// ~15ms.
    ///
    /// `protocol` is filled by the daemon once it exists. Until then a granted
    /// protocol harness is present but answers that it is not connected,
    /// which is a clearer failure than a call that waits for one.
    pub fn new(
        protocol: Arc<OnceLock<Dispatch>>,
        images: PathBuf,
        cache: PathBuf,
    ) -> anyhow::Result<Self> {
        let mut config = Config::new();
        config.cache_dir(cache);
        Ok(Self {
            engine: Engine::new(&config)?,
            registry: RwLock::new(Registry::default()),
            images,
            protocol,
            reactor: Handle::try_current()
                .context("crabtalk's system harnesses need a reactor to reach the runtime")?,
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
        registry.clear(agent);
        registry.declared.insert(
            *agent,
            Declared {
                harnesses: config.harnesses.clone(),
                skills: config.skills.clone(),
            },
        );
        self.resolve(&mut registry, agent, None);
        registry.sweep();
    }

    /// Build `agent`'s declarations against `root`, filing them under it.
    ///
    /// Idempotent: a resolution already present is left alone, and an image
    /// two roots agree on is shared rather than compiled twice.
    fn resolve(&self, registry: &mut Registry, agent: &AgentId, root: Option<&Path>) {
        let key = (*agent, root.map(Path::to_path_buf));
        if registry.agents.contains_key(&key) {
            return;
        }
        let Some(declared) = registry.declared.get(agent) else {
            return;
        };
        let (harnesses, skills) = (declared.harnesses.clone(), declared.skills.clone());

        let mut digests = Vec::new();
        for declaration in &harnesses {
            match self.image(registry, agent, declaration, &skills, root) {
                Ok(digest) => digests.push(digest),
                Err(error) => tracing::warn!(
                    %agent,
                    harness = declaration.name,
                    "harness not loaded: {error:#}"
                ),
            }
        }
        registry.agents.insert(key, digests);
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
        session_root: Option<&Path>,
    ) -> anyhow::Result<Digest> {
        let path = self.images.join(format!("{}.elf", declaration.name));
        let elf = std::fs::read(&path).map_err(|e| {
            anyhow::anyhow!(
                "{}: {e} — `make harness` installs images here",
                path.display()
            )
        })?;

        // The declaration names system harnesses in crabtalk's vocabulary; berm
        // takes values. Translating between them is what this function does,
        // and the argument each value is built with is the grant — without a
        // root, `fs` and `exec` are never constructed.
        let granted = |name: &str| declaration.system.iter().any(|c| c == name);
        let read = granted("protocol:read");
        let sessions = granted("protocol:sessions");
        let scope = (read || sessions).then(|| Scope {
            read,
            sessions,
            skills: skills.to_vec(),
            agent: *agent,
        });

        let root = bind(declaration.root.as_ref(), session_root)?;

        let digest = digest(&elf, declaration, root.as_deref(), scope.as_ref());
        if registry.images.contains_key(&digest) {
            return Ok(digest);
        }

        let mut system = Vec::new();
        if let Some(root) = &root {
            if granted("fs") {
                system.push(fs::read(root.clone()));
                system.push(fs::write(root.clone()));
            }
            if granted("exec") {
                system.push(exec::run(root.clone()));
            }
        }
        if let Some(scope) = scope {
            system
                .push(Protocol::new(self.protocol.clone(), self.reactor.clone(), scope).harness());
        }
        if granted("http") && !declaration.hosts.is_empty() {
            system.push(Http::new(declaration.hosts.clone(), self.reactor.clone()).harness());
        }

        let harness = Berm::load(&self.engine, &elf, &system)?;
        registry.images.insert(digest, Arc::new(harness));
        Ok(digest)
    }

    /// The image serving `tool` for `agent` under `root`, building that
    /// resolution if this is the first call against it.
    ///
    /// The build is a compile, and it happens here rather than when the
    /// session opened, so the first tool call in a project pays for it once.
    fn owner(&self, agent: &AgentId, root: Option<&Path>, tool: &str) -> Option<Arc<Berm>> {
        let find = |registry: &Registry| {
            registry
                .of(agent, root)
                .find(|harness| harness.manifest().tools.iter().any(|t| t.name == tool))
                .cloned()
        };
        if let Some(found) = find(&self.registry.read().expect("harness registry")) {
            return found.into();
        }
        let mut registry = self.registry.write().expect("harness registry");
        self.resolve(&mut registry, agent, root);
        find(&registry)
    }

    /// Tool names an agent's declarations bring. Root-independent: a
    /// declaration reaches a different subtree under a different root, never
    /// different tools.
    fn names(&self, agent: &AgentId) -> Vec<String> {
        self.registry
            .read()
            .expect("harness registry")
            .of(agent, None)
            .flat_map(|harness| harness.manifest().tools.iter().map(|t| t.name.clone()))
            .collect()
    }
}

/// The path `fs` and `exec` are bounded by, or `None` when neither is granted.
///
/// A [`Root::Session`] declaration is the outer bound and the session narrows
/// inside it, so a session cannot widen its own reach — and one that named no
/// root gets the bound itself, which is what every declaration meant before a
/// session could narrow at all.
///
/// This is where [`Root::Home`] becomes a path, because it is the last moment
/// before one is needed and the first at which the running machine is the one
/// answering.
pub fn bind(declared: Option<&Root>, session: Option<&Path>) -> anyhow::Result<Option<PathBuf>> {
    let bound = match declared {
        None => return Ok(None),
        Some(Root::Fixed(path)) => return Ok(Some(path.clone())),
        Some(Root::Session(path)) => path.clone(),
        Some(Root::Home) => dirs::home_dir().context("no home directory to bind a harness to")?,
    };
    let Some(session) = session else {
        return Ok(Some(bound));
    };
    let session = session
        .to_str()
        .with_context(|| format!("session root {} is not utf-8", session.display()))?;
    crate::root::resolve(&bound, session).map(Some)
}

/// The digest that names an image: the ELF, and everything that determines the
/// system harnesses it is built with. Everything that changes what the sandbox
/// *is* is in here; nothing else is, so a rename or a second agent declaring
/// the same thing is not a new image.
///
/// The declaration covers which are constructed and what bounds them — `hosts`
/// for `http`, and for `fs` and `exec` the already-bound `root`, which is what
/// two sessions in one project share and two in different projects do not.
/// `scope` adds what only the agent knows: `read` and `sessions` are already in
/// the declaration, but the skills and the agent itself are not, and narrowing
/// is per-agent, so two agents declaring the same session harness are
/// deliberately two images.
fn digest(
    elf: &[u8],
    declaration: &HarnessConfig,
    root: Option<&Path>,
    scope: Option<&Scope>,
) -> Digest {
    let mut hasher = Sha256::new();
    hasher.update(elf);
    for name in &declaration.system {
        hasher.update(name.as_bytes());
        hasher.update([0]);
    }
    hasher.update([0]);
    if let Some(root) = root {
        hasher.update(root.as_os_str().as_encoded_bytes());
    }
    hasher.update([0]);
    for host in &declaration.hosts {
        hasher.update(host.as_bytes());
        hasher.update([0]);
    }
    hasher.update([0]);
    if let Some(scope) = scope {
        hasher.update(scope.agent.to_string().as_bytes());
        hasher.update([0]);
        for skill in &scope.skills {
            hasher.update(skill.as_bytes());
            hasher.update([0]);
        }
    }
    hasher.finalize().into()
}

impl runtime::Harness for BermHarness {
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
            let path = self.images.join(format!("{}.elf", declaration.name));
            let usage = std::fs::read(&path)
                .ok()
                .and_then(|elf| Manifest::from_elf(&elf).ok())
                .map(|manifest| manifest.usage)
                .unwrap_or_default();
            if !usage.is_empty() {
                config.description.push_str("\n\n");
                config.description.push_str(usage.trim_end());
            }
        }
        config
    }

    fn on_resolve_agent(&self, id: &AgentId, config: &AgentConfig) {
        self.load(id, config);
    }

    fn on_forget_agent(&self, id: &AgentId) {
        let mut registry = self.registry.write().expect("harness registry");
        registry.declared.remove(id);
        registry.clear(id);
        registry.sweep();
    }

    fn scoped_tools(&self, config: &AgentConfig) -> (Vec<String>, Option<String>) {
        (self.names(&config.id), None)
    }

    fn dispatch<'a>(&'a self, name: &'a str, call: ToolDispatch) -> Option<ToolFuture<'a>> {
        let harness = self.owner(&call.agent, call.root.as_deref(), name)?;
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
