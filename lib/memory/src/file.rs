//! Binary file format v1.
//!
//! Layout:
//! ```text
//! [HEADER 16 bytes]
//!   magic     "CRMEM\0"  (6 bytes)
//!   version   u32 LE     (4 bytes)
//!   flags     u16 LE     (2 bytes, = 0; unknown bits rejected on read)
//!   reserved  [u8; 4]    (4 bytes, = 0)
//! [BODY]
//!   next_id      u64 LE
//!   entry_count  u32 LE
//!   entries*     (count times)
//! ```
//!
//! Per entry:
//! ```text
//!   id          u64 LE
//!   created_at  u64 LE
//!   kind        u32 LE    (0 = Note, 1 = Archive, 2 = Topic)
//!   name        u32 len LE + utf8 bytes
//!   content     u32 len LE + utf8 bytes
//!   alias_cnt   u32 LE
//!   aliases*    (u32 len + utf8 bytes, alias_cnt times)
//! ```
//!
//! `kind` is u32 rather than u8 so the fixed entry prefix stays 4-byte
//! aligned — cheap hygiene for any future on-disk index work.
//!
//! The inverted index is not persisted; it is rebuilt from entries on
//! load. Keeps the file small and the format boring.

use crate::{
    entry::{Entry, EntryId, EntryKind},
    error::{Error, Result},
};
use std::{
    ffi::OsString,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

const MAGIC: &[u8; 6] = b"CRMEM\0";
// Bump when a new kind ships post-1.0 so older binaries refuse files
// they can't interpret instead of crashing on "unknown entry kind".
const VERSION: u32 = 1;
const HEADER_LEN: usize = 16;
const KIND_NOTE: u32 = 0;
const KIND_ARCHIVE: u32 = 1;
const KIND_TOPIC: u32 = 2;

pub(crate) struct Snapshot {
    pub(crate) next_id: EntryId,
    pub(crate) entries: Vec<Entry>,
}

/// Read a memory file. Returns `None` if the file does not exist.
pub(crate) fn read(path: &Path) -> Result<Option<Snapshot>> {
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(Error::Io(e)),
    };
    if bytes.len() < HEADER_LEN {
        return Err(Error::BadFormat("file shorter than header"));
    }
    if &bytes[0..6] != MAGIC {
        return Err(Error::BadFormat("invalid magic"));
    }
    let version = u32::from_le_bytes(bytes[6..10].try_into().unwrap());
    if version != VERSION {
        return Err(Error::BadFormat("unsupported version"));
    }
    let flags = u16::from_le_bytes(bytes[10..12].try_into().unwrap());
    if flags != 0 {
        return Err(Error::BadFormat("unknown flags"));
    }

    let mut cur = Cursor::new(&bytes[HEADER_LEN..]);
    let next_id = cur.read_u64()?;
    let count = cur.read_u32()? as usize;
    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        entries.push(cur.read_entry()?);
    }
    Ok(Some(Snapshot { next_id, entries }))
}

/// Write a memory file atomically: encode to a sibling temp file, fsync
/// it, rename, then fsync the parent directory so the rename is durable.
pub(crate) fn write(path: &Path, next_id: EntryId, entries: &[&Entry]) -> Result<()> {
    let entry_count = u32_from_len(entries.len(), "too many entries")?;
    let mut buf = Vec::with_capacity(256 + entries.len() * 128);
    buf.extend_from_slice(MAGIC);
    buf.extend_from_slice(&VERSION.to_le_bytes());
    buf.extend_from_slice(&[0u8; 6]); // flags + reserved
    buf.extend_from_slice(&next_id.to_le_bytes());
    buf.extend_from_slice(&entry_count.to_le_bytes());
    for e in entries {
        encode_entry(&mut buf, e)?;
    }

    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }
    let tmp = tmp_path(path);
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(&buf)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::File::open(parent)?.sync_all()?;
    }
    Ok(())
}

fn tmp_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().map(OsString::from).unwrap_or_default();
    name.push(".tmp");
    path.with_file_name(name)
}

fn encode_entry(buf: &mut Vec<u8>, e: &Entry) -> Result<()> {
    buf.extend_from_slice(&e.id.to_le_bytes());
    buf.extend_from_slice(&e.created_at.to_le_bytes());
    let kind = match e.kind {
        EntryKind::Note => KIND_NOTE,
        EntryKind::Archive => KIND_ARCHIVE,
        EntryKind::Topic => KIND_TOPIC,
    };
    buf.extend_from_slice(&kind.to_le_bytes());
    encode_string(buf, &e.name)?;
    encode_string(buf, &e.content)?;
    let alias_cnt = u32_from_len(e.aliases.len(), "too many aliases")?;
    buf.extend_from_slice(&alias_cnt.to_le_bytes());
    for a in &e.aliases {
        encode_string(buf, a)?;
    }
    Ok(())
}

fn encode_string(buf: &mut Vec<u8>, s: &str) -> Result<()> {
    let len = u32_from_len(s.len(), "string exceeds 4GiB")?;
    buf.extend_from_slice(&len.to_le_bytes());
    buf.extend_from_slice(s.as_bytes());
    Ok(())
}

fn u32_from_len(n: usize, msg: &'static str) -> Result<u32> {
    u32::try_from(n).map_err(|_| Error::BadFormat(msg))
}

struct Cursor<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        if self.pos + n > self.buf.len() {
            return Err(Error::BadFormat("truncated body"));
        }
        let slice = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(slice)
    }

    fn read_u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn read_u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }

    fn read_string(&mut self) -> Result<String> {
        let len = self.read_u32()? as usize;
        let bytes = self.take(len)?.to_vec();
        String::from_utf8(bytes).map_err(|_| Error::BadFormat("invalid utf8"))
    }

    fn read_entry(&mut self) -> Result<Entry> {
        let id = self.read_u64()?;
        let created_at = self.read_u64()?;
        let kind = match self.read_u32()? {
            KIND_NOTE => EntryKind::Note,
            KIND_ARCHIVE => EntryKind::Archive,
            KIND_TOPIC => EntryKind::Topic,
            _ => return Err(Error::BadFormat("unknown entry kind")),
        };
        let name = self.read_string()?;
        let content = self.read_string()?;
        let alias_cnt = self.read_u32()? as usize;
        let mut aliases = Vec::with_capacity(alias_cnt);
        for _ in 0..alias_cnt {
            aliases.push(self.read_string()?);
        }
        Ok(Entry {
            id,
            created_at,
            kind,
            name,
            content,
            aliases,
        })
    }
}
