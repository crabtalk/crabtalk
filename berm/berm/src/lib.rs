//! berm — a sandbox for harnesses.
//!
//! Loads a hash-pinned RV64 ELF, compiles it once, and instantiates it per
//! invocation under rvtime: arguments are pulled in through host calls, the
//! result is read back out of guest memory, and nothing survives the call.
//!
//! A harness reaches the world only through *system harnesses* it was given,
//! and the grant is the [`Linker`] it is instantiated with — an ungranted call
//! traps because nothing is registered for it, not because a check said no.
//!
//! A system harness is native host code behind a name, the way a precompile is
//! native code behind an address. berm ships none: what a filesystem is bounded
//! by, what shape a command's result takes, where bytes persist — each is a
//! decision about a host, and berm has no host. An embedder passes [`Harness`]
//! values to [`Berm::load`] and that list is the whole of what a guest can
//! reach.

use anyhow::{Context, Result, bail};
pub use manifest::{Manifest, ToolSpec};
use rvtime::{Caller, Linker, Module, Store, TypedFunc};
pub use rvtime::{Config, Engine};
use std::{collections::BTreeMap, sync::Arc};

pub mod abi;
mod manifest;
mod watchdog;
pub mod wire;

// Reached by the host expansion, and re-exported for the reason `berm-lang`
// re-exports serde: an embedder depends on this crate and nothing else, and
// cannot pick a version the generated code disagrees with.
pub use anyhow;
pub use berm_codegen::hosts;

/// A guest entry point: takes nothing, returns a pointer and a length.
type Export = TypedFunc<(), (u64, u64)>;

/// What a system harness does: request bytes in, result bytes out. An `Err`
/// reaches the guest as a failure message on the same wire as a result.
pub type Call = Arc<dyn Fn(&[u8]) -> Result<Vec<u8>> + Send + Sync>;

/// A compiled harness. Compilation is paid once per ELF; every invocation
/// gets a fresh [`Store`] so no guest state crosses between calls.
pub struct Berm {
    engine: Engine,
    module: Module,
    linker: Linker<Invocation>,
    /// Read from the ELF at load, without running anything.
    manifest: Manifest,
    /// Resolved once at load. A [`TypedFunc`] belongs to the module rather
    /// than to a store, so these stay valid for every invocation.
    tools: BTreeMap<String, Export>,
}

impl Berm {
    /// Compile `elf` and resolve its exports, granting `system`. The engine's
    /// code cache makes a second load of the same bytes cheap across processes
    /// as well as within one.
    pub fn load(engine: &Engine, elf: &[u8], system: &[Harness]) -> Result<Self> {
        let module = Module::new(engine, elf).context("failed to compile harness")?;
        let mut linker = Linker::new(engine);

        linker.func_wrap(abi::HOST_LOG, |caller: Caller<'_, Invocation>, ptr, len| {
            let bytes = caller.read(ptr, len)?;
            tracing::info!(target: "harness", "{}", String::from_utf8_lossy(bytes));
            Ok(0u64)
        })?;

