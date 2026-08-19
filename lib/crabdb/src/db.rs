//! The store.
//!
//! Records are appended and never edited; the newest record for a key
//! wins. A resident `BTreeMap` maps each live key to the offset of its
//! record, which is what makes a lookup one seek and a prefix scan an
//! ordered walk. The map holds offsets, not values — a 4 MB harness
//! image costs the same entry as a 4-byte posting, so residency tracks
//! how many keys exist rather than how much has been written.
//!
//! Durability: writes reach the OS immediately, so a process crash loses
//! nothing. `fsync` happens on [`CrabDb::checkpoint`] and compaction, so
//! a power loss can lose writes since the last one. Call `checkpoint`
//! when that matters.

use crate::format::{
    HEADER_LEN, Header, Key, Op, decode_index, decode_record, encode_index, encode_record,
};
use anyhow::Result;
use parking_lot::Mutex;
use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::PathBuf,
};

/// Tuning.
#[derive(Debug, Clone, Copy)]
pub struct Options {
    /// Compact once dead bytes reach this share of the file. Lower
    /// reclaims sooner and rewrites more often; the default trades one
    /// rewrite for never letting a file exceed twice its live size.
    pub compact_at: f64,
    /// Snapshot the key index during compaction, so the next open reads
    /// it instead of replaying. Off only makes sense for a store being
    /// written by something that will never reopen it.
    pub snapshot: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            compact_at: 0.5,
            snapshot: true,
        }
    }
}

/// A store: one file, and the key index over it.
///
/// One mutex, so operations serialize — reads included, since a read
/// seeks the same handle a write appends to.
pub struct CrabDb {
    inner: Mutex<Inner>,
}

struct Inner {
    path: PathBuf,
    file: File,
    index: BTreeMap<Key, u64>,
    /// Append position, and the bytes now superseded by later records.
    end: u64,
    dead: u64,
    options: Options,
}

impl CrabDb {
    /// Open with [`Options::default`].
    pub fn open(path: impl Into<PathBuf>) -> Result<Self> {
        Self::open_with(path, Options::default())
    }

