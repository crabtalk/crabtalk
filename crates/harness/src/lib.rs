//! Harness host — loads a guest ELF and runs it under rvtime.
//!
//! Spike scope (RFC 0205): compile once, instantiate per invocation, pull
//! arguments in through host calls, read the result back out of guest memory.
//! Capabilities beyond logging and argument transfer are not wired yet.

use anyhow::{Context, Result};
use rvtime::{Caller, Engine, Instance, Linker, Module, Store};

pub mod abi;

/// Guest state for one invocation. Memory is per-invocation; anything a
/// harness needs to survive belongs in a storage capability, not here.
pub struct Invocation {
    args: Vec<u8>,
}

/// A compiled harness. Compilation is paid once per ELF; every invocation
/// gets a fresh [`Store`] so no guest state crosses between calls.
pub struct Harness {
    engine: Engine,
    module: Module,
    linker: Linker<Invocation>,
}

impl Harness {
    /// Compile `elf`. The engine's code cache makes a second load of the
    /// same bytes cheap across processes as well as within one.
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

        linker.func_wrap(
            abi::HOST_ARG_READ,
            |mut caller: Caller<'_, Invocation>, ptr, cap| {
                let n = (cap as usize).min(caller.data().args.len());
                let args = caller.data().args[..n].to_vec();
                caller.write(ptr, &args)?;
                Ok(n as u64)
            },
        )?;

        Ok(Self {
            engine: engine.clone(),
            module,
            linker,
        })
    }

    /// The harness's self-description: ABI version, tools, capabilities wanted.
    pub fn describe(&self) -> Result<String> {
        self.invoke(abi::EXPORT_DESCRIBE, Vec::new())
    }

    /// Run one invocation with `args` as its argument blob.
    pub fn call(&self, args: impl Into<Vec<u8>>) -> Result<String> {
        self.invoke(abi::EXPORT_CALL, args.into())
    }

    fn invoke(&self, export: &str, args: Vec<u8>) -> Result<String> {
        let mut store = Store::new(&self.engine, Invocation { args });
        let instance = self.linker.instantiate(&mut store, &self.module)?;
        self.run(&instance, &mut store, export)
    }

    fn run(
        &self,
        instance: &Instance,
        store: &mut Store<Invocation>,
        export: &str,
    ) -> Result<String> {
        let func = instance.get_typed_func::<(), (u64, u64)>(export)?;
        let (ptr, len) = func
            .call(store, ())
            .with_context(|| format!("harness trapped in {export}"))?;
        let bytes = store.read(ptr, len)?;
        String::from_utf8(bytes.to_vec()).context("harness returned invalid UTF-8")
    }
}
