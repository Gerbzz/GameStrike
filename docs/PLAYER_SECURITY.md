# GameStrike player boundary

Imported games are treated as untrusted code. This document describes what the player does, what has been measured, and what is still open. It is not a safety certification.

## Processes and trust

- **Launcher** (`gamestrike-launcher`). The trusted UI, built only from bundled files in `ui/`. Its IPC accepts six JSON commands from its own origin; see [LAUNCHER_UI.md](LAUNCHER_UI.md). It never loads game HTML.
- **Player** (`gamestrike-player`). One process per running game, started by the launcher with a validated game UUID and `--offline` or `--allow-network`. It has no IPC handler, adds no host objects, and disables WebView2 web messages and host objects before navigating. `player_source_registers_no_host_bridge` fails the build's tests if an IPC handler, `WebMessageReceived` handler, host object, or initialization script is added.
- **Integrity.** For imported games, the player queries its own Windows token and refuses to start unless it is at standard (medium) integrity. It never modifies the token. Fixture modes (`--smoke`, `--boundary-smoke`, `--probe-offline`, `--probe-network`) skip this check because they run only exact bundled first-party bytes; this lets CI runners with elevated tokens exercise them.
- **Per-game lock.** The player holds an exclusive OS file lock on `profiles/<id>.lock`, so a second player, a removal, or a save reset for that game is refused while it runs. The OS releases the lock if the player crashes.

## Per-game origin, storage, and files

Each game is served from `https://gamestrike<uuid>.localhost/` through a Wry custom protocol, with its own WebView2 user-data folder under `LIBRARY/profiles/<uuid>`. Origins and profiles never overlap, so `localStorage`, IndexedDB, and cookies are separate per game and persist across launches.

The asset handler serves only canonical regular files inside that game's `content` folder (no `..`, drive paths, device names, or links). It returns 403 or 404 otherwise, and adds `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`, and the mode's CSP to every response, errors included.

Top-level and frame navigation outside the game's origin is cancelled, including `file://`. New windows, downloads, permission requests, and external protocol launches are all denied. Accelerator keys, devtools, and the default context menu are disabled.

## Offline mode (default)

1. **CSP:** `default-src 'self' data: blob: 'unsafe-inline' 'unsafe-eval' 'wasm-unsafe-eval'; object-src 'none'; base-uri 'self'; form-action 'self'`.
2. **Owned sink-hole proxy:** `--proxy-server=http://127.0.0.1:<port>` with `--proxy-bypass-list=<-loopback>`, so loopback goes through the proxy too. The proxy is a listener inside the player that answers every connection with 403 and contacts nothing.
3. **WebRTC:** `--force-webrtc-ip-handling-policy=disable_non_proxied_udp` and `--webrtc-ip-handling-policy=disable_non_proxied_udp`. The first switch alone did not stop STUN UDP in WebView2, as measured below; with both, no UDP was observed.

When the game closes, the player reports how many blocked requests came from the game. WebView2's own configuration traffic (Microsoft service domains) and the runtime's requests for the game's own origin are still blocked, but they are not counted against the game.

## Network mode

The CSP is reduced to `object-src 'none'; base-uri 'self'`, and no proxy is used. Every other boundary above still applies. The launcher asks for consent each time, with focus starting on *Go back*.

## Measurements

These come from `tests/player_isolation.rs`, which drives the real player against `fixtures/isolation-probe.html` while a harness listens on `127.0.0.1:47391` over TCP and UDP. Run on the dev PC (Windows 11 Pro) on 2026-10-17, and in GitHub Actions `windows-latest` (runs 39068938218 and 39069702468):

| Probe | Offline | Network (control) |
| --- | --- | --- |
| TCP connections reaching the harness | **0** (two runs) | 8 |
| UDP (WebRTC STUN) packets reaching the harness | **0** with both WebRTC switches (4 with only the first) | 4 |
| `fetch`, XHR, WebSocket, image to loopback | rejected or error | `fetch` resolved |
| `fetch("file:///C:/Windows/win.ini")` | rejected | rejected |
| Fetch of another game's origin | rejected | — |
| `window.open` | `null` | `null` |
| `window.ipc` (Wry IPC) | `undefined` | `undefined` |
| `window.chrome.webview.postMessage` / `hostObjects` | present; no host handler or objects exist | same |
| `localStorage` and IndexedDB counters, game 1 | 1, then 2, then 3 after game 2 ran | — |
| Same counters, game 2 | — | 1 |
| Second player for a locked game | refused ("already open") | — |

A manual run of the real app also confirmed save persistence across exit and relaunch, offline loading of a wrapped ZIP's relative assets, and rejection of zip-slip and corrupt archives. See the README's Verification section.

## Open items

- The player is not an OS sandbox (no AppContainer or job object). A WebView2 engine exploit is out of scope for these defences.
- `window.chrome.webview` is still defined in game pages. Nothing is registered behind it, but removing it entirely would need a reliable per-frame mechanism.
- The probe doesn't exercise DNS prefetch, `<link rel=preconnect>`, service-worker registration, or `navigator.sendBeacon` delivery directly. The beacon is queued, but CSP and the proxy should stop it; the harness received no TCP connections at all.
- In network mode, the runtime's own requests for `https://gamestrike<uuid>.localhost` outside the custom protocol resolve to localhost:443.
- Refusal at elevated integrity is covered by unit tests of the decision. It has not been exercised from a real elevated launcher on the dev PC.
