pub type EntryId = u64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EntryKind {
    Note,
    Archive,
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub id: EntryId,
    pub name: String,
    pub content: String,
    pub aliases: Vec<String>,
    pub created_at: u64,
    pub kind: EntryKind,
}
