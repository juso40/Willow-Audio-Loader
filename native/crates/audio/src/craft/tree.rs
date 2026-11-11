//! Native step-selection trees using the BL2 v62 bank format.

use super::*;
use std::collections::HashSet;

const NODE_STEP: u32 = 0x9E37_79B9;
const MAX_NODES: usize = 65535;

/// Step-selection mode: each event post selects one leaf.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Selection {
    /// Weighted random, excluding recently selected entries.
    Random,
    /// Weighted shuffle: exhaust entries before refilling the pool.
    Shuffle,
    /// Advance in playlist order and wrap at the end.
    Sequence,
}

/// Existing output buses from the game's Init bank.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Routing {
    /// The original Startup_SFX donor route.
    #[default]
    Sfx,
    /// Base dialogue bus, independent of Player_CallOuts suppression.
    Voice,
    /// Player dialogue bus, honoring Player_CallOuts On/Off.
    PlayerVoice,
}

impl Routing {
    fn bus(self) -> u32 {
        match self {
            Self::Sfx => 0x0553_61E8,
            Self::Voice => 0x1AB4_BFFE,
            Self::PlayerVoice => 0x17C6_E01B,
        }
    }
}

/// A declarative tree with clip pools and nestable switch/state branches.
/// None branches are intentionally silent, not missing-value fallbacks.
#[derive(Debug, Clone)]
pub enum SoundSource {
    /// One audio source, normalized to embedded WEM at craft time.
    Clip {
        /// Common audio or WEM bytes.
        media: Vec<u8>,
        /// Optional source size hint, as in NewSound.
        size_hint: Option<u32>,
    },
    /// A playlist of audio clips, not simultaneously layered sounds.
    Pool {
        /// Common audio or WEM sources in playlist order.
        clips: Vec<Vec<u8>>,
        /// How each post selects a clip.
        selection: Selection,
        /// Positive weights in clip order. Wwise's default is 50000.
        weights: Vec<u32>,
        /// Number of recent entries excluded from selection.
        avoid_repeat: u16,
        /// Share history across emitters instead of per game object.
        global: bool,
    },
    /// Read an existing game-object switch or global state.
    Switch {
        /// Wwise group id.
        group: u32,
        /// True for a global state, false for a game-object switch.
        is_state: bool,
        /// Declared branch selected for unhandled values.
        default: u32,
        /// Value ids with subtrees. None means intentional silence.
        branches: Vec<(u32, Option<SoundSource>)>,
    },
}

/// One event backed by an audio clip or native selection tree.
#[derive(Debug, Clone)]
pub struct NewEvent {
    /// Nonzero numeric Wwise event id.
    pub event_id: u32,
    /// Audio and selection logic.
    pub source: SoundSource,
    /// Emitter positioning, including optional perspective selection.
    pub positioning: Positioning,
    /// Loops the selected leaf. Selection still happens once per post.
    pub looping: bool,
    /// Optional companion Stop event targeting the entire tree.
    pub stop_event_id: Option<u32>,
    /// Existing output bus, without changing global engine settings.
    pub routing: Routing,
    /// Event-wide volume offset in dB (-96..+12), defaulting to zero.
    pub gain: f32,
}

impl SoundSource {
    fn count(&self, depth: usize, total: &mut usize, clips: &mut usize) -> Result<usize> {
        *total += 1;
        if depth > 16 || *total > MAX_NODES {
            return Err(AudioError::format("sound tree exceeds depth/node limit"));
        }
        match self {
            Self::Clip { .. } => {
                *clips += 1;
                Ok(1)
            }
            Self::Pool {
                clips: media,
                selection,
                weights,
                avoid_repeat,
                ..
            } => {
                if depth == 16
                    || media.is_empty()
                    || media.len() > MAX_NODES
                    || weights.len() != media.len()
                    || usize::from(*avoid_repeat) >= media.len()
                    || weights.contains(&0)
                    || weights.iter().map(|w| u64::from(*w)).sum::<u64>() > 0x4000_0000
                    || (*selection == Selection::Sequence
                        && (*avoid_repeat != 0 || weights.iter().any(|w| *w != 50000)))
                {
                    return Err(AudioError::format(
                        "invalid pool size, weights, or repeat avoidance",
                    ));
                }
                *clips += media.len();
                *total += media.len();
                if *total > MAX_NODES {
                    return Err(AudioError::format("sound tree exceeds node limit"));
                }
                Ok(1 + media.len())
            }
            Self::Switch {
                group,
                default,
                branches,
                ..
            } => {
                let states: HashSet<_> = branches.iter().map(|(state, _)| *state).collect();
                if *group == 0
                    || branches.is_empty()
                    || branches.len() > MAX_NODES
                    || states.contains(&0)
                    || states.len() != branches.len()
                    || !states.contains(default)
                {
                    return Err(AudioError::format(
                        "invalid switch/state group, branches, or default",
                    ));
                }
                let mut count = 1;
                for source in branches.iter().filter_map(|(_, source)| source.as_ref()) {
                    count += source.count(depth + 1, total, clips)?;
                }
                Ok(count)
            }
        }
    }
}

