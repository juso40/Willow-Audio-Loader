//! The Wwise file package (`.pck`) container as BL2 ships it.
//!
//! Layout, verified field by field against `WillowGame/CookedPCConsole/
//! Audio_Banks.pck` (57 banks), `Audio_Streaming.pck` (550 streams) and
//! `DLC/Anemone/Lic/Audio/Audio_Banks_anemone.pck` (18 banks). Entry
//! offsets land on `BKHD` / `RIFF` payloads in all three:
//!
//! ```text
//! u8[4] magic = "AKPK"
//! u32   header_size          // total header size minus these 8 bytes
//! u32   version              // 1
//! u32   language_table_size  // includes count, offset/id pairs, UTF-16 names
//! u32   banks_table_size     // u32 count + count * entry_size
//! u32   streams_table_size   // u32 count + count * entry_size
//! u32   externals_table_size // always 4 (empty) in every observed pck
//! [languages][banks table][streams table][externals table]
//! languages: u32 count, {u32 name_offset, u32 language_id}[count], then
//! NUL-terminated UTF-16LE names. Offsets are relative to the language block.
//! u8    data[]               // entry payloads, contiguous, offsets absolute
//! ```
//!
//! Each entry is `{id, flags, size, offset, language_id}` (5 × u32 LE).
//! Bank entry ids are **bank** ids. Stream entry ids are **media** ids,
//! the same ids the engine's loose-file resolver turns into `%u.bnk` /
//! `%u.wem`. Read stream entries directly and
//! bank-embedded media through `DIDX` (module [`crate::bank`]).
//!
//! This module only reads archives. Craft new banks in memory through
//! [`crate::craft`] and load them through [`crate::wwise`] instead.

use std::path::Path;

use crate::error::{AudioError, Result};

/// Container magic.
pub const MAGIC: [u8; 4] = *b"AKPK";
/// Entry size in BL2 archives (5 × u32).
pub const ENTRY_SIZE: usize = 20;
/// Fixed header before the language block (count belongs to that block).
const FIXED_HEADER: usize = 0x1c;

/// Which payload table an entry lives in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableKind {
    /// Soundbanks (`.bnk` payloads). Entry ids are bank ids.
    Banks,
    /// Streamed media (`.wem` payloads). Entry ids are media ids.
    Streams,
    /// External sources. Always empty in every observed pck.
    Externals,
}

impl TableKind {
    /// The spelling used in probe output.
    pub fn label(self) -> &'static str {
        match self {
            Self::Banks => "banks",
            Self::Streams => "streams",
            Self::Externals => "externals",
        }
    }
}

/// One 20-byte table entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PckEntry {
    /// Bank id (banks table) or media id (streams table).
    pub id: u32,
    /// `1` in every observed entry. Preserved verbatim.
    pub flags: u32,
    /// Payload size in bytes.
    pub size: u32,
    /// Absolute file offset of the payload.
    pub offset: u32,
    /// Language id (0 for SFX, 1 for English in the observed localized pack).
    pub language_id: u32,
}

/// One language-table entry, e.g. `sfx` or `english(us)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Language {
    /// Language id (`0` for `sfx`).
    pub id: u32,
    /// UTF-16 name.
    pub name: String,
}

/// A read-only `.pck`. The parser retains the original bytes for payload access.
#[derive(Debug)]
pub struct Pck {
    version: u32,
    /// The archive's language table.
    pub languages: Vec<Language>,
    /// Soundbank entries.
    pub banks: Vec<PckEntry>,
    /// Streamed-media entries.
    pub streams: Vec<PckEntry>,
    /// External-source entries (always empty in observed pcks).
    pub externals: Vec<PckEntry>,
    data: Vec<u8>,
}

