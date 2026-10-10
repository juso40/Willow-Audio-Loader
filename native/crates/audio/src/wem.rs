//! The RIFF/WAVE header of a `.wem` sound payload.
//!
//! Streamed and bank-embedded media alike are whole `.wem` files
//! (`RIFF....WAVE` with `fmt ` + `data`). Every stream sampled from BL2's
//! `Audio_Streaming.pck` carries fmt tag `0xFFFF` with a 48-byte extension,
//!  the 2011+ Wwise encoding.
//! The engine's Wwise build compiles the Vorbis source codecs
//! (`CAkSrcBankVorbis` / `CAkSrcFileVorbis` RTTI in `Borderlands2.exe`),
//! so that tag maps to [`WemCodec::WwiseVorbis`] here. Bank PCM must be
//! WAVEFORMATEXTENSIBLE (`0xFFFE`) to play, see [`write_pcm_wem`].

use crate::error::{AudioError, Result};

/// The codec a `.wem` payload is encoded with, from the `fmt ` tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WemCodec {
    /// Uncompressed PCM (fmt `0x0001`, or `0xFFFE` WAVEFORMATEXTENSIBLE
    /// as `CAkSrcBankPCM` demands in banks).
    Pcm,
    /// MS ADPCM (fmt `0x0002`).
    Adpcm,
    /// Wwise Vorbis (fmt `0x0165`, and `0xFFFF` for 2011+ encodings).
    WwiseVorbis,
    /// xWMA (fmt `0x0166`).
    Xwma,
    /// A tag this build does not know. It carries the raw tag value.
    Unknown(u16),
}

impl WemCodec {
    /// Maps a WAVE format tag to a codec.
    pub fn from_fmt_tag(tag: u16) -> Self {
        match tag {
            0x0001 | 0xFFFE => Self::Pcm,
            0x0002 => Self::Adpcm,
            0x0165 | 0xFFFF => Self::WwiseVorbis,
            0x0166 => Self::Xwma,
            other => Self::Unknown(other),
        }
    }

    /// The codec id a bank sound node carries for this codec, from the
    /// game's own source factory (`FUN_0130d9a0`): 1 = PCM and 2 = ADPCM
    /// construct the native `CAkSrcBank*` sources, anything else goes to
    /// the plugin-source factory (Vorbis = 4, matching every shipped
    /// sound node). `Xwma` has no bank-embedded source in the build.
    pub fn bank_codec_id(self) -> Result<u16> {
        match self {
            Self::Pcm => Ok(1),
            Self::Adpcm => Ok(2),
            Self::WwiseVorbis => Ok(4),
            Self::Xwma | Self::Unknown(_) => Err(AudioError::format(format!(
                "{self:?} has no bank-embedded source in this build (use PCM or Vorbis)"
            ))),
        }
    }
}

/// `KSDATAFORMAT_SUBTYPE_PCM`: the SubFormat GUID a
/// WAVEFORMATEXTENSIBLE `fmt ` carries for plain PCM.
const PCM_SUBTYPE_GUID: [u8; 16] = [
    0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71,
];

/// Builds a 16-bit PCM `.wem` (RIFF/WAVE with a WAVEFORMATEXTENSIBLE
/// `fmt `, tag `0xFFFE`) around interleaved s16le samples.
///
/// The tag is not a detail: `CAkSrcBankPCM::Start` (`FUN_01320400`)
/// rejects anything but `0xFFFE` with `AK_InvalidFile`, then reads the
/// rate at +4, block align at +12, bits at +14 and the channel mask at
/// +20 out of the extensible block. A plain `0x0001` PCM `.wem`, the
/// obvious shape, plays nothing.
pub fn write_pcm_wem(pcm: &[u8], channels: u16, sample_rate: u32) -> Vec<u8> {
    let block_align = channels * 2;
    let channel_mask: u32 = match channels {
        1 => 0x4, // SPEAKER_FRONT_CENTER
        2 => 0x3, // SPEAKER_FRONT_LEFT | SPEAKER_FRONT_RIGHT
        n => (1u32 << n.min(18)) - 1,
    };
    let mut out = Vec::with_capacity(pcm.len() + 58);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&0u32.to_le_bytes()); // patched below
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&0xFFFEu16.to_le_bytes()); // WAVE_FORMAT_EXTENSIBLE
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(sample_rate * block_align as u32).to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    out.extend_from_slice(&22u16.to_le_bytes()); // cbSize
    out.extend_from_slice(&16u16.to_le_bytes()); // valid bits per sample
    out.extend_from_slice(&channel_mask.to_le_bytes());
    out.extend_from_slice(&PCM_SUBTYPE_GUID);
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
    out.extend_from_slice(pcm);
    let riff_size = (out.len() - 8) as u32;
    out[4..8].copy_from_slice(&riff_size.to_le_bytes());
    out
}

