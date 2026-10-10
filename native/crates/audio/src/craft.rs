//! Crafting a minimal "new audio" bank: one Wwise event per sound.
//!
//! The default 3D path uses four `HIRC` objects per event plus media
//! in `DIDX`/`DATA`, with one shared attenuation object:
//!
//! ```text
//! Event        (type 0x04): id, 1 action: the Play action below
//! Action       (type 0x03): id, 0x0403 (Play), target: the sound
//! Sound        (type 0x02): id, media id, bank id, media size,
//!                           parent @35 -> its actor-mixer
//! Actor-mixer  (type 0x07): id, child: the sound, 3D attenuation id
//! Attenuation  (type 0x0E): one shared leaf, verbatim from the donor
//! ```
//!
//! Weapon positioning adds a perspective switch and a second sound node.
//! The first-person voice inherits 2D. The third-person voice overrides
//! it with 3D. Both reference the same media. The switch comes from
//! Startup_SFX event `0xad34538a`.
//!
//! The donor graph is from the base game's `Audio_Banks.pck`, bank
//! `Startup_SFX` (`0xf3949abc`): event `0xab77694d` -> action `0x1e51e83f`
//! -> sound `0x30109020` under mixer `0x2a029d2a` -> attenuation `0x03f8f132`.
//! The mixer routes to output bus `0x055361e8` in the vanilla `Init` bank.
//! Templates are embedded here.
//!
//! The donor event contains one action ID. Its 17-byte Play action carries
//! the bank ID at +13. The sound's media ID (+12), bank ID (+16), and media
//! size (+24) match its embedded `DIDX` entry. Its parent ID is at +35
//! (a parentless sound posts and resolves but never makes a sound).
//!
//! The crafter copies unknown fields verbatim from the donor and patches
//! only the identified ones, including positioning in weapon mode.
//! The size hint has unknown semantics at
//! +20: shipped values range from 0 to 9522x the media size and
//! everything plays, so it is a hint, not a constraint. We write the
//! media length there. `NewSound::size_hint` overrides it.

use crate::error::{AudioError, Result};

/// Bank format version claimed in `BKHD` (matches the template bank).
const BANK_VERSION: u32 = 62;

/// Object id derivation constants. A sound's object ids are the event id
/// with these xored in, so one caller-supplied id determines the graph.
/// Pick event ids that cannot collide with loaded banks (Wwise ids are
/// name hashes, so anything from your own naming scheme is fine).
const SOUND_ID_XOR: u32 = 0x534F_554E; // "SOUN"
const ACTION_ID_XOR: u32 = 0x4143_5449; // "ACTI"
const MIXER_ID_XOR: u32 = 0x4D49_5852; // "MIXR"
const SWITCH_ID_XOR: u32 = 0x5357_4954; // "SWIT"
const THIRD_PERSON_ID_XOR: u32 = 0x5448_5244; // "THRD"

// Startup_SFX event 0xad34538a uses this game-object switch group.
const PERSPECTIVE_GROUP_ID: u32 = 0x336A_82E2;
const FIRST_PERSON_ID: u32 = 0x47C7_8432; // Wwise hash of "FirstPerson"
const THIRD_PERSON_ID: u32 = 0xA4B0_480F; // Wwise hash of "ThirdPerson"

/// The attenuation object shared by all crafted spatialized 3D sound
/// nodes. It is copied unchanged from the donor graph, including its ID.
const ATTENUATION_ID: u32 = 0x03F8_F132;

// Keep the shared attenuation reservation in blaudio/ids.py in sync.
// Node templates below come from vanilla `Audio_Banks.pck`, bank
// `Startup_SFX`: sound `0x30109020`, actor-mixer `0x2a029d2a`, attenuation
// `0x03f8f132` (event `0xab77694d` -> action `0x1e51e83f` -> sound).
// Templates zero the fields patched per sound and retain all other bytes.

/// A 59-byte BL2 sound node (in-bank media class). Patched per sound:
/// id @0, codec id @6 (PCM = 1, Vorbis = 4, see
/// [`crate::wem::WemCodec::bank_codec_id`]), media id @12, bank id @16,
/// size hint @20, media size @24, **parent actor-mixer id @35** (the
/// routing chain). A sound with no parent posts and resolves but never
/// makes a sound.
const SOUND_TEMPLATE: [u8; 59] = [
    0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, // id / flags + codec
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, // media id / bank id / hint / size
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // +28
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, // parent id @35 (patched)
    0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, // +48 (play-mode enum @50 = 2)
];

