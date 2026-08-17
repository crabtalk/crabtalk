//! The CRMEM on-disk format.
//!
//! All integers little-endian, all strings length-prefixed by a `u32`
//! byte count. The file is a fixed header, then appended records, then —
//! wherever it last fit — a snapshot of the key index the header points
//! at. Nothing is aligned and nothing is padded; the format is meant to
//! be readable with `xxd` and boring enough to never need a migration.

use anyhow::{Result, bail};

/// A key, qualified by the column that owns it. Column first so the
/// map's ordering groups by column before key, which is what lets a
/// prefix scan be one range rather than a filter over everything.
pub type Key = (u8, Vec<u8>);

/// `CRMEM\0`.
pub const MAGIC: [u8; 6] = *b"CRMEM\0";
pub const VERSION: u32 = 1;

/// Fixed, so it can be rewritten in place when the snapshot moves.
pub const HEADER_LEN: u64 = 32;

/// What a record does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Put = 0,
    Delete = 1,
}

impl Op {
    fn from_byte(b: u8) -> Option<Self> {
        match b {
            0 => Some(Op::Put),
            1 => Some(Op::Delete),
            _ => None,
        }
    }
}

/// Where the index snapshot lives. Zeroes mean there is none, and the
/// whole log has to be replayed.
#[derive(Debug, Default, Clone, Copy)]
pub struct Header {
    pub index_at: u64,
    pub index_len: u64,
}

impl Header {
    pub fn encode(&self) -> [u8; HEADER_LEN as usize] {
        let mut out = [0u8; HEADER_LEN as usize];
        out[0..6].copy_from_slice(&MAGIC);
        out[6..10].copy_from_slice(&VERSION.to_le_bytes());
        // flags u16 and 4 reserved bytes stay zero.
        out[16..24].copy_from_slice(&self.index_at.to_le_bytes());
        out[24..32].copy_from_slice(&self.index_len.to_le_bytes());
        out
    }

    /// Reject anything we do not understand rather than guessing: a
    /// wrong magic is someone else's file, and an unknown flag is a
    /// writer that knew something this reader does not.
    pub fn decode(buf: &[u8]) -> Result<Self> {
        if buf.len() < HEADER_LEN as usize {
            bail!("crabdb: file shorter than its header");
        }
        if buf[0..6] != MAGIC {
            bail!("crabdb: not a CRMEM file");
        }
        let version = u32::from_le_bytes(buf[6..10].try_into()?);
        if version != VERSION {
            bail!("crabdb: unsupported format version {version}");
        }
        let flags = u16::from_le_bytes(buf[10..12].try_into()?);
        if flags != 0 {
            bail!("crabdb: unknown flags {flags:#06x}");
        }
        Ok(Self {
            index_at: u64::from_le_bytes(buf[16..24].try_into()?),
            index_len: u64::from_le_bytes(buf[24..32].try_into()?),
        })
    }
}

/// `op | col | key_len | key | val_len | value`.
pub fn encode_record(op: Op, col: u8, key: &[u8], value: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(10 + key.len() + value.len());
    out.push(op as u8);
    out.push(col);
    out.extend_from_slice(&(key.len() as u32).to_le_bytes());
    out.extend_from_slice(key);
    out.extend_from_slice(&(value.len() as u32).to_le_bytes());
    out.extend_from_slice(value);
    out
}

/// One decoded record and how many bytes it occupied.
pub struct Record {
    pub op: Op,
    pub col: u8,
    pub key: Vec<u8>,
    pub value: Vec<u8>,
    pub len: usize,
}

/// Decode the record at the start of `buf`.
///
/// `Ok(None)` means the buffer ends mid-record — a tail torn by a crash,
/// which the caller discards rather than treats as corruption.
pub fn decode_record(buf: &[u8]) -> Result<Option<Record>> {
    let Some(op) = buf.first().copied() else {
        return Ok(None);
    };
    let Some(op) = Op::from_byte(op) else {
        bail!("crabdb: unknown record op {op}");
    };
    let mut at = 2;
    let Some(key_len) = take_u32(buf, &mut at) else {
        return Ok(None);
    };
    let Some(key) = take(buf, &mut at, key_len as usize) else {
        return Ok(None);
    };
    let Some(val_len) = take_u32(buf, &mut at) else {
        return Ok(None);
    };
    let Some(value) = take(buf, &mut at, val_len as usize) else {
        return Ok(None);
    };
    Ok(Some(Record {
        op,
        col: buf[1],
        key,
        value,
        len: at,
    }))
}

/// `count | repeated { col | key_len | key | offset }`.
pub fn encode_index<'a>(entries: impl ExactSizeIterator<Item = (u8, &'a [u8], u64)>) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    for (col, key, offset) in entries {
        out.push(col);
        out.extend_from_slice(&(key.len() as u32).to_le_bytes());
        out.extend_from_slice(key);
        out.extend_from_slice(&offset.to_le_bytes());
    }
    out
}

/// Decode a snapshot. A truncated one yields `None` so the caller can
/// fall back to replaying the whole log.
pub fn decode_index(buf: &[u8]) -> Option<Vec<(Key, u64)>> {
    let mut at = 0;
    let count = take_u32(buf, &mut at)?;
    let mut out = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let col = *buf.get(at)?;
        at += 1;
        let key_len = take_u32(buf, &mut at)?;
        let key = take(buf, &mut at, key_len as usize)?;
        let offset = u64::from_le_bytes(take(buf, &mut at, 8usize)?.try_into().ok()?);
        out.push(((col, key), offset));
    }
    Some(out)
}

fn take_u32(buf: &[u8], at: &mut usize) -> Option<u32> {
    let bytes = take(buf, at, 4)?;
    Some(u32::from_le_bytes(bytes.try_into().ok()?))
}

fn take(buf: &[u8], at: &mut usize, len: usize) -> Option<Vec<u8>> {
    let end = at.checked_add(len)?;
    let slice = buf.get(*at..end)?.to_vec();
    *at = end;
    Some(slice)
}