/// Parsed `fmt ` essentials of one `.wem`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WemHeader {
    /// The payload codec.
    pub codec: WemCodec,
    /// The raw WAVE format tag.
    pub fmt_tag: u16,
    /// Channel count.
    pub channels: u16,
    /// Samples per second.
    pub sample_rate: u32,
    /// The `fmt ` extension bytes after `cbSize`, verbatim (Wwise codec
    /// parameters live here).
    pub fmt_extension: Vec<u8>,
}

/// Whether the bytes start `RIFF....WAVE`.
pub fn is_wem(data: &[u8]) -> bool {
    data.len() >= 12 && &data[..4] == b"RIFF" && &data[8..12] == b"WAVE"
}

/// Parses the `fmt ` chunk of a `.wem`. Fails with [`AudioError::Format`]
/// when the bytes are not RIFF/WAVE or carry no usable `fmt ` chunk.
pub fn parse(data: &[u8]) -> Result<WemHeader> {
    if !is_wem(data) {
        return Err(AudioError::format(format!(
            "not a .wem payload: {:02x?} (expected RIFF....WAVE)",
            &data[..data.len().min(4)]
        )));
    }
    let mut pos = 12;
    while pos + 8 <= data.len() {
        let id = &data[pos..pos + 4];
        let size = u32::from_le_bytes([data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7]])
            as usize;
        let payload = data.get(pos + 8..pos + 8 + size).ok_or_else(|| {
            AudioError::format(format!(
                "chunk {:?} overruns the file",
                String::from_utf8_lossy(id)
            ))
        })?;
        if id == b"fmt " {
            return parse_fmt(payload);
        }
        // RIFF chunks pad to an even offset.
        pos += 8 + size + (size % 2);
    }
    Err(AudioError::format("no fmt chunk in .wem"))
}

fn parse_fmt(payload: &[u8]) -> Result<WemHeader> {
    if payload.len() < 16 {
        return Err(AudioError::format(format!(
            "fmt chunk is {} bytes (need at least 16)",
            payload.len()
        )));
    }
    let fmt_tag = u16::from_le_bytes([payload[0], payload[1]]);
    let channels = u16::from_le_bytes([payload[2], payload[3]]);
    let sample_rate = u32::from_le_bytes([payload[4], payload[5], payload[6], payload[7]]);
    // WAVEFORMATEX (18+ bytes) carries cbSize then the codec extension.
    // Plain WAVEFORMAT (16) has neither.
    let fmt_extension = match payload.get(16..18) {
        Some(size_bytes) => {
            let cb_size = u16::from_le_bytes([size_bytes[0], size_bytes[1]]) as usize;
            payload
                .get(18..18 + cb_size)
                .unwrap_or(&payload[18..])
                .to_vec()
        }
        None => Vec::new(),
    };
    Ok(WemHeader {
        codec: WemCodec::from_fmt_tag(fmt_tag),
        fmt_tag,
        channels,
        sample_rate,
        fmt_extension,
    })
}

/// One probe line for a payload: `"wem vorbis 44100Hz 1ch"` when it parses
/// as a `.wem`, otherwise a note that it is raw bytes (bank media may be).
pub fn describe(data: &[u8]) -> String {
    match parse(data) {
        Ok(header) => format!(
            "wem {:?} {}Hz {}ch",
            header.codec, header.sample_rate, header.channels
        ),
        Err(_) => format!("raw bytes (leading {:02x?})", &data[..data.len().min(4)]),
    }
}

/// Caveats for a replacement: codec mismatches when both payloads are
/// `.wem`s, and a note when replacing a `.wem` with non-`.wem` bytes.
/// Raw-to-raw swaps (bank media) are unchecked by design, the caller
/// supplies the format the consumer expects.
pub fn codec_warnings(old: &[u8], new: &[u8]) -> Vec<String> {
    let mut warnings = Vec::new();
    match (is_wem(old), is_wem(new)) {
        (true, true) => {
            if let (Ok(old), Ok(new)) = (parse(old), parse(new)) {
                if old.codec != new.codec {
                    warnings.push(format!(
                        "codec mismatch: original is {:?}, replacement is {:?}",
                        old.codec, new.codec
                    ));
                }
                if old.channels != new.channels {
                    warnings.push(format!(
                        "channel mismatch: original is {}ch, replacement is {}ch. \
                         The existing bank's positioning settings stay unchanged",
                        old.channels, new.channels
                    ));
                }
            }
        }
        (true, false) => {
            warnings.push(
                "replacement is not a RIFF/WAVE .wem but the original is. \
                 The consumer expects the same wrapping"
                    .to_owned(),
            );
        }
        _ => {}
    }
    warnings
}
