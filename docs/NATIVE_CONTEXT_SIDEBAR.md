# Native contextual sidebar

The four primary routes remain fixed: **Games, Soundboard, Voice, Music**. The lower
sidebar belongs to the selected route. The fixed orange-metal theme and cover-grid
geometry are unchanged. This is a native-feature-branch change, not a main cutover.

## Games

- **All Games**, **Favorites**, and **Recently Played** browse the current local
  catalog. Provider filters use typed catalog origins, never editable source names.
- Launcher catalogs: Steam, Epic Games, EA app, Microsoft Store, GOG Galaxy,
  Ubisoft Connect, Battle.net, and **Custom Programs**. Zero-count rows stay visible.
  The provider area scrolls when necessary; HTML/import/scan controls stay visible.
- Counts mean nonhidden saved catalog entries, including retained missing installs.
  They do **not** mean an installed launcher, signed-in account, or launch-ready game.
  `WIN` means Windows-only discovery; `OFF` means the existing scanner setting is off.
  Tooltips explain these states. Retained entries remain browsable even if their
  discovery adapter is unavailable on the current OS.
- **Scan Installed Games** explicitly invokes the existing local scanners. Startup
  initially reads the saved catalog; the sidebar does not imply a scan occurred.
  No launcher is installed, signed in, or contacted by selecting a filter.
- **HTML Games** is separate from Custom Programs. **Import HTML / ZIP…** opens a
  local file picker for `.html`, `.htm`, or `.zip`, then uses the existing validated
  importer. Ctrl/Cmd+O and local drag/drop remain available. Import copies content
  into the local library; it neither hosts/uploads content nor launches it.
- HTML launch remains an explicit separate action and requires the optional sibling
  isolated `gamestrike-player`, invoked with `--offline`. Archive validation and
  process/URI launch validation are unchanged.

Steam discovery supports Windows, Linux and macOS. The other named launcher
adapters are Windows-only. Microsoft Store discovery covers supported GDK packages,
not every Xbox/Game Pass title; EA discovery is best effort; an unknown Battle.net
product may only open its client. These are retained adapter limits, not new support
claims. Real-machine/provider acceptance remains separate from fixture contracts.

## Other routes

- **Music:** current queue, actual saved playlists, Add Local Music, and explicit
  Load Selected Queue reuse the existing Music state and commands. Selecting a
  playlist only changes selection. Loading changes the queue, not playback. No
  invented playlists, device outputs, sign-in, or provider integration are added.
- **Soundboard:** the lower bay explains that sound collections are not connected
  in this native build. No fake collection controls or output meters are shown.
- **Voice:** the lower bay explains that voice presets/processing are not connected.
  No microphone or voice device is opened.

Games filter/selection/scroll and Music selection/scroll are independent in-memory
state while moving between routes; this does not add cross-session preferences.
Game refresh is not offered as an active action in Soundboard or Voice.

## Validation boundary

New no-GPU contracts use the installed fonts and actual production egui controls:
typed-origin separation, real filter activation, unavailable/off scanners, explicit
HTML-picker/scan intents, clipped hit rejection, measured count/name regions, Music
selection retention, explicit playlist load and no keyboard file-picker leakage.
Minimum 880×620 and both acceptance sizes (1250×800, 1672×941) are covered.

The existing hosted native capture harness retains its nine Games/startup cases and
adds Soundboard, Voice and Music at both acceptance sizes (15 cases total).
`--capture-section` accepts only those existing routes and Games, and requires both
`--capture-native` and `--synthetic-library`. JSON metadata records the rendered
route. Input isolation, personal-catalog exclusion, no-host authority, immutable
output paths, source identity, real GPU readback, and pixel checks remain enforced.

Source preparation and formatting are not execution evidence. The sidebar change
requires fresh exact-source hosted format/tests/fatal Clippy/build, real framebuffer
inspection, and independent review before integration. Synthetic frames cannot
establish real catalogs, local player availability, audio devices, or release parity.
