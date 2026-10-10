//! audio: Wwise soundbank and package work for Borderlands 2.
//!
//! ## Overview
//!
//! - [`craft`]: build a minimal bank with brand-new sounds and events
//!   (the "add new audio" path): event -> Play action -> sound per
//!   sound, media embedded through `DIDX`/`DATA`.
//! - [`wwise`]: load and unload banks at runtime through the game's own
//!   `AK::SoundEngine`.
//! - [`resolve`]: build-independent lookup of the game's Wwise entries
//!   by decorated name, through the symbol table BL2 and TPS ship.
//! - [`pck`]: the `AKPK` file-package container (`.pck`). BL2 ships all
//!   soundbanks and streamed media inside `Audio_Banks*.pck` /
//!   `Audio_Streaming*.pck` archives. This module reads their payloads
//!   without changing any files.
//! - [`bank`]: a Wwise soundbank (`.bnk`) as its chunk list (`BKHD`, `DIDX`,
//!   `DATA`, ...), including access to embedded media through `DIDX`.
//! - [`wem`]: the RIFF/WAVE header of a `.wem` sound payload, enough to
//!   identify the codec so callers can check a payload.

pub mod bank;
pub mod craft;
pub mod decode;
pub mod error;
pub mod pck;
pub mod resolve;
pub mod wem;
pub mod wwise;

use std::path::Path;

pub use error::{AudioError, Result};

/// Reads a Wwise `.pck` package and returns a summary.
///
/// The summary includes the file size, AKPK version, language names, each
/// bank and stream's ID, size, and payload description, and the number of
/// external entries. Unreadable bank or stream payloads are reported in the
/// summary rather than failing the entire probe.
pub fn probe(pck_path: &Path) -> Result<String> {
    let package = pck::Pck::open(pck_path)?;
    let mut out = format!(
        "{}: {} bytes, AKPK version {}, {} language(s): {}\n",
        pck_path.display(),
        package.data_len(),
        package.version(),
        package.languages.len(),
        package
            .languages
            .iter()
            .map(|lang| lang.name.as_str())
            .collect::<Vec<_>>()
            .join(", "),
    );

    out.push_str(&format!("banks ({}):\n", package.banks.len()));
    for (index, entry) in package.banks.iter().enumerate() {
        out.push_str(&format!("  0x{:08x}  {} bytes  ", entry.id, entry.size));
        match package
            .blob(pck::TableKind::Banks, index)
            .map(bank::describe)
        {
            Ok(description) => out.push_str(&description),
            Err(e) => out.push_str(&format!("unreadable: {e}")),
        }
        out.push('\n');
    }

    out.push_str(&format!("streams ({}):\n", package.streams.len()));
    for (index, entry) in package.streams.iter().enumerate() {
        out.push_str(&format!("  0x{:08x}  {} bytes  ", entry.id, entry.size));
        match package.blob(pck::TableKind::Streams, index) {
            Ok(blob) => out.push_str(&wem::describe(blob)),
            Err(e) => out.push_str(&format!("unreadable: {e}")),
        }
        out.push('\n');
    }

    out.push_str(&format!("externals ({})\n", package.externals.len()));
    Ok(out)
}
