//! OS tools as a harness: bash, read, edit.
//!
//! These were once dispatched to whichever client was connected, so a run with
//! no client had no tools at all. As a harness they run wherever the runtime
//! does, which is the machine that owns the files (RFC 0205).
//!
//! Paths are relative to the root the harness was granted, and nothing here
//! checks that — the root is enforced host-side, so a path that escapes comes
//! back as an error rather than as an invariant this file has to maintain.
//!
//! Arguments are deserialized into structs rather than read off a
//! `serde_json::Value`. That is not a style preference: `Value` reaches the
//! sandbox's only unsupported construct, dynamic dispatch, and traps. See
//! `docs/src/rfcs/0205-berm.md`.

// `no_std` and `no_main` are the harness's shape. Off its target this is an
// ordinary binary so `cargo test` can run the tools below natively.
#![cfg_attr(target_arch = "riscv64", no_std, no_main)]

extern crate alloc;

// A read of two thousand lines is the tool's own limit and it has to fit in
// one result, so this harness needs more room than the SDK's default. The
// buffers live in `.bss` and are zeroed per invocation, which against an LLM
// round trip is not a cost worth trading a truncated read for.
#[berm_lang::harness(buffer = 262144)]
mod tools {
    use alloc::{collections::BTreeMap, string::String, vec::Vec};
    use berm_lang::{
        Failed, Out,
        call::{exec, fs},
        tool::{failed, parse, system},
    };
    use core::fmt::Write;

    /// Lines returned per read when the caller does not say.
    const DEFAULT_LIMIT: usize = 2000;

    /// Run a shell command.
    #[args(Bash)]
    pub fn bash(args: &[u8], out: &mut Out) -> Result<(), Failed> {
        let input: Bash = parse(args, out)?;

        let environment: Vec<(&str, &str)> = input
            .env
            .iter()
            .flatten()
            .map(|(key, value)| (key.as_str(), value.as_str()))
            .collect();

        // The system harness returns the JSON the model has always read for this
        // tool, so it goes back untouched rather than being parsed to be
        // printed again — which this harness could not do anyway.
        let result = system(
            exec::run(
                &input.command,
                input.cwd.as_deref().unwrap_or("."),
                &environment,
            ),
            out,
        )?;
        out.write(&result);
        Ok(())
    }

    /// Read a file with line numbers. Supports offset/limit for pagination.
    #[args(Read)]
    pub fn read(args: &[u8], out: &mut Out) -> Result<(), Failed> {
        let input: Read = parse(args, out)?;
        let content = system(fs::read(&input.path), out)?;
        let content = utf8(&content, &input.path, out)?;

        let total = content.lines().count();
        let offset = input.offset.unwrap_or(1).max(1);
        let limit = input.limit.unwrap_or(DEFAULT_LIMIT);
        let start = offset - 1;

        if start >= total {
            let _ = write!(
                out,
                "--- {total} total lines (offset {offset} is past end of file) ---"
            );
            return Ok(());
        }

        let mut shown = 0;
        for (index, line) in content.lines().skip(start).take(limit).enumerate() {
            let _ = writeln!(out, "{}\t{line}", start + index + 1);
            shown += 1;
        }

        let end = start + shown;
        if start > 0 || end < total {
            let _ = write!(
                out,
                "\n--- {total} total lines (showing lines {}-{end}) ---",
                start + 1
            );
        } else {
            let _ = write!(out, "\n--- {total} total lines ---");
        }
        Ok(())
    }

    /// Replace an exact string in a file. Fails if the string is not found or appears more than once.
    #[args(Edit)]
    pub fn edit(args: &[u8], out: &mut Out) -> Result<(), Failed> {
        let input: Edit = parse(args, out)?;

        if input.old_string.is_empty() {
            return failed("old_string must not be empty", out);
        }
        if input.old_string == input.new_string {
            return failed("old_string and new_string are identical", out);
        }

        let content = system(fs::read(&input.path), out)?;
        let content = utf8(&content, &input.path, out)?;

        // Uniqueness is what makes this safe without having read the file
        // first: naming a string that appears exactly once means already
        // knowing what is there.
        match content.matches(input.old_string.as_str()).count() {
            0 => return failed("old_string not found", out),
            1 => {}
            count => {
                let _ = write!(out, "old_string is not unique, found {count} occurrences");
                return Err(Failed);
            }
        }

        let edited = content.replacen(input.old_string.as_str(), &input.new_string, 1);
        system(fs::write(&input.path, edited.as_bytes()), out)?;
        out.write(b"ok");
        Ok(())
    }

    /// Arguments for `bash`.
    pub struct Bash {
        /// Shell command to run (e.g. `"ls -la"`, `"cat foo.txt | grep bar"`).
        pub command: String,
        /// Directory to run in, relative to the workspace root. Defaults to the root.
        pub cwd: Option<String>,
        /// Environment variables to set for the process.
        pub env: Option<BTreeMap<String, String>>,
    }

    /// Arguments for `read`.
    pub struct Read {
        /// Path to the file, relative to the workspace root.
        pub path: String,
        /// Line number to start reading from (1-based). Defaults to 1.
        pub offset: Option<usize>,
        /// Maximum number of lines to read. Defaults to 2000.
        pub limit: Option<usize>,
    }

    /// Arguments for `edit`.
    pub struct Edit {
        /// Path to the file, relative to the workspace root.
        pub path: String,
        /// Exact string to find and replace. Must appear exactly once in the file.
        pub old_string: String,
        /// Replacement string.
        pub new_string: String,
    }

    fn utf8<'a>(content: &'a [u8], path: &str, out: &mut Out) -> Result<&'a str, Failed> {
        match core::str::from_utf8(content) {
            Ok(text) => Ok(text),
            Err(_) => {
                let _ = write!(out, "{path} is not valid UTF-8");
                Err(Failed)
            }
        }
    }
}
