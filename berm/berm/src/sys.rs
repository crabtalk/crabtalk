//! The host half of berm's system set: one constructor per name, taking the
//! implementation and returning the [`Harness`] that serves it.
//!
//! `berm-lang` declares the guest half of the same thing in its own crate,
//! under `harnesses!` — the crate a declaration is reached through is the side
//! it generates, so neither can be written for the other by mistake.

berm_codegen::hosts! {
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
