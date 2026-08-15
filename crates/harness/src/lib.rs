//! Harness host — loads a guest ELF and runs it under rvtime.
//!
//! Spike scope (RFC 0205): compile once, instantiate per invocation, pull
//! arguments in through host calls, read the result back out of guest memory.
//! Capabilities beyond logging and argument transfer are not wired yet.

use anyhow::{Context, Result, bail};
use object::{Object, ObjectSection};
use rvtime::{Caller, Engine, Linker, Module, Store, TypedFunc};
use std::collections::BTreeMap;

pub mod abi;

/// A guest entry point: takes nothing, returns a pointer and a length.
type Export = TypedFunc<(), (u64, u64)>;

/// Guest state for one invocation. Memory is per-invocation; anything a
/// harness needs to survive belongs in a storage capability, not here.
pub struct Invocation {
    args: Vec<u8>,
    /// Set when the guest reports failure, which is how a tool that failed is
    /// told apart from one that returned the word "error".
    failure: Option<String>,
}

/// A compiled harness. Compilation is paid once per ELF; every invocation
/// gets a fresh [`Store`] so no guest state crosses between calls.
pub struct Harness {
    engine: Engine,
    module: Module,
    linker: Linker<Invocation>,
    /// Read from the ELF at load, without running anything.
    manifest: String,
    /// Resolved once at load. A [`TypedFunc`] belongs to the module rather
    /// than to a store, so these stay valid for every invocation.
    tools: BTreeMap<String, Export>,
}

impl Harness {
    /// Compile `elf` and resolve its exports. The engine's code cache makes a
    /// second load of the same bytes cheap across processes as well as within
    /// one.
    pub fn load(engine: &Engine, elf: &[u8]) -> Result<Self> {
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

        Ok(Self {
            engine: engine.clone(),
            module,
            linker,
            manifest: manifest(elf)?,
            tools,
        })
    }

    /// The tools this harness exports, as the symbol table reports them.
    pub fn tools(&self) -> impl Iterator<Item = &str> {
        self.tools.keys().map(String::as_str)
    }

    /// What the harness says it is: ABI version, tools, capabilities wanted.
    pub fn manifest(&self) -> &str {
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
        let (ptr, len) = func
            .call(&mut store, ())
            .with_context(|| format!("harness trapped in {tool}"))?;

        if let Some(failure) = store.data_mut().failure.take() {
            return Ok(Err(failure));
        }
        Ok(Ok(read(&store, ptr, len)?))
    }

    /// A store with the guest mapped into it and its heap handed over.
    fn instantiate(&self, args: Vec<u8>) -> Result<Store<Invocation>> {
        let mut store = Store::new(
            &self.engine,
            Invocation {
                args,
                failure: None,
            },
        );
        self.linker.instantiate(&mut store, &self.module)?;
        Ok(store)
    }
}

impl Invocation {
    fn empty() -> Self {
        Self {
            args: Vec::new(),
            failure: None,
        }
    }
}

/// Pull the manifest out of the ELF. This runs before anything is compiled,
/// let alone executed — a harness gets to describe itself without being given
/// a turn.
fn manifest(elf: &[u8]) -> Result<String> {
    let file = object::File::parse(elf).context("harness is not a readable ELF")?;
    let section = file
        .section_by_name(abi::ABI_SECTION)
        .with_context(|| format!("harness has no {} section", abi::ABI_SECTION))?;
    let bytes = section.data().context("harness manifest is unreadable")?;
    String::from_utf8(bytes.to_vec()).context("harness manifest is not UTF-8")
}

fn read(store: &Store<Invocation>, ptr: u64, len: u64) -> Result<String> {
    let bytes = store.read(ptr, len)?;
    String::from_utf8(bytes.to_vec()).context("harness returned invalid UTF-8")
}