/// Build a mixed bank of ordinary sounds and native selection trees.
/// Extra ids are event_id XOR (index * NODE_STEP), index starting at 1.
/// Fixed and indexed ids are validated before any audio decoding.
pub fn craft_events(bank_id: u32, bank_name: &str, events: &[NewEvent]) -> Result<Vec<u8>> {
    if bank_id == 0 || events.is_empty() || bank_name.is_empty() || bank_name.len() > 255 {
        return Err(AudioError::format(
            "craft_events: need events and a 1..=255 byte bank name",
        ));
    }
    let mut ids = HashSet::from([0, ATTENUATION_ID]);
    for event in events {
        if !event.gain.is_finite() || !(-96.0..=12.0).contains(&event.gain) {
            return Err(AudioError::format(
                "gain must be finite and between -96 and +12 dB",
            ));
        }
        let mut total = 0;
        let mut clips = 0;
        let count = event.source.count(0, &mut total, &mut clips)?;
        if clips == 0 {
            return Err(AudioError::format(
                "sound trees must contain at least one clip",
            ));
        }
        let mut family = vec![
            event.event_id,
            event.event_id ^ SOUND_ID_XOR,
            event.event_id ^ ACTION_ID_XOR,
            event.event_id ^ MIXER_ID_XOR,
        ];
        if event.positioning == Positioning::Perspective {
            family.extend([
                event.event_id ^ SWITCH_ID_XOR,
                event.event_id ^ THIRD_PERSON_ID_XOR,
            ]);
        }
        if !matches!(event.source, SoundSource::Clip { .. }) {
            let count = count
                * if event.positioning == Positioning::Perspective {
                    2
                } else {
                    1
                };
            for i in 1..=count {
                family.push(event.event_id ^ (i as u32).wrapping_mul(NODE_STEP));
            }
        }
        if let Some(stop) = event.stop_event_id {
            family.extend([stop, stop ^ ACTION_ID_XOR]);
        }
        for id in family {
            if !ids.insert(id) {
                return Err(AudioError::format(format!(
                    "craft_events: id 0x{id:08x} collides or is reserved"
                )));
            }
        }
    }

    let mut nodes = Vec::new();
    let mut didx = Vec::new();
    let mut data = Vec::new();
    for event in events {
        let (mut event_nodes, media) = TreeBuilder::build(bank_id, event)?;
        // Each event has one root actor-mixer. Apply Volume here, never
        // on both parents and children (which would multiply the gain).
        // Patch after all fixed-offset positioning/routing edits.
        if event.gain != 0.0 {
            for (kind, body) in &mut event_nodes {
                if *kind == 7 {
                    set_mixer_gain(body, event.gain);
                }
            }
        }
        nodes.extend(event_nodes);
        for (id, bytes) in media {
            push_media(&mut didx, &mut data, id, &bytes)?;
        }
    }
    let mut bkhd = Vec::new();
    for word in [BANK_VERSION, bank_id, 0, 0, 0, 0] {
        push_u32(&mut bkhd, word);
    }
    let mut hirc = Vec::new();
    push_u32(
        &mut hirc,
        (nodes.len() + 1)
            .try_into()
            .map_err(|_| AudioError::format("too many HIRC nodes"))?,
    );
    for (kind, body) in nodes {
        push_object(&mut hirc, kind, &body);
    }
    push_object(&mut hirc, 0x0e, &ATTENUATION_TEMPLATE);
    let mut stid = Vec::new();
    for word in [1, 1, bank_id] {
        push_u32(&mut stid, word);
    }
    stid.push(bank_name.len() as u8);
    stid.extend_from_slice(bank_name.as_bytes());
    let mut out = Vec::new();
    for (tag, body) in [
        (b"BKHD", bkhd),
        (b"DIDX", didx),
        (b"DATA", data),
        (b"HIRC", hirc),
        (b"STID", stid),
    ] {
        push_chunk(&mut out, tag, &body);
    }
    Ok(out)
}