/// A 58-byte BL2 actor-mixer node (type `0x07`): `id(4) node block(46)
/// u32 child_count child_ids[]`. The node block holds the attenuation id
/// at +29 (verbatim). Patch id @0 and child id @54 (always one child,
/// the sound it routes).
const MIXER_TEMPLATE: [u8; 58] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xe8, 0x61, 0x53, 0x05, 0x00, 0x00, // id (patched)
    0x00, 0x00, 0x00, 0x01, 0x01, 0x04, 0x00, 0x00, 0x0c, 0x42, 0x00, 0x01, // node block
    0x01, 0x03, 0x00, 0x00, 0x00, 0x32, 0xf1, 0xf8, 0x03, 0x01, 0x01,
    0x01, // attenuation id 0x03f8f132 @29
    0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //
    0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // count = 1, child id
];

/// The 40-byte attenuation leaf, type `0x0E`, including its id
/// [`ATTENUATION_ID`]. Self-contained, with no external references.
const ATTENUATION_TEMPLATE: [u8; 40] = [
    0x32, 0xf1, 0xf8, 0x03, 0x00, 0x00, 0x00, 0x00, 0xff, 0xff, 0x01, 0x02, 0x02, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x40, 0x9c, 0x45, 0x9a, 0x99,
    0xc0, 0xc2, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00,
];

/// The fixed parts of the 17-byte Play action (`0x0403`), same source.
/// Layout: `id(4) type(2) target(4) 00 00 04 bank_id(4)`.
const ACTION_TEMPLATE: [u8; 17] = [
    0, 0, 0, 0, // id (patched)
    0x03, 0x04, // 0x0403 = Play
    0, 0, 0, 0, // target sound id (patched)
    0x00, 0x00, 0x04, // constant in every sampled Play action
    0, 0, 0, 0, // bank id (patched)
];

/// Positioning behavior for a crafted event, independent of media channels.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Positioning {
    /// The existing donor graph's game-defined, dynamic 3D positioning.
    #[default]
    Spatial,
    /// Non-positional 2D playback with the 2D panner disabled.
    Flat,
    /// 2D playback with the bank's panner enabled, centered by default.
    Panned,
    /// Spatialized 3D using the emitter's initial position, not updates.
    Static,
    /// Dynamic spatialized 3D without an attenuation curve.
    Unattenuated,
    /// Dynamic 3D attenuation without direction-based spatialization.
    Unspatialized,
    /// The Vladof shot's perspective switch: 2D in first person, 3D in
    /// third person. The game must set the switch on the posting object.
    Weapon,
}

impl std::str::FromStr for Positioning {
    type Err = AudioError;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "3d" => Ok(Self::Spatial),
            "2d" => Ok(Self::Flat),
            "2d_panned" => Ok(Self::Panned),
            "3d_static" => Ok(Self::Static),
            "3d_no_attenuation" => Ok(Self::Unattenuated),
            "3d_unspatialized" => Ok(Self::Unspatialized),
            "weapon" => Ok(Self::Weapon),
            _ => Err(AudioError::format(format!(
                "unknown positioning {value:?}. Expected 2d, 2d_panned, 3d, \
                 3d_static, 3d_no_attenuation, 3d_unspatialized or weapon"
            ))),
        }
    }
}

impl Positioning {
    /// BL2 SetPositioningParams bytes. These are variable-length: three
    /// for explicit 2D, twelve for game-defined 3D. Weapon's root is 2D.
    fn parameters(self) -> Vec<u8> {
        match self {
            Self::Flat | Self::Weapon => vec![1, 0, 0],
            Self::Panned => vec![1, 0, 1],
            Self::Spatial | Self::Static | Self::Unattenuated | Self::Unspatialized => {
                let mut params = MIXER_TEMPLATE[23..35].to_vec();
                match self {
                    Self::Static => params[11] = 0,
                    Self::Unattenuated => params[6..10].fill(0),
                    Self::Unspatialized => params[10] = 0,
                    _ => {}
                }
                params
            }
        }
    }
}

/// One sound to add: a new Wwise event and the audio it plays.
#[derive(Debug, Clone)]
pub struct NewSound {
    /// The Wwise event id posted to play this sound (e.g. through
    /// `blaudio.play_id`).
    pub event_id: u32,
    /// The audio: any common format (WAV, MP3, FLAC, OGG, AAC, ...) or a
    /// `.wem`. Normalized at craft time (decoded to a PCM `.wem`, or
    /// passed through for `.wem`s) and embedded in the bank's `DATA`
    /// chunk through `DIDX`.
    pub media: Vec<u8>,
    /// Optional override for the sound node's size-hint field (offset
    /// +20). Defaults to the media length. See the module docs.
    pub size_hint: Option<u32>,
    /// How the bank positions the sound. Does not downmix the media.
    pub positioning: Positioning,
}

