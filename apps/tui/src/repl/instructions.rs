//! Project instructions, discovered from the user's working directory.
//!
//! This is the one thing about the local filesystem that stays the client's
//! job after OS tools moved into the runtime. `SendMsg` dropped its `cwd`
//! field for the reason recorded there — the daemon does not read the user's
//! filesystem to decide what to say — so the client reads `Crab.md` itself and
//! renders it into the message it sends.

use std::path::Path;

/// Walk up from `cwd` collecting `Crab.md` files, plus the global one in the
/// user's config dir. Returned text is the concatenation in deepest-first
/// order, so project-local rules layer over global ones.
pub fn discover(cwd: &Path) -> Option<String> {
    let config_dir = &*wcore::paths::CONFIG_DIR;
    let mut layers = Vec::new();

    let global = config_dir.join("Crab.md");
    if let Ok(content) = std::fs::read_to_string(&global) {
        layers.push(content);
    }

    let mut found = Vec::new();
    let mut dir = cwd;
    loop {
        let candidate = dir.join("Crab.md");
        if candidate.is_file()
            && !candidate.starts_with(config_dir)
            && let Ok(content) = std::fs::read_to_string(&candidate)
        {
            found.push(content);
        }
        match dir.parent() {
            Some(p) => dir = p,
            None => break,
        }
    }
    found.reverse();
    layers.extend(found);

    if layers.is_empty() {
        return None;
    }
    Some(layers.join("\n\n"))
}
