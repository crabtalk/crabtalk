//! SQL queries loaded from `sql/*.sql` files via `include_str!`.

pub(crate) const SCHEMA: &str = include_str!("../sql/schema.sql");
pub(crate) const TOUCH_ACCESS: &str = include_str!("../sql/touch_access.sql");
pub(crate) const SELECT_VALUE: &str = include_str!("../sql/select_value.sql");
pub(crate) const SELECT_ENTRIES: &str = include_str!("../sql/select_entries.sql");
pub(crate) const UPSERT: &str = include_str!("../sql/upsert.sql");
pub(crate) const DELETE: &str = include_str!("../sql/delete.sql");
pub(crate) const UPSERT_FULL: &str = include_str!("../sql/upsert_full.sql");
pub(crate) const SELECT_ENTRY: &str = include_str!("../sql/select_entry.sql");
pub(crate) const RECALL_FTS: &str = include_str!("../sql/recall_fts.sql");
pub(crate) const RECALL_VECTOR: &str = include_str!("../sql/recall_vector.sql");