    /// Open, creating the file if absent.
    ///
    /// Loads the index snapshot if the header points at one, then
    /// replays whatever was appended after it. A record torn by a crash
    /// ends the replay: everything before it is intact, and the append
    /// position is set to the last clean boundary so the next write
    /// overwrites the fragment.
    pub fn open_with(path: impl Into<PathBuf>, options: Options) -> Result<Self> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)?;

        let size = file.metadata()?.len();
        if size < HEADER_LEN {
            file.set_len(0)?;
            file.write_all(&Header::default().encode())?;
            file.sync_all()?;
            return Ok(Self {
                inner: Mutex::new(Inner {
                    path,
                    file,
                    index: BTreeMap::new(),
                    end: HEADER_LEN,
                    dead: 0,
                    options,
                }),
            });
        }

        let mut head = [0u8; HEADER_LEN as usize];
        file.read_exact(&mut head)?;
        let header = Header::decode(&head)?;

        let mut index = BTreeMap::new();
        let mut replay_from = HEADER_LEN;
        if header.index_len > 0 {
            let mut buf = vec![0u8; header.index_len as usize];
            file.seek(SeekFrom::Start(header.index_at))?;
            if file.read_exact(&mut buf).is_ok()
                && let Some(entries) = decode_index(&buf)
            {
                index.extend(entries);
                replay_from = header.index_at + header.index_len;
            } else {
                tracing::warn!("crabdb: unreadable index snapshot, replaying the log");
            }
        }

        let mut dead = 0;
        let mut end = replay_from;
        file.seek(SeekFrom::Start(replay_from))?;
        let mut tail = Vec::new();
        file.read_to_end(&mut tail)?;
        let mut at = 0usize;
        while at < tail.len() {
            let Some(record) = decode_record(&tail[at..])? else {
                tracing::warn!(
                    "crabdb: torn record at {}, discarding tail",
                    replay_from + at as u64
                );
                break;
            };
            let key = (record.col, record.key);
            let previous = match record.op {
                Op::Put => index.insert(key, replay_from + at as u64),
                Op::Delete => index.remove(&key),
            };
            if previous.is_some() {
                dead += record.len as u64;
            }
            at += record.len;
            end = replay_from + at as u64;
        }

        Ok(Self {
            inner: Mutex::new(Inner {
                path,
                file,
                index,
                end,
                dead,
                options,
            }),
        })
    }

    /// The value at `key`. One index lookup and one seek.
    pub fn get(&self, col: u8, key: &[u8]) -> Result<Option<Vec<u8>>> {
        let mut inner = self.inner.lock();
        let Some(offset) = inner.index.get(&(col, key.to_vec())).copied() else {
            return Ok(None);
        };
        inner.read_value(offset)
    }

    /// Write `value` at `key`. The superseded record is dead bytes until
    /// a compaction reclaims it, which this call may trigger.
    pub fn put(&self, col: u8, key: &[u8], value: &[u8]) -> Result<()> {
        let mut inner = self.inner.lock();
        let offset = inner.append(Op::Put, col, key, value)?;
        if let Some(stale) = inner.index.insert((col, key.to_vec()), offset) {
            inner.charge_dead(stale)?;
        }
        inner.maybe_compact()
    }

    /// `true` if the key was there. A delete is itself a record, so it
    /// survives a crash the same way a write does.
    pub fn delete(&self, col: u8, key: &[u8]) -> Result<bool> {
        let mut inner = self.inner.lock();
        let Some(stale) = inner.index.remove(&(col, key.to_vec())) else {
            return Ok(false);
        };
        inner.append(Op::Delete, col, key, &[])?;
        inner.charge_dead(stale)?;
        inner.maybe_compact()?;
        Ok(true)
    }

    /// Keys under `prefix`, ascending. Reads no values.
    pub fn scan_keys(&self, col: u8, prefix: &[u8]) -> Result<Vec<Vec<u8>>> {
        let inner = self.inner.lock();
        Ok(inner.range(col, prefix).map(|(key, _)| key).collect())
    }

    /// Keys and values under `prefix`, ascending. One seek per key.
    pub fn scan(&self, col: u8, prefix: &[u8]) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        let mut inner = self.inner.lock();
        let found: Vec<_> = inner.range(col, prefix).collect();
        let mut out = Vec::with_capacity(found.len());
        for (key, offset) in found {
            if let Some(value) = inner.read_value(offset)? {
                out.push((key, value));
            }
        }
        Ok(out)
    }

    /// Live keys, off the resident index rather than a scan.
    pub fn len(&self) -> usize {
        self.inner.lock().index.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Flush the index snapshot and fsync. Makes the next open a read
    /// rather than a replay, and is the point everything before it is
    /// durable against power loss.
    pub fn checkpoint(&self) -> Result<()> {
        let mut inner = self.inner.lock();
        inner.snapshot()
    }

    /// Rewrite the file with only live records, then snapshot.
    pub fn compact(&self) -> Result<()> {
        let mut inner = self.inner.lock();
        inner.compact()
    }
}