/// Builds a minimal bank (version [`BANK_VERSION`]) with one event per
/// sound, wired event -> Play action -> sound -> embedded media, named
/// `bank_name` in its `STID` entry.
///
/// Chunk order follows the shipped banks: `BKHD`, `DIDX`, `DATA`,
/// `HIRC`, `STID` (chunks are back to back, no padding). The result is
/// exactly what `wwise::load_bank` takes.
pub fn craft_bank(bank_id: u32, bank_name: &str, sounds: &[NewSound]) -> Result<Vec<u8>> {
    if sounds.is_empty() {
        return Err(AudioError::format("craft_bank: no sounds"));
    }
    if bank_name.is_empty() || bank_name.len() > 255 {
        return Err(AudioError::format(
            "craft_bank: bank_name must be 1..=255 bytes",
        ));
    }
    let mut ids = std::collections::HashSet::new();
    ids.insert(ATTENUATION_ID);
    for sound in sounds {
        let mut object_ids = vec![
            sound.event_id,
            sound.event_id ^ SOUND_ID_XOR,
            sound.event_id ^ ACTION_ID_XOR,
            sound.event_id ^ MIXER_ID_XOR,
        ];
        if sound.positioning == Positioning::Weapon {
            object_ids.extend([
                sound.event_id ^ SWITCH_ID_XOR,
                sound.event_id ^ THIRD_PERSON_ID_XOR,
            ]);
        }
        for id in object_ids {
            if !ids.insert(id) {
                return Err(AudioError::format(format!(
                    "craft_bank: id 0x{id:08x} collides (event ids too close together)"
                )));
            }
        }
    }

    // BKHD: version, bank id, four zero words (24-byte form, matching
    // the vanilla Startup_SFX donor bank).
    let mut bkhd = Vec::with_capacity(24);
    push_u32(&mut bkhd, BANK_VERSION);
    push_u32(&mut bkhd, bank_id);
    for _ in 0..4 {
        push_u32(&mut bkhd, 0);
    }

    // Media is normalized first (arbitrary audio -> PCM `.wem`, `.wem`s
    // pass through), so each sound's codec id is known before the nodes
    // are built. Source channels and sample rate are preserved.
    let mut medias = Vec::with_capacity(sounds.len());
    for sound in sounds {
        let media = crate::decode::to_wem(&sound.media)?;
        let codec_id = crate::wem::parse(&media)?.codec.bank_codec_id()?;
        medias.push((media, codec_id));
    }

    // DIDX + DATA: one media entry per sound, payloads contiguous.
    let mut didx = Vec::with_capacity(sounds.len() * 12);
    let mut data = Vec::new();
    for (sound, (media, _)) in sounds.iter().zip(&medias) {
        let media_id = sound.event_id ^ SOUND_ID_XOR;
        push_u32(&mut didx, media_id);
        push_u32(&mut didx, data.len() as u32);
        push_u32(&mut didx, media.len() as u32);
        data.extend_from_slice(media);
    }

    // HIRC: four nodes per spatial event, six per weapon event, then
    // one shared attenuation leaf. Parent ids link each voice to its
    // switch or mixer.
    let mut hirc = Vec::new();
    let object_count: usize = sounds
        .iter()
        .map(|sound| {
            if sound.positioning == Positioning::Weapon {
                6
            } else {
                4
            }
        })
        .sum();
    push_u32(&mut hirc, (object_count + 1) as u32);
    for (sound, (media, codec_id)) in sounds.iter().zip(&medias) {
        let sound_id = sound.event_id ^ SOUND_ID_XOR;
        let mixer_id = sound.event_id ^ MIXER_ID_XOR;
        let action_id = sound.event_id ^ ACTION_ID_XOR;
        let media_id = sound_id; // derived the same way, one media per sound

        let mut sound_body = SOUND_TEMPLATE;
        sound_body[0..4].copy_from_slice(&sound_id.to_le_bytes());
        sound_body[6..8].copy_from_slice(&codec_id.to_le_bytes());
        sound_body[12..16].copy_from_slice(&media_id.to_le_bytes());
        sound_body[16..20].copy_from_slice(&bank_id.to_le_bytes());
        let hint = sound.size_hint.unwrap_or(media.len() as u32);
        sound_body[20..24].copy_from_slice(&hint.to_le_bytes());
        sound_body[24..28].copy_from_slice(&(media.len() as u32).to_le_bytes());
        let target_id = if sound.positioning == Positioning::Weapon {
            let switch_id = sound.event_id ^ SWITCH_ID_XOR;
            let third_id = sound.event_id ^ THIRD_PERSON_ID_XOR;
            // Both voices belong to the switch. First person inherits
            // 2D from the mixer. Third person overrides it with 3D.
            sound_body[35..39].copy_from_slice(&switch_id.to_le_bytes());
            push_object(&mut hirc, 0x02, &sound_body);
            let mut third_body = sound_body.to_vec();
            third_body[0..4].copy_from_slice(&third_id.to_le_bytes());
            // SetPositioningParams consumes one byte for inheritance,
            // or twelve for an explicit game-defined 3D override.
            third_body.splice(43..44, Positioning::Spatial.parameters());
            push_object(&mut hirc, 0x02, &third_body);
            push_object(
                &mut hirc,
                0x06,
                &weapon_switch(switch_id, mixer_id, sound_id, third_id),
            );

            push_object(
                &mut hirc,
                0x07,
                &mixer_body(mixer_id, switch_id, Positioning::Flat),
            );
            switch_id
        } else {
            sound_body[35..39].copy_from_slice(&mixer_id.to_le_bytes());
            push_object(&mut hirc, 0x02, &sound_body);
            push_object(
                &mut hirc,
                0x07,
                &mixer_body(mixer_id, sound_id, sound.positioning),
            );
            sound_id
        };

        let mut action_body = ACTION_TEMPLATE;
        action_body[0..4].copy_from_slice(&action_id.to_le_bytes());
        action_body[6..10].copy_from_slice(&target_id.to_le_bytes());
        action_body[13..17].copy_from_slice(&bank_id.to_le_bytes());
        push_object(&mut hirc, 0x03, &action_body);

        let mut event_body = Vec::with_capacity(12);
        push_u32(&mut event_body, sound.event_id);
        push_u32(&mut event_body, 1); // action count
        push_u32(&mut event_body, action_id);
        push_object(&mut hirc, 0x04, &event_body);
    }
    push_object(&mut hirc, 0x0E, &ATTENUATION_TEMPLATE);

    let mut out = Vec::with_capacity(bkhd.len() + didx.len() + data.len() + hirc.len() + 32);
    // STID: version 1, one entry naming the bank (the shape of every
    // shipped data bank: `u32 version`, `u32 count`, then
    // `{u32 id, u8 name_len, name}` entries, names not NUL-terminated).
    let mut stid = Vec::with_capacity(bank_name.len() + 13);
    push_u32(&mut stid, 1);
    push_u32(&mut stid, 1);
    push_u32(&mut stid, bank_id);
    stid.push(bank_name.len() as u8);
    stid.extend_from_slice(bank_name.as_bytes());

    push_chunk(&mut out, b"BKHD", &bkhd);
    push_chunk(&mut out, b"DIDX", &didx);
    push_chunk(&mut out, b"DATA", &data);
    push_chunk(&mut out, b"HIRC", &hirc);
    push_chunk(&mut out, b"STID", &stid);
    Ok(out)
}

