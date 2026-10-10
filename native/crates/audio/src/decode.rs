//! Decoding arbitrary audio to Wwise-ready 16-bit PCM.
//!
//! `symphonia` covers WAV, MP3, FLAC, OGG Vorbis, AAC/MP4 and raw PCM.
//! [`to_wem`] turns any of those into a PCM `.wem` the bank crafter (and
//! the game) understands. Bank-playable `.wem`s pass through untouched
//! (Wwise-Vorbis, `0xFFFE` PCM, ADPCM). A plain `0x0001` WAV is
//! rewrapped, since that tag plays nothing in banks.
//!
//! Output is interleaved signed 16-bit little-endian PCM at the source's
//! own sample rate and channel count. No resampling or downmixing.
//! Media channels and bank positioning are independent.

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

use crate::error::{AudioError, Result};

/// Decoded PCM: interleaved s16le samples plus the stream geometry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PcmAudio {
    /// Interleaved signed 16-bit little-endian samples.
    pub data: Vec<u8>,
    /// Channel count (1 = mono, 2 = stereo, ...).
    pub channels: u16,
    /// Samples per second.
    pub sample_rate: u32,
}

/// Decodes any supported audio file to 16-bit PCM.
///
/// Covers WAV, MP3, FLAC, OGG Vorbis, AAC/MP4 and headerless PCM. Bad
/// data fails with [`AudioError::Format`]. A truncated-but-valid file
/// yields whatever decoded before the truncation.
pub fn decode_to_pcm16(bytes: &[u8]) -> Result<PcmAudio> {
    let source = MediaSourceStream::new(
        Box::new(std::io::Cursor::new(bytes.to_vec())),
        Default::default(),
    );
    let probed = symphonia::default::get_probe()
        .format(
            &Hint::new(),
            source,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|e| AudioError::format(format!("cannot decode audio: {e}")))?;
    let mut format = probed.format;
    let track = format
        .default_track()
        .ok_or_else(|| AudioError::format("audio has no decodable track"))?;
    let track_id = track.id;
    let mut channels = track
        .codec_params
        .channels
        .map(|c| c.count() as u16)
        .unwrap_or(1);
    let mut sample_rate = track.codec_params.sample_rate.unwrap_or(44_100);
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| AudioError::format(format!("unsupported audio codec: {e}")))?;

    let mut data = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(symphonia::core::errors::Error::IoError(e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(e) => return Err(AudioError::format(format!("cannot decode audio: {e}"))),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(decoded) => decoded,
            Err(symphonia::core::errors::Error::DecodeError(_)) => continue,
            Err(e) => return Err(AudioError::format(format!("cannot decode audio: {e}"))),
        };
        let spec = *decoded.spec();
        channels = spec.channels.count() as u16;
        sample_rate = spec.rate;
        let mut samples = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
        samples.copy_interleaved_ref(decoded);
        for sample in samples.samples() {
            let clamped = sample.clamp(-1.0, 1.0);
            data.extend_from_slice(&((clamped * i16::MAX as f32).round() as i16).to_le_bytes());
        }
    }
    if data.is_empty() {
        return Err(AudioError::format("audio decoded to zero samples"));
    }
    Ok(PcmAudio {
        data,
        channels,
        sample_rate,
    })
}

/// Normalizes any audio to a `.wem` the bank crafter embeds.
///
/// A Wwise `.wem` (Vorbis, PCM or ADPCM) passes through untouched.
/// Anything else gets decoded to PCM and wrapped in a PCM `.wem`
/// ([`crate::wem::write_pcm_wem`]). This is what makes `add` accept WAV,
/// MP3 and others.
///
/// The output keeps the source channels and sample rate. Bank positioning is
/// configured separately when crafting the bank.
pub fn to_wem(bytes: &[u8]) -> Result<Vec<u8>> {
    // Passthrough only for tags the `CAkSrcBank*` sources accept as-is:
    // Wwise Vorbis, WAVEFORMATEXTENSIBLE PCM (`0xFFFE`), ADPCM. A plain
    // `0x0001` WAV looks like a `.wem` but is NOT bank-playable, so it
    // falls through and gets rewrapped below.
    if let Ok(header) = crate::wem::parse(bytes)
        && matches!(header.fmt_tag, 0xFFFF | 0x0165 | 0xFFFE | 0x0002)
    {
        return Ok(bytes.to_vec());
    }
    let pcm = decode_to_pcm16(bytes)?;
    Ok(crate::wem::write_pcm_wem(
        &pcm.data,
        pcm.channels,
        pcm.sample_rate,
    ))
}
