# blaudio
*Create new Audio Banks and load any audio file into Borderlands 2 as a new in-game AkEvent.*

This is a [pyunrealsdk](https://bl-sdk.github.io/willow2-mod-db/) mod.

## Install

Unpack `blaudio.zip` into your games `Binaries/Win32/Plugins/sdk_mods/` directory.

## How to Use
This mod works both, standalone as a way for you to add new sounds, or as a library for other mods to build upon.  
In most cases you just want to install this mod and follow the instructions of whatever mod you want to use that has this mod listed as a requirement.  

The following information is mainly relevant for the mod devs that want to add new sounds for you to enjoy :)  

### Sound Manifests
To add new non replacing sounds you can use the `add_sounds/` folder in this mods directory!  
Inside `add_sounds/` you can create any amount of nested subfolders.  
The mod scans through the folders and searches for `.toml` files.  

Each `.toml` file describes a new Soundbank which can contain any number of audio events.  

The following snippet explains each field and its purpose:

```toml
namespace = "my_mod"          # required, usually your mod name
bank_name = "Weapons"         # required, name of the sound bank
# The resulting AkBank object will be named: `AkBank'Transient.{namespace}.{bank_name}'`

# [[sound]] is a table of sound events.
# You can declare any amount of [[sound]] table entries.
# Each describes one new AkEvent object.
# So make sure the name is unique each time!

[[sound]]
name = "shot"                 # required
sound = "new_shot.mp3"        # required, audio file, relative to the manifest
positioning = "weapon"        # optional: see below for a explanation
# The new AkEvent object will be named: AkEvent'Transient.{namespace}.{bank_name}.{name}'
# In the example above this is AkEvent'Transient.my_mod.Weapons.shot'

```

`namespace`, `bank_name`, and each sound `name` must contain only letters, digits, or underscores and must not start with a digit. Sound names must be unique within a bank (case-insensitive). Audio paths are relative to their manifest, including manifests in nested folders.


#### Positioning
| Manifest value      | Behavior                                                                                                    | Typical use cases                                                                                                                                 |
| ------------------- | ----------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------- |
| `2d`                | Non-positional playback: emitter location does not control direction or distance attenuation.               | UI clicks, notifications, music, narration, player-only feedback.                                                                                 |
| `2d_panned`         | Non-positional playback with Wwise’s 2D panner enabled.                                | Deliberately left/right-panned UI or cinematic audio. Merely selecting this does not automatically pan the sound. **Probably useless**                                |
| `3d`                | Directional, distance-attenuated audio that follows emitter position updates. **Default**                      | NPC speech, footsteps, moving machinery, vehicles, enemies, world sound effects.                                                                  |
| `3d_static`         | Directional, distance-attenuated audio using the emitter’s initial position rather than subsequent updates. | An explosion or impact that should remain where it happened, even if the posting actor moves afterward.                                           |
| `3d_no_attenuation` | Directional audio that follows the emitter, but without the bank’s distance-attenuation curve.              | A directional objective beacon or special cue that should remain audible at long distances. Usually unsuitable for ordinary world sounds. **Probably useless**         |
| `3d_unspatialized`  | Distance attenuation without direction-based spatialization.                                                | A nearby ambiance or hum that should fade with distance but not sound distinctly left or right. **Probably useless**                                               |
| `weapon`            | Perspective switch: 2D in first person, 3D in third person.                                                 | Weapon sounds that should be immediate for the wielder but positional for other listeners—provided the game sets the required perspective switch. |


### Python

Use the package-level helpers for automatic IDs, bank loading, and Unreal object creation:

```python
from blaudio import add, play

result = add(
    {"shot": "shot.mp3", "reload": "reload.mp3"},
    namespace="my_mod",
    bank_name="Weapons",
    positioning={"shot": "weapon", "reload": "2d"},
)
play(result.events["shot"])
```

`add` follows the manifest's naming rules: `namespace` and `bank_name` are required, and each dictionary key is a sound name. Names use the same identifier restrictions and case-insensitive uniqueness rules. Bank and event IDs are always allocated internally.

For low-level explicit-ID operations, use `api.craft_bank` and `api.load_bank`.

**ToDo**


## Notes:
Borderlands 2 uses Wwise as its underlying audio system.
It stores its Soundbanks and audio media in `Audio_Banks*.pck` and `Audio_Streaming*.pck` archives.  
Soundbanks contain event definitions and playback configuration, but can also contain embedded audio itself.  
The streamed audio is stored separately as `.wem` payloads.  
Unreal packages (`.upk`) contain the game-side objects, e.g. `AkEvent`, that hold the references used to trigger Wwise events.
`AkEvent` do not store the media itself! They just store a `ShortId` used to reference the specific Wwise sound event.

`.wem` is a Wwise audio container which usually contains compressed audio, such as Wwise Vorbis, but it can also hold uncompressed PCM samples!  
That means we do not need to use the audiokinetic WwiseCLI to create our `.wem` containers to encode the audio itself, we can just use the uncompressed payload.  

The only downside is memory. Because our audio is now uncompressed (and does not support streaming from disk) we need roughly ~10mb for a 1 minute audio file.