        linker.func_wrap(abi::HOST_ARG_LEN, |caller: Caller<'_, Invocation>| {
            Ok(caller.data().args.len() as u64)
        })?;

        // Returns the blob's full length rather than what fit, so a guest with
        // too small a buffer can tell it was truncated instead of acting on
        // half a request.
        linker.func_wrap(
            abi::HOST_ARG_READ,
            |mut caller: Caller<'_, Invocation>, ptr, capacity| {
                let length = caller.data().args.len();
                let args = caller.data().args[..length.min(capacity as usize)].to_vec();
                caller.write(ptr, &args)?;
                Ok(length as u64)
            },
        )?;

        // Asked for on the guest's first allocation, from inside the entry it
        // is already in. Pushing these in would mean entering the guest a
        // second time, which costs ~13µs against ~30ns for a host call.
        linker.func_wrap(abi::HOST_HEAP_START, |caller: Caller<'_, Invocation>| {
            Ok(caller.heap().start)
        })?;

        linker.func_wrap(abi::HOST_HEAP_SIZE, |caller: Caller<'_, Invocation>| {
            let heap = caller.heap();
            Ok(heap.end - heap.start)
        })?;

        linker.func_wrap(
            abi::HOST_FAIL,
            |mut caller: Caller<'_, Invocation>, ptr, len| {
                let message = String::from_utf8_lossy(caller.read(ptr, len)?).into_owned();
                caller.data_mut().failure = Some(message);
                Ok(0u64)
            },
        )?;

        // The other half of every system harness call. Plumbing rather than a
        // grant: a harness given none never stages anything, so this
        // is registered unconditionally and has nothing to hand over.
        linker.func_wrap(
            abi::HOST_RESULT_READ,
            |mut caller: Caller<'_, Invocation>, ptr, capacity| {
                let length = caller.data().result.len();
                let result = caller.data().result[..length.min(capacity as usize)].to_vec();
                caller.write(ptr, &result)?;
                Ok(length as u64)
            },
        )?;

        for harness in system {
            let call = harness.call.clone();
            linker.func_wrap(
                abi::hash(&harness.name),
                move |caller: Caller<'_, Invocation>, ptr, len| {
                    let call = call.clone();
                    Invocation::stage(caller, ptr, len, move |request| call(request))
                },
            )?;
        }

        let mut store = Store::new(engine, Invocation::empty());
        let instance = linker.instantiate(&mut store, &module)?;

        let names: Vec<String> = instance
            .exports()
            .filter_map(|export| export.strip_prefix(abi::TOOL_PREFIX))
            .map(str::to_owned)
            .collect();
        if names.is_empty() {
            bail!("harness exports no tools");
        }

        let mut tools = BTreeMap::new();
        for name in names {
            let symbol = format!("{}{name}", abi::TOOL_PREFIX);
            tools.insert(name, instance.get_typed_func(&symbol)?);
        }

        // A harness that advertises a tool it does not export would fail at
        // dispatch, on a model's turn, as a missing symbol. The symbol table
        // and the manifest are both in hand here, so disagreement is caught
        // before the harness is ever offered.
        let manifest = Manifest::from_elf(elf)?;
        for tool in &manifest.tools {
            if !tools.contains_key(&tool.name) {
                bail!(
                    "harness manifest declares tool {:?}, which it does not export",
                    tool.name
                );
            }
        }

        Ok(Self {
            engine: engine.clone(),
            module,
            linker,
            manifest,
            tools,
        })
    }

    /// The tools this harness exports, as the symbol table reports them.
    pub fn tools(&self) -> impl Iterator<Item = &str> {
        self.tools.keys().map(String::as_str)
    }

    /// What the harness says it is: ABI version, tools, and usage.
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// Run one tool by name.
    ///
    /// The outer `Result` is the host's — a missing tool, a trap, a broken
    /// image. The inner one is the guest's: `Err` means the harness reported
    /// failure, which is what a tool result carries back to the model.
    pub fn call(&self, tool: &str, args: impl Into<Vec<u8>>) -> Result<Result<String, String>> {
        let Some(func) = self.tools.get(tool) else {
            bail!("harness exports no tool named {tool:?}");
        };

        let mut store = self.instantiate(args.into())?;

        // Entering the guest blocks this thread until the guest chooses to
        // return, so the bound on that has to be held by someone else. Dropped
        // on the way out of this function, before the store is.
        let _deadline = watchdog::Deadline::set(store.interrupt_handle()?);

        let (ptr, len) = func
            .call(&mut store, ())
            .with_context(|| format!("harness trapped in {tool}"))?;

        if let Some(failure) = store.data_mut().failure.take() {
            return Ok(Err(failure));
        }

        let result = store.read(ptr, len)?;
        Ok(Ok(
            String::from_utf8(result.to_vec()).context("harness returned invalid UTF-8")?
        ))
    }

    /// A store with the guest mapped into it and its heap handed over.
    fn instantiate(&self, args: Vec<u8>) -> Result<Store<Invocation>> {
        let mut store = Store::new(
            &self.engine,
            Invocation {
                args,
                result: Vec::new(),
                failure: None,
            },
        );
        self.linker.instantiate(&mut store, &self.module)?;
        Ok(store)
    }
}

/// Guest state for one invocation. Memory is per-invocation; anything a
/// harness needs to survive belongs in a storage harness, not here.
pub struct Invocation {
    args: Vec<u8>,
    /// The last system harness call's result, waiting for the guest to pull it.
    /// Staged rather than pushed because its size is not known until the work
    /// is done, and doing the work twice to measure it is not an option.
    result: Vec<u8>,
    /// Set when the guest reports failure, which is how a tool that failed is
    /// told apart from one that returned the word "error".
    failure: Option<String>,
}

impl Invocation {
    fn empty() -> Self {
        Self {
            args: Vec::new(),
            result: Vec::new(),
            failure: None,
        }
    }

    /// Run one system harness and leave its bytes for the guest to pull.
    ///
    /// Failure rides on the same return value: the [`abi::ERROR`] bit says the
    /// staged bytes are a message. One that fails therefore costs the
    /// guest nothing extra to find out about, and an empty result cannot be
    /// mistaken for one.
    pub fn stage(
        mut caller: Caller<'_, Self>,
        ptr: u64,
        len: u64,
        harness: impl FnOnce(&[u8]) -> Result<Vec<u8>>,
    ) -> Result<u64> {
        let request = caller.read(ptr, len)?.to_vec();
        let (staged, outcome) = match harness(&request) {
            Ok(result) => (result, 0),
            Err(error) => (error.to_string().into_bytes(), abi::ERROR),
        };
        let length = staged.len() as u64;
        caller.data_mut().result = staged;
        Ok(length | outcome)
    }
}

/// One thing a harness may reach.
///
/// A system harness absent from the list given to [`Berm::load`] is absent from
/// the [`Linker`], and that absence is the enforcement — there is no check to
/// write and none to forget. Whatever bounds it is whatever it closes over.
///
/// The name is hashed to the number the guest puts in `a7`, and berm neither
/// reserves a namespace nor recognises one: every name in the list belongs to
/// the embedder that wrote it.
#[derive(Clone)]
pub struct Harness {
    /// What the guest calls it, e.g. `crabtalk.fs.read`.
    pub name: String,
    /// What it does.
    pub call: Call,
}