/// Patch ids before replacing the positioning block, since its length
/// shifts the child list. All other donor fields remain unchanged.
fn mixer_body(id: u32, child: u32, positioning: Positioning) -> Vec<u8> {
    let mut body = MIXER_TEMPLATE.to_vec();
    body[0..4].copy_from_slice(&id.to_le_bytes());
    body[54..58].copy_from_slice(&child.to_le_bytes());
    body.splice(23..35, positioning.parameters());
    body
}

/// Switch node from Startup_SFX 0x2ba7dcd7, selected by event
/// 0xad34538a. It retains the node parameters and child playback flags,
/// replaces the ids, and turns its two random containers into direct
/// sounds.
fn weapon_switch(id: u32, parent: u32, first: u32, third: u32) -> Vec<u8> {
    let mut out = vec![
        0, 0, 0, 0, // id
        0, 0, // FX override and count
        0, 0, 0, 0, // output bus: inherit
        0, 0, 0, 0, // parent
        0, 1, // priority override and distance factor
        1, 4, 0, 0, 0x44, 0x42, // priority property = 49
        0,    // ranged property count
        0,    // positioning: inherit
        0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    ];
    out[0..4].copy_from_slice(&id.to_le_bytes());
    out[10..14].copy_from_slice(&parent.to_le_bytes());
    push_u32(&mut out, PERSPECTIVE_GROUP_ID);
    push_u32(&mut out, THIRD_PERSON_ID); // default if the switch is unset
    out.push(0); // not a continuous switch container
    push_u32(&mut out, 2);
    push_u32(&mut out, first);
    push_u32(&mut out, third);
    push_u32(&mut out, 2);
    for (state, child) in [(FIRST_PERSON_ID, first), (THIRD_PERSON_ID, third)] {
        push_u32(&mut out, state);
        push_u32(&mut out, 1);
        push_u32(&mut out, child);
    }
    push_u32(&mut out, 2);
    for child in [first, third] {
        push_u32(&mut out, child);
        out.extend_from_slice(&[0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    }
    out
}

fn push_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_chunk(out: &mut Vec<u8>, id: &[u8; 4], payload: &[u8]) {
    out.extend_from_slice(id);
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(payload);
}

fn push_object(out: &mut Vec<u8>, hirc_type: u8, body: &[u8]) {
    out.push(hirc_type);
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(body);
}
