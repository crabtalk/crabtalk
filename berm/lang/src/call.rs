#![cfg(feature = "alloc")]

use berm_codegen::harnesses;

// The guest half of berm's system set. berm declares the host half of the same
// thing, in its own crate, under `host!`.
//
// Drift is caught rather than prevented. A renamed call hashes to a number
// nothing is registered for, and a changed field count fails the arity check
// the host expansion emits — both loud, on the first call, which
// `cargo run --example os -p berm` makes.
harnesses! {
    namespace = "berm";

    /// Files, bounded by a granted root.
    mod fs {
        /// Read a file whole.
        fn read(path: &str) -> Vec<u8>;
        /// Write a file, replacing what was there.
        fn write(path: &str, content: &[u8]);
    }

    /// Commands, under the same root `fs` is bounded by.
    mod exec {
        /// Run a command through a shell, in `cwd` relative to the root.
        fn run(command: &str, cwd: &str, env: &[(&str, &str)]) -> Vec<u8>;
    }
}