type Nodes = Vec<(u8, Vec<u8>)>;
type Media = Vec<(u32, Vec<u8>)>;

struct TreeBuilder<'a> {
    bank: u32,
    event: &'a NewEvent,
    next: u32,
    nodes: Nodes,
    media: Media,
    leaf: usize,
    mirror: bool,
}

impl<'a> TreeBuilder<'a> {
    fn build(bank: u32, event: &'a NewEvent) -> Result<(Nodes, Media)> {
        let mut this = Self {
            bank,
            event,
            next: 0,
            nodes: Vec::new(),
            media: Vec::new(),
            leaf: 0,
            mirror: false,
        };
        let mixer = event.event_id ^ MIXER_ID_XOR;
        let target = if event.positioning == Positioning::Perspective {
            let switch = event.event_id ^ SWITCH_ID_XOR;
            let first = this.root(switch, None)?;
            this.leaf = 0;
            this.mirror = true;
            let third = this.root(switch, Some(Positioning::Spatial))?;
            this.nodes
                .push((6, perspective_switch(switch, mixer, first, third)));
            this.nodes.push((
                7,
                routed_mixer(mixer, switch, Positioning::Flat, event.routing),
            ));
            switch
        } else {
            let root = this.root(mixer, None)?;
            this.nodes.push((
                7,
                routed_mixer(mixer, root, event.positioning, event.routing),
            ));
            root
        };
        let action = event.event_id ^ ACTION_ID_XOR;
        let mut body = ACTION_TEMPLATE;
        body[..4].copy_from_slice(&action.to_le_bytes());
        body[6..10].copy_from_slice(&target.to_le_bytes());
        body[13..17].copy_from_slice(&bank.to_le_bytes());
        this.nodes.push((3, body.to_vec()));
        this.event_node(event.event_id, action);
        if let Some(stop) = event.stop_event_id {
            let mut body = Vec::new();
            push_u32(&mut body, stop ^ ACTION_ID_XOR);
            body.extend([3, 1]);
            push_u32(&mut body, target);
            body.extend([0, 0, 4]);
            push_u32(&mut body, 0);
            this.nodes.push((3, body));
            this.event_node(stop, stop ^ ACTION_ID_XOR);
        }
        Ok((this.nodes, this.media))
    }

    fn root(&mut self, parent: u32, positioning: Option<Positioning>) -> Result<u32> {
        if let SoundSource::Clip { media, size_hint } = &self.event.source {
            let mask = if self.mirror {
                THIRD_PERSON_ID_XOR
            } else {
                SOUND_ID_XOR
            };
            self.clip(
                self.event.event_id ^ mask,
                media,
                parent,
                positioning,
                *size_hint,
            )
        } else {
            self.source(&self.event.source, parent, positioning)
        }
    }

    fn id(&mut self) -> u32 {
        self.next += 1;
        self.event.event_id ^ self.next.wrapping_mul(NODE_STEP)
    }

    fn event_node(&mut self, id: u32, action: u32) {
        let mut body = Vec::new();
        for word in [id, 1, action] {
            push_u32(&mut body, word);
        }
        self.nodes.push((4, body));
    }

