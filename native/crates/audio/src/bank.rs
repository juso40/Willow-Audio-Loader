//! Read a Wwise soundbank (`.bnk`) as its chunk list and embedded media.
//!
//! Chunk framing:
//! each chunk is `u8[4] id + u32 payload_size`
//! followed by `payload_size` bytes, back to back with **no padding**
//! (chunk sizes are not 4-aligned, so `STID` follows a 77-byte `HIRC` immediately).
//!
//! Observed chunks and payloads:
//!
//! - `BKHD`: 16 to 28 bytes. The first two u32s are the bank format
//!   version (62 to 70 across banks) and the bank id (matching the `.pck`
//!   entry id). The parser preserves the rest verbatim.
//! - `DIDX`: the embedded-media index, 12-byte entries
//!   `{media_id u32, offset u32, size u32}` with offsets into the `DATA`
//!   payload.
//! - `DATA`: the concatenated media payloads (`.wem`-shaped or raw codec
//!   data, depending on how the sound author encoded it).
//! - `HIRC`: the event/action hierarchy, opaque to us.
//! - `STID`: the bank's string table (name entries).
//!
//! This parser treats unknown chunks and the event/action hierarchy
//! (`HIRC`) as opaque payloads. Creating banks with new events is handled
//! separately by [`crate::craft`].

use crate::error::{AudioError, Result};

/// One bank chunk: the 4CC and its payload (framing bytes excluded).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chunk<'a> {
    /// The chunk's 4CC (`BKHD`, `DIDX`, `DATA`, ...).
    pub id: [u8; 4],
    /// The chunk payload, excluding the 8 framing bytes.
    pub payload: &'a [u8],
}

/// The interpreted head of a `BKHD` chunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BankHeader {
    /// Bank format version (`62` in every observed bank).
    pub version: u32,
    /// The bank id (matches the `.pck` entry id).
    pub id: u32,
}

/// One `DIDX` entry: embedded media located inside the `DATA` payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MediaEntry {
    /// The media id (`%u.wem` of the engine's loose-file resolver).
    pub media_id: u32,
    /// Offset inside the `DATA` payload.
    pub offset: u32,
    /// Payload size in bytes.
    pub size: u32,
}

/// Walks the chunk list. Fails when a chunk overruns the buffer.
pub fn chunks(data: &[u8]) -> Result<Vec<Chunk<'_>>> {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos < data.len() {
        if data.len() - pos < 8 {
            return Err(AudioError::format(format!(
                "chunk header at {pos:#x} runs past the end of the data"
            )));
        }
        let mut id = [0u8; 4];
        id.copy_from_slice(&data[pos..pos + 4]);
        let size = u32::from_le_bytes([data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7]])
            as usize;
        let payload = data.get(pos + 8..pos + 8 + size).ok_or_else(|| {
            AudioError::format(format!(
                "chunk {:?} at {pos:#x} claims {size} bytes, overruns the file",
                String::from_utf8_lossy(&id)
            ))
        })?;
        out.push(Chunk { id, payload });
        // Chunks are back to back (sizes are not 4-aligned).
        pos += 8 + size;
    }
    Ok(out)
}

/// Reads the `BKHD` head (version + bank id).
pub fn header(data: &[u8]) -> Result<BankHeader> {
    let chunks = chunks(data)?;
    let bkhd = chunks
        .iter()
        .find(|chunk| chunk.id == *b"BKHD")
        .ok_or_else(|| AudioError::format("no BKHD chunk in bank"))?;
    if bkhd.payload.len() < 8 {
        return Err(AudioError::format(format!(
            "BKHD chunk is {} bytes (need at least 8)",
            bkhd.payload.len()
        )));
    }
    let word = |pos: usize| {
        u32::from_le_bytes([
            bkhd.payload[pos],
            bkhd.payload[pos + 1],
            bkhd.payload[pos + 2],
            bkhd.payload[pos + 3],
        ])
    };
    Ok(BankHeader {
        version: word(0),
        id: word(4),
    })
}

/// Reads the `DIDX` media index (empty when the bank embeds no media).
pub fn media_index(data: &[u8]) -> Result<Vec<MediaEntry>> {
    let chunks = chunks(data)?;
    let Some(didx) = chunks.iter().find(|chunk| chunk.id == *b"DIDX") else {
        return Ok(Vec::new());
    };
    if didx.payload.len() % 12 != 0 {
        return Err(AudioError::format(format!(
            "DIDX payload is {} bytes (not a multiple of 12)",
            didx.payload.len()
        )));
    }
    let word = |bytes: &[u8], pos: usize| {
        u32::from_le_bytes([bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]])
    };
    Ok(didx
        .payload
        .chunks_exact(12)
        .map(|entry| MediaEntry {
            media_id: word(entry, 0),
            offset: word(entry, 4),
            size: word(entry, 8),
        })
        .collect())
}

/// Whether the bank embeds media with this id (any error reads as `false`).
pub fn has_media(data: &[u8], media_id: u32) -> bool {
    media_index(data)
        .map(|index| index.iter().any(|entry| entry.media_id == media_id))
        .unwrap_or(false)
}

/// The payload of one embedded media item.
pub fn embedded_media(data: &[u8], media_id: u32) -> Result<&[u8]> {
    let chunks = chunks(data)?;
    let index = media_index(data)?;
    let entry = index
        .iter()
        .find(|entry| entry.media_id == media_id)
        .ok_or_else(|| AudioError::not_found(format!("bank embeds no media 0x{media_id:08x}")))?;
    let data_chunk = chunks
        .iter()
        .find(|chunk| chunk.id == *b"DATA")
        .ok_or_else(|| AudioError::format("DIDX present but no DATA chunk"))?;
    let start = entry.offset as usize;
    let end = start + entry.size as usize;
    data_chunk.payload.get(start..end).ok_or_else(|| {
        AudioError::format(format!(
            "media 0x{media_id:08x} spans {start:#x}..{end:#x}, outside the DATA chunk"
        ))
    })
}

/// One probe line: bank head, chunk list summary and embedded media ids.
pub fn describe(data: &[u8]) -> String {
    let Ok(chunks) = chunks(data) else {
        return format!("raw bytes (leading {:02x?})", &data[..data.len().min(4)]);
    };
    let mut parts: Vec<String> = Vec::new();
    for chunk in &chunks {
        match &chunk.id {
            b"BKHD" => match header(data) {
                Ok(head) => parts.push(format!("BKHD v{} id 0x{:08x}", head.version, head.id)),
                Err(_) => parts.push(format!("BKHD ({} bytes)", chunk.payload.len())),
            },
            b"DIDX" => match media_index(data) {
                Ok(index) if index.len() <= 6 => {
                    let ids: Vec<String> = index
                        .iter()
                        .map(|entry| format!("0x{:08x}", entry.media_id))
                        .collect();
                    parts.push(format!("DIDX ({} media: {})", index.len(), ids.join(", ")));
                }
                Ok(index) => parts.push(format!("DIDX ({} media)", index.len())),
                Err(_) => parts.push("DIDX (unparsed)".to_owned()),
            },
            b"DATA" => parts.push(format!("DATA ({} bytes)", chunk.payload.len())),
            other => parts.push(format!(
                "{} ({} bytes)",
                String::from_utf8_lossy(other),
                chunk.payload.len()
            )),
        }
    }
    format!("bank {}", parts.join(", "))
}
