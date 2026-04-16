use crate::entry::EntryKind;

/// Write operations. `Update` rewrites content and aliases but preserves
/// `kind` — an archive stays an archive for life. Use `Remove` + `Add` to
/// change kind.
#[derive(Clone, Debug)]
pub enum Op {
    Add {
        name: String,
        content: String,
        aliases: Vec<String>,
        kind: EntryKind,
    },
    Update {
        name: String,
        content: String,
        aliases: Vec<String>,
    },
    Alias {
        name: String,
        aliases: Vec<String>,
    },
    Remove {
        name: String,
    },
}