    fn clip(
        &mut self,
        id: u32,
        media: &[u8],
        parent: u32,
        positioning: Option<Positioning>,
        size_hint: Option<u32>,
    ) -> Result<u32> {
        if !self.mirror {
            self.media.push((id, crate::decode::to_wem(media)?));
        }
        let (media_id, bytes) = &self.media[self.leaf];
        self.leaf += 1;
        let codec = crate::wem::parse(bytes)?.codec.bank_codec_id()?;
        let len =
            u32::try_from(bytes.len()).map_err(|_| AudioError::format("clip exceeds 4 GiB"))?;
        let mut body = SOUND_TEMPLATE.to_vec();
        for (offset, value) in [
            (0, id),
            (12, *media_id),
            (16, self.bank),
            (20, size_hint.unwrap_or(len)),
            (24, len),
            (35, parent),
        ] {
            body[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        body[6..8].copy_from_slice(&codec.to_le_bytes());
        if let Some(mode) = positioning {
            body.splice(43..44, mode.parameters());
        }
        set_looping(&mut body, self.event.looping);
        self.nodes.push((2, body));
        Ok(id)
    }

    fn source(
        &mut self,
        source: &SoundSource,
        parent: u32,
        positioning: Option<Positioning>,
    ) -> Result<u32> {
        let id = self.id();
        if let SoundSource::Clip { media, size_hint } = source {
            return self.clip(id, media, parent, positioning, *size_hint);
        }
        let mut body = node_base(id, parent, positioning);
        match source {
            SoundSource::Pool {
                clips,
                selection,
                weights,
                avoid_repeat,
                global,
            } => {
                let mut children = Vec::new();
                for clip in clips {
                    let child = self.id();
                    children.push(self.clip(child, clip, id, None, None)?);
                }
                // Loop once with zero transition times and no crossfade.
                body.extend(1u16.to_le_bytes());
                body.extend([0; 12]);
                body.extend(avoid_repeat.to_le_bytes());
                body.extend([0, u8::from(*selection == Selection::Shuffle)]);
                body.extend(u16::from(*selection == Selection::Sequence).to_le_bytes());
                // Keep playlist progress, not ping-pong, step (one clip per post), history scope
                body.extend([0, 0, 0, u8::from(*global)]);
                push_u32(&mut body, children.len() as u32);
                for child in &children {
                    push_u32(&mut body, *child);
                }
                body.extend((children.len() as u16).to_le_bytes());
                for (child, weight) in children.into_iter().zip(weights) {
                    push_u32(&mut body, child);
                    push_u32(&mut body, *weight);
                }
                self.nodes.push((5, body));
            }
            SoundSource::Switch {
                group,
                is_state,
                default,
                branches,
            } => {
                let mut mapped = Vec::new();
                let mut children = Vec::new();
                for (state, branch) in branches {
                    let child = match branch {
                        None => None,
                        Some(branch) => Some(self.source(branch, id, None)?),
                    };
                    if let Some(child) = child {
                        children.push(child);
                    }
                    mapped.push((*state, child));
                }
                push_u32(&mut body, u32::from(*is_state));
                push_u32(&mut body, *group);
                push_u32(&mut body, *default);
                body.push(0); // step switch: state changes affect later posts, not current speech
                push_u32(&mut body, children.len() as u32);
                for child in &children {
                    push_u32(&mut body, *child);
                }
                push_u32(&mut body, mapped.len() as u32);
                for (state, child) in mapped {
                    push_u32(&mut body, state);
                    push_u32(&mut body, u32::from(child.is_some()));
                    if let Some(child) = child {
                        push_u32(&mut body, child);
                    }
                }
                push_u32(&mut body, children.len() as u32);
                for child in children {
                    push_u32(&mut body, child);
                    body.extend([0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
                }
                self.nodes.push((6, body));
            }
            SoundSource::Clip { .. } => unreachable!(),
        }
        Ok(id)
    }
}

/// The donor mixer's property bundle at +16 contains only Priority (id 4).
/// Add Volume (id 0, float dB) while preserving priority and all other bytes.
/// Serialized bundles have separate id/value arrays with no alignment padding.
fn set_mixer_gain(body: &mut Vec<u8>, gain: f32) {
    let mut bundle = vec![2, 0, 4]; // count, Volume, Priority
    bundle.extend(gain.to_le_bytes());
    bundle.extend_from_slice(&body[18..22]);
    body.splice(16..22, bundle);
}

fn routed_mixer(id: u32, child: u32, positioning: Positioning, routing: Routing) -> Vec<u8> {
    let mut body = mixer_body(id, child, positioning);
    body[6..10].copy_from_slice(&routing.bus().to_le_bytes());
    body
}

/// Common node parameters from the existing donor mixer, without its child
/// list. Child containers inherit output bus and (normally) positioning.
fn node_base(id: u32, parent: u32, positioning: Option<Positioning>) -> Vec<u8> {
    let mut body = MIXER_TEMPLATE[..50].to_vec();
    body[..4].copy_from_slice(&id.to_le_bytes());
    body[6..10].fill(0);
    body[10..14].copy_from_slice(&parent.to_le_bytes());
    body.splice(
        23..35,
        positioning.map_or_else(|| vec![0], Positioning::parameters),
    );
    body
}
