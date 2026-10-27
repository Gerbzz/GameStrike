# GameStrike player boundary

Imported games are treated as untrusted code. This document describes what the player does, what has been measured, and what is still open. It is not a safety certification.

## Processes and trust

- **Launcher** (`gamestrike-launcher`). The trusted UI, built only from bundled files in `ui/`. Its IPC accepts six JSON commands from its own origin; see [LAUNCHER_UI.md](LAUNCHER_UI.md). It never loads game HTML.
- **Player** (`gamestrike-player`). One process per running game, started by the launcher with a validated game UUID and `--offline` or `--allow-network`. It has no IPC handler, adds no host objects, and disables WebView2 web messages and host objects before navigating. `player_source_registers_no_host_bridge` fails the build's tests if an IPC handler, `WebMessageReceived` handler, host object, or initialization script is added.
- **Integrity.** For imported games, the player queries its own Windows token and refuses to start unless it is at standard (medium) integrity. It never modifies the token. Fixture modes (`--smoke`, `--boundary-smoke`, `--probe-offline`, `--probe-network`) skip this check because they run only exact bundled first-party bytes; this lets CI runners with elevated tokens exercise them.
- **Per-game lock.** The player holds an exclusive OS file lock on `profiles/<id>.lock`, so a second player, a removal, or a save reset for that game is refused while it runs. The OS releases the lock if the player crashes.

## Per-platform implementation

The sections below describe the Windows (WebView2) player, which is the measured one. The same guarantees are implemented on Linux and macOS as follows. **Only platforms that GitHub Actions has built and tested count as verified; see the README's Platforms section for the current status.**

| Guarantee | Windows (WebView2) | Linux (WebKitGTK 4.1) | macOS 14+ (WKWebView) |
| --- | --- | --- | --- |
| Per-game data store | User-data folder `profiles/<id>` | WebKit data and cache directories `profiles/<id>`, persistent; the player reads the directory back and refuses to start if it differs | `WKWebsiteDataStore` with identifier = game ID, persistent; verified after the WebView is built. It lives in WebKit's own folder, not the library; `gamestrike-player --clear-engine-data` deletes all of its website data types through WebKit's public `removeDataOfTypes:modifiedSince:` (the empty store folder stays), and the launcher and CLI call that before clearing saves or removing a game without kept saves |
| Per-game origin | `https://gamestrike<id>.localhost` (Wry custom protocol) | `gamestrike<id>://localhost` (registered as secure) | `gamestrike<id>://localhost` (`WKURLSchemeHandler`) |
| No `file://` escape, assets only from the game folder | Same asset handler on every platform: canonical regular files inside `content/` only | same | same |
| Navigation outside the game blocked | Navigation handler plus native frame-navigation and external-URI guards | Navigation handler (WebKit's navigation policy covers main frame and subframes); the `https://…localhost` form is refused because it would be a real network request | Navigation handler (`decidePolicyForNavigationAction`, all frames); same refusal of the https form |
| Popups, downloads, permissions | Denied | Denied | Denied |
| CSP | Header on every response | Same header (custom-scheme responses carry headers) | same |
| No host IPC | No IPC handler; web messages and host objects disabled. `window.chrome.webview` still exists | No IPC handler or script message handler; Wry's `window.ipc` stub script is removed before the game loads, so `window.ipc` and `window.webkit` are undefined | No IPC handler or user scripts (checked after build), so `window.ipc` and `window.webkit` are undefined |
| Offline network block | CSP + owned sink-hole proxy (loopback included) + WebRTC limited to proxied UDP | CSP + owned sink-hole proxy set on the WebKit data manager; WebRTC and media capture disabled in every mode; DNS prefetch and hyperlink auditing off | CSP + owned sink-hole proxy set on the data store (verified); WebRTC disabled in every mode through WebKit's peer-connection preference, which is private WebKit API (`_setPeerConnectionEnabled:`); if it is missing or does not stick, the player refuses to start |
| Elevated identity refused | Not at medium integrity | Root, or real and effective user or group IDs differ | same as Linux |
| OS sandbox | None | None (WebKitGTK's bubblewrap sandbox is not enabled) | None (the app is unsigned and not App Sandboxed) |

WebRTC is off in network mode too on Linux and macOS, so games that need WebRTC do not work there. On macOS, game saves are not counted in the library's "game data" size because WebKit stores them outside the library.

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

### Linux and macOS measurements

The same test, run by the cross-platform workflow in [run 39198332877](https://github.com/Gerbzz/GameStrike/actions/runs/39198332877) (Ubuntu 24.04 with WebKitGTK 2.52.6 under Xvfb; macOS 26.6 on Apple silicon). These are CI results only; no one has tested the Linux or macOS builds by hand.

| Probe | Linux offline | Linux network | macOS offline | macOS network |
| --- | --- | --- | --- | --- |
| TCP connections reaching the harness | **0** | 7 | **0** | 7 |
| UDP packets reaching the harness | **0** | 0 (WebRTC unavailable) | **0** | 0 (WebRTC unavailable) |
| `csp` (violation events) | enforced | none | enforced | none |
| `fetch`, XHR, image to loopback | rejected / error / error | resolved / loaded / error | rejected / error / error | resolved / loaded / error |
| `file://` fetch, other game origin | rejected, rejected | rejected, rejected | rejected, rejected | rejected, rejected |
| `window.open`, `window.ipc`, `window.webkit` | null, undefined, undefined | same | null, undefined, undefined | same |
| `RTCPeerConnection` | unavailable | unavailable | unavailable | unavailable |
| Reported origin | `gamestrike<id>://localhost`, different per game | | same | |
| Saves | 1, 2, then 3 after game 2 ran; 1 again after clearing saves | game 2 starts at 1 | same | same |

## Open items

- The player is not an OS sandbox (no AppContainer or job object). A WebView2 engine exploit is out of scope for these defences.
- `window.chrome.webview` is still defined in game pages. Nothing is registered behind it, but removing it entirely would need a reliable per-frame mechanism.
- The probe doesn't exercise DNS prefetch, `<link rel=preconnect>`, service-worker registration, or `navigator.sendBeacon` delivery directly. The beacon is queued, but CSP and the proxy should stop it; the harness received no TCP connections at all.
- In network mode, the runtime's own requests for `https://gamestrike<uuid>.localhost` outside the custom protocol resolve to localhost:443.
- Refusal at elevated integrity is covered by unit tests of the decision. It has not been exercised from a real elevated launcher on the dev PC.