impl Inner {
    fn range(&self, col: u8, prefix: &[u8]) -> impl Iterator<Item = (Vec<u8>, u64)> + '_ {
        let prefix = prefix.to_vec();
        self.index
            .range((col, prefix.clone())..)
            .take_while(move |((c, key), _)| *c == col && key.starts_with(&prefix))
            .map(|((_, key), offset)| (key.clone(), *offset))
    }

    fn read_value(&mut self, offset: u64) -> Result<Option<Vec<u8>>> {
        self.file.seek(SeekFrom::Start(offset))?;
        let mut head = [0u8; 6];
        self.file.read_exact(&mut head)?;
        let key_len = u32::from_le_bytes(head[2..6].try_into()?) as i64;
        self.file.seek(SeekFrom::Current(key_len))?;
        let mut len = [0u8; 4];
        self.file.read_exact(&mut len)?;
        let mut value = vec![0u8; u32::from_le_bytes(len) as usize];
        self.file.read_exact(&mut value)?;
        Ok(Some(value))
    }

    /// Append a record and return the offset it landed at.
    fn append(&mut self, op: Op, col: u8, key: &[u8], value: &[u8]) -> Result<u64> {
        let bytes = encode_record(op, col, key, value);
        self.file.seek(SeekFrom::Start(self.end))?;
        self.file.write_all(&bytes)?;
        let offset = self.end;
        self.end += bytes.len() as u64;
        Ok(offset)
    }

    /// Count a superseded record's bytes against the file.
    fn charge_dead(&mut self, offset: u64) -> Result<()> {
        self.file.seek(SeekFrom::Start(offset))?;
        let mut head = [0u8; 6];
        if self.file.read_exact(&mut head).is_ok() {
            let key_len = u32::from_le_bytes(head[2..6].try_into()?) as u64;
            self.file.seek(SeekFrom::Current(key_len as i64))?;
            let mut len = [0u8; 4];
            if self.file.read_exact(&mut len).is_ok() {
                self.dead += 10 + key_len + u32::from_le_bytes(len) as u64;
            }
        }
        Ok(())
    }

    fn maybe_compact(&mut self) -> Result<()> {
        let live = self.end.saturating_sub(HEADER_LEN);
        if live > 0 && (self.dead as f64 / live as f64) >= self.options.compact_at {
            self.compact()?;
        }
        Ok(())
    }

    /// Append the index snapshot and point the header at it.
    fn snapshot(&mut self) -> Result<()> {
        if !self.options.snapshot {
            self.file.sync_all()?;
            return Ok(());
        }
        let entries: Vec<(u8, Vec<u8>, u64)> = self
            .index
            .iter()
            .map(|((col, key), offset)| (*col, key.clone(), *offset))
            .collect();
        let bytes = encode_index(entries.iter().map(|(c, k, o)| (*c, k.as_slice(), *o)));
        let at = self.end;
        self.file.seek(SeekFrom::Start(at))?;
        self.file.write_all(&bytes)?;
        // Appends resume after the snapshot, so the next write cannot
        // land on the bytes the header is about to name.
        self.end = at + bytes.len() as u64;
        // And the snapshot has to be on disk before anything points at
        // it, or a crash between the two leaves the header naming
        // whatever happened to be there.
        self.file.sync_all()?;

        let header = Header {
            index_at: at,
            index_len: bytes.len() as u64,
        };
        self.file.seek(SeekFrom::Start(0))?;
        self.file.write_all(&header.encode())?;
        self.file.sync_all()?;
        Ok(())
    }

    fn compact(&mut self) -> Result<()> {
        let live: Vec<(Key, u64)> = self
            .index
            .iter()
            .map(|(key, offset)| (key.clone(), *offset))
            .collect();

        let tmp = self.path.with_extension("tmp");
        let mut out = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&tmp)?;
        out.write_all(&Header::default().encode())?;

        let mut moved = BTreeMap::new();
        let mut end = HEADER_LEN;
        for ((col, key), offset) in live {
            let Some(value) = self.read_value(offset)? else {
                continue;
            };
            let bytes = encode_record(Op::Put, col, &key, &value);
            out.write_all(&bytes)?;
            moved.insert((col, key), end);
            end += bytes.len() as u64;
        }
        out.sync_all()?;

        // Rename last: until it lands, the original file is still the
        // whole truth, so a crash during compaction costs the work and
        // nothing else.
        std::fs::rename(&tmp, &self.path)?;
        if let Some(parent) = self.path.parent() {
            // The rename itself is only durable once the directory is.
            let _ = File::open(parent).and_then(|dir| dir.sync_all());
        }

        self.file = out;
        self.index = moved;
        self.end = end;
        self.dead = 0;
        self.snapshot()
    }
}