impl Pck {
    /// Reads and parses the archive at `path`.
    pub fn open(path: &Path) -> Result<Self> {
        let data = std::fs::read(path).map_err(|source| AudioError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        Self::parse(data)
    }

    /// Parses archive bytes (retained as backing storage for [`Self::blob`]).
    pub fn parse(data: Vec<u8>) -> Result<Self> {
        if data.len() < FIXED_HEADER {
            return Err(AudioError::format(format!(
                "the data ends inside the pck header ({} bytes, need {FIXED_HEADER})",
                data.len()
            )));
        }
        if data[..4] != MAGIC {
            return Err(AudioError::format(format!(
                "not a pck: magic {:02x?} (expected AKPK)",
                &data[..4]
            )));
        }
        let header_size = get_u32(&data, 0x04)? as usize;
        let version = get_u32(&data, 0x08)?;
        let language_size = get_u32(&data, 0x0c)? as usize;
        let banks_size = get_u32(&data, 0x10)? as usize;
        let streams_size = get_u32(&data, 0x14)? as usize;
        let externals_size = get_u32(&data, 0x18)? as usize;

        let tables_end = FIXED_HEADER
            .checked_add(language_size)
            .and_then(|pos| pos.checked_add(banks_size))
            .and_then(|pos| pos.checked_add(streams_size))
            .and_then(|pos| pos.checked_add(externals_size))
            .ok_or_else(|| AudioError::format("pck header sizes overflow"))?;
        if header_size.checked_add(8) != Some(tables_end) {
            return Err(AudioError::format(format!(
                "pck header_size {header_size:#x} disagrees with table sizes (expected {:#x})",
                tables_end - 8
            )));
        }

        let (languages, pos) = parse_languages(&data, FIXED_HEADER, language_size)?;
        let (banks, pos) = parse_table(&data, pos, banks_size)?;
        let (streams, pos) = parse_table(&data, pos, streams_size)?;
        let (externals, _) = parse_table(&data, pos, externals_size)?;

        let package = Self {
            version,
            languages,
            banks,
            streams,
            externals,
            data,
        };
        for kind in [TableKind::Banks, TableKind::Streams, TableKind::Externals] {
            for index in 0..package.entries(kind).len() {
                package.blob(kind, index)?;
            }
        }
        Ok(package)
    }

    /// The archive version (`1` in every observed pck).
    pub fn version(&self) -> u32 {
        self.version
    }

    /// Total size of the parsed file bytes.
    pub fn data_len(&self) -> usize {
        self.data.len()
    }

    fn entries(&self, kind: TableKind) -> &[PckEntry] {
        match kind {
            TableKind::Banks => &self.banks,
            TableKind::Streams => &self.streams,
            TableKind::Externals => &self.externals,
        }
    }

    /// The original payload bytes of one entry.
    pub fn blob(&self, kind: TableKind, index: usize) -> Result<&[u8]> {
        let entry = self
            .entries(kind)
            .get(index)
            .ok_or_else(|| AudioError::format(format!("no entry #{index} in {}", kind.label())))?;
        let start = entry.offset as usize;
        let end = start.checked_add(entry.size as usize).ok_or_else(|| {
            AudioError::format(format!("entry 0x{:08x} payload range overflows", entry.id))
        })?;
        self.data.get(start..end).ok_or_else(|| {
            AudioError::format(format!(
                "entry 0x{:08x} payload {start:#x}..{end:#x} is out of bounds",
                entry.id
            ))
        })
    }

    /// Index of the streams-table entry with this media id, if any.
    /// (Bank entry ids are a different id space and are never matched.)
    pub fn find_stream(&self, media_id: u32) -> Option<usize> {
        self.streams.iter().position(|entry| entry.id == media_id)
    }

    /// Every addressable media payload: stream entries by media id and
    /// bank-embedded media through `DIDX`. Returns `(media_id, size)`
    /// pairs, streams first.
    pub fn list_media(&self) -> Vec<(u32, u32)> {
        let mut out: Vec<(u32, u32)> = self
            .streams
            .iter()
            .map(|entry| (entry.id, entry.size))
            .collect();
        for index in 0..self.banks.len() {
            let Ok(blob) = self.blob(TableKind::Banks, index) else {
                continue;
            };
            let Ok(entries) = crate::bank::media_index(blob) else {
                continue;
            };
            out.extend(
                entries
                    .into_iter()
                    .map(|entry| (entry.media_id, entry.size)),
            );
        }
        out
    }

    /// Reads one media payload: a stream by media id, or bank-embedded
    /// media through `DIDX`.
    pub fn extract_media(&self, media_id: u32) -> Result<Vec<u8>> {
        if let Some(index) = self.find_stream(media_id) {
            return Ok(self.blob(TableKind::Streams, index)?.to_vec());
        }
        for index in 0..self.banks.len() {
            let blob = self.blob(TableKind::Banks, index)?;
            if crate::bank::has_media(blob, media_id) {
                return Ok(crate::bank::embedded_media(blob, media_id)?.to_vec());
            }
        }
        Err(AudioError::format(format!(
            "media 0x{media_id:08x} not found in the archive"
        )))
    }
}

fn get_u32(data: &[u8], pos: usize) -> Result<u32> {
    let bytes = data
        .get(pos..)
        .and_then(|tail| tail.get(..4))
        .ok_or_else(|| {
            AudioError::format(format!("field at {pos:#x} runs past the end of the data"))
        })?;
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn parse_languages(data: &[u8], start: usize, size: usize) -> Result<(Vec<Language>, usize)> {
    let end = start
        .checked_add(size)
        .ok_or_else(|| AudioError::format("language size overflow"))?;
    let table = data
        .get(start..end)
        .ok_or_else(|| AudioError::format("language table out of bounds"))?;
    let count = get_u32(table, 0)? as usize;
    if count > size.saturating_sub(4) / 8 {
        return Err(AudioError::format("language directory out of bounds"));
    }
    let mut languages = Vec::with_capacity(count);
    for index in 0..count {
        let mut pos = get_u32(table, 4 + index * 8)? as usize;
        let id = get_u32(table, 8 + index * 8)?;
        if pos < 4 + count * 8 || !pos.is_multiple_of(2) {
            return Err(AudioError::format("invalid language name offset"));
        }
        let mut units = Vec::new();
        loop {
            let unit = table
                .get(pos..)
                .and_then(|tail| tail.get(..2))
                .ok_or_else(|| {
                    AudioError::format("language name runs past the end of the table")
                })?;
            pos += 2;
            let unit = u16::from_le_bytes([unit[0], unit[1]]);
            if unit == 0 {
                break;
            }
            units.push(unit);
        }
        let name = String::from_utf16(&units)
            .map_err(|_| AudioError::format("language name is not valid UTF-16"))?;
        languages.push(Language { id, name });
    }
    Ok((languages, end))
}

fn parse_table(data: &[u8], start: usize, size: usize) -> Result<(Vec<PckEntry>, usize)> {
    let end = start
        .checked_add(size)
        .ok_or_else(|| AudioError::format("table size overflow"))?;
    if end > data.len() {
        return Err(AudioError::format("archive table out of bounds"));
    }
    if size < 4 {
        return Err(AudioError::format(format!(
            "table at {start:#x} is smaller than its count field"
        )));
    }
    let count = get_u32(data, start)? as usize;
    if count.checked_mul(ENTRY_SIZE).and_then(|n| n.checked_add(4)) != Some(size) {
        return Err(AudioError::format(format!(
            "table at {start:#x}: {count} entries of {ENTRY_SIZE} bytes disagree with size {size}"
        )));
    }
    let mut entries = Vec::with_capacity(count);
    for index in 0..count {
        let pos = start + 4 + index * ENTRY_SIZE;
        entries.push(PckEntry {
            id: get_u32(data, pos)?,
            flags: get_u32(data, pos + 4)?,
            size: get_u32(data, pos + 8)?,
            offset: get_u32(data, pos + 12)?,
            language_id: get_u32(data, pos + 16)?,
        });
    }
    Ok((entries, end))
}
