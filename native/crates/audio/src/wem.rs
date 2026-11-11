//! The RIFF/WAVE header of a `.wem` sound payload.
//!
//! Streamed and bank-embedded media alike are whole `.wem` files
//! (`RIFF....WAVE` with `fmt ` + `data`). Every stream sampled from BL2's
//! `Audio_Streaming.pck` carries fmt tag `0xFFFF` with a 48-byte extension,
//! the 2011+ Wwise encoding.
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
/// obvious shape, plays nothing. Invalid geometry, partial sample frames,
/// and lengths or byte rates that exceed the RIFF fields return an error.
pub fn write_pcm_wem(pcm: &[u8], channels: u16, sample_rate: u32) -> Result<Vec<u8>> {
    if channels == 0 || sample_rate == 0 {
        return Err(AudioError::format(
            "PCM needs nonzero channels and sample rate",
        ));
    }
    let block_align = channels
        .checked_mul(2)
        .ok_or_else(|| AudioError::format("PCM block alignment exceeds uint16"))?;
    if !pcm.len().is_multiple_of(usize::from(block_align)) {
        return Err(AudioError::format("PCM ends inside a sample frame"));
    }
    let byte_rate = sample_rate
        .checked_mul(u32::from(block_align))
        .ok_or_else(|| AudioError::format("PCM byte rate exceeds uint32"))?;
    let data_size =
        u32::try_from(pcm.len()).map_err(|_| AudioError::format("PCM exceeds 4 GiB"))?;
    let riff_size = data_size
        .checked_add(60)
        .ok_or_else(|| AudioError::format("PCM RIFF exceeds 4 GiB"))?;
    let capacity = pcm
        .len()
        .checked_add(68)
        .ok_or_else(|| AudioError::format("PCM buffer size overflows"))?;
    let channel_mask: u32 = match channels {
        1 => 0x4, // SPEAKER_FRONT_CENTER
        2 => 0x3, // SPEAKER_FRONT_LEFT | SPEAKER_FRONT_RIGHT
        n => (1u32 << n.min(18)) - 1,
    };
    let mut out = Vec::with_capacity(capacity);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&riff_size.to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&0xFFFEu16.to_le_bytes()); // WAVE_FORMAT_EXTENSIBLE
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    out.extend_from_slice(&22u16.to_le_bytes()); // cbSize
    out.extend_from_slice(&16u16.to_le_bytes()); // valid bits per sample
    out.extend_from_slice(&channel_mask.to_le_bytes());
    out.extend_from_slice(&PCM_SUBTYPE_GUID);
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_size.to_le_bytes());
    out.extend_from_slice(pcm);
    Ok(out)
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
        let payload = data[pos + 8..].get(..size).ok_or_else(|| {
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
            payload[18..]
                .get(..cb_size)
                .ok_or_else(|| AudioError::format("fmt extension overruns the fmt chunk"))?
                .to_vec()
        }
        None if payload.len() == 16 => Vec::new(),
        None => return Err(AudioError::format("truncated fmt extension size")),
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
