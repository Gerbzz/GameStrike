# Native local music

The native PWDS Music section manages ordered local-file queues and independent
saved playlists. Metadata writes are versioned and atomic; corrupt metadata is
preserved. Queue edits do not rescan the game catalog. Selecting or reopening a
playlist never automatically plays audio.

Transport runs separately from catalog work, using explicit track identities,
bounded commands, revocable session/operation authority and acknowledged state.
The UI exposes Play, Pause, Stop, Previous and Next intents and displays the actual
service snapshot. Music does not feed the microphone or communication mixer.

The optional Windows `native-music-host` feature uses pinned CPAL 0.16.0
(Apache-2.0) and Symphonia 0.5.5 (MPL-2.0). Decoder work stays off the UI/audio
callback, and the output callback consumes a bounded stereo PCM ring. The caller
must supply an explicitly selected output; this host never changes system audio
defaults or opens a microphone. Codec features are WAV/PCM, MP3 and FLAC only.

This is an engineering slice. The desktop intentionally retains an unavailable
playback service: real device-selection/lifecycle attachment is unfinished.
CPAL does not expose strict output drain, so natural completion/automatic advance
is unresolved. Initial output requires advertised exact-rate F32 stereo with no
adapter resampling/fallback. Raw contiguous MP3 decoder completion uses exact
packet-byte coverage; skipped metadata/info/junk and incomplete tails fail
conservatively. Gapless playback is not implemented. No provider account is
required, and no provider playback integration is added by this change.

Owned in-memory codec fixtures, fake devices and native view/service contracts
cover source boundaries without accessing audio devices. Hosted validation must
use a reviewed committed Cargo lock; resolver/formatting proposals require a new
commit and fresh-head run before acceptance. Passing synthetic checks does not
establish actual speaker output or physical-device acceptance.

Dependency source and license references:

- [CPAL 0.16.0](https://crates.io/crates/cpal/0.16.0),
  [source and Apache-2.0 license](https://github.com/RustAudio/cpal/tree/v0.16.0).
- [Symphonia 0.5.5](https://crates.io/crates/symphonia/0.5.5),
  [source](https://github.com/pdeljanov/Symphonia),
  [Mozilla Public License 2.0](https://www.mozilla.org/en-US/MPL/2.0/).

Distribution must retain applicable notices and make the corresponding dependency
source available as required by its license. This change includes no third-party
music, proprietary console recordings or virtual-audio driver.
