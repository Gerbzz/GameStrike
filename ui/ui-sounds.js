// Original, short interface cues. No samples, ambience, network, media, or capture APIs.
(function (root, factory) {
  const api = factory(root);
  if (typeof module === "object" && module.exports) module.exports = api;
  else root.GameStrikeUiSounds = api;
})(typeof window !== "undefined" ? window : globalThis, function (root) {
  "use strict";

  const CUES = Object.freeze({
    navigate: Object.freeze({ frequency: 780, milliseconds: 28, peak: 0.032, attack: 2.5 }),
    confirm: Object.freeze({ frequency: 1040, milliseconds: 65, peak: 0.05, attack: 4 }),
    back: Object.freeze({ frequency: 520, milliseconds: 45, peak: 0.03, attack: 3 }),
    error: Object.freeze({ frequency: 260, milliseconds: 80, peak: 0.036, attack: 4 }),
  });
  const ALIASES = Object.freeze({ move: "navigate", zone: "navigate", select: "confirm", open: "confirm", launch: "confirm", cancel: "back" });
  function canonicalCue(kind) {
    const name = ALIASES[kind] || kind;
    return Object.prototype.hasOwnProperty.call(CUES, name) ? name : null;
  }
  function clampVolume(value, fallback = 0.72) {
    return typeof value === "number" && Number.isFinite(value) ? Math.min(1, Math.max(0, value)) : fallback;
  }

  // A fixed-pitch rounded pulse, with a small second harmonic for definition.
  // The analytic envelope starts/ends at zero and never exceeds the cue's peak.
  function renderCue(kind, sampleRate = 48000) {
    const cue = CUES[canonicalCue(kind)];
    if (!cue) return null;
    if (!Number.isFinite(sampleRate) || sampleRate < 8000 || sampleRate > 192000) throw new RangeError("Unsupported cue sample rate");
    const pcm = new Float32Array(Math.round(sampleRate * cue.milliseconds / 1000));
    for (let i = 0; i < pcm.length; i += 1) {
      const seconds = i / sampleRate;
      const progress = i / (pcm.length - 1);
      const attack = Math.sin(Math.min(1, seconds * 1000 / cue.attack) * Math.PI / 2);
      const envelope = attack * Math.pow(1 - progress, 2.2);
      const phase = 2 * Math.PI * cue.frequency * seconds;
      pcm[i] = cue.peak * envelope * (Math.sin(phase) + 0.12 * Math.sin(phase * 2)) / 1.12;
    }
    return pcm;
  }

  function create(options = {}) {
    const host = options.host || root;
    const document = host.document;
    const read = options.readPreference || (() => null);
    const write = options.writePreference || (() => {});
    const now = options.now || (() => host.performance.now());
    const suppressed = options.isSuppressed || (() => false);
    let enabled = read("gamestrike-sounds", "on") !== "off";
    const savedVolume = read("gamestrike-sounds-volume", null);
    let volume = savedVolume === null ? 0.72 : clampVolume(Number(savedVolume));
    let held = false, unlocked = false, disposed = false;
    let context = null, master = null, pending = false, epoch = 0;
    let lastNavigation = -Infinity, lastAction = -Infinity, lastError = -Infinity;
    const voices = new Set();
    const buffers = new Map();
    const canPlay = () => !disposed && unlocked && enabled && volume > 0 && !held && !document?.hidden && !suppressed();

    function stop() {
      epoch += 1;
      for (const voice of [...voices]) {
        try { voice.stop(); } catch (_) { /* already ended */ }
        voice.disconnect();
      }
      voices.clear();
    }
    // Unlocking records actual input; it never creates or starts an audio device.
    function unlock() { if (!disposed) unlocked = true; }
    const input = (event) => { if (event.isTrusted) unlock(); };
    const visibility = () => { if (document.hidden) stop(); };
    for (const name of ["keydown", "pointerdown", "mousedown"]) host.addEventListener?.(name, input, true);
    document?.addEventListener("visibilitychange", visibility);

    function emit(kind, timestamp) {
      if (!canPlay() || context.state !== "running") return false;
      if (kind === "navigate" && (timestamp - lastNavigation < 120 || timestamp - lastAction < 120)) return false;
      if ((kind === "confirm" || kind === "back") && timestamp - lastAction < 100) return false;
      if (kind === "error" && timestamp - lastError < 800) return false;
      if (voices.size >= 2) return false;
      let buffer = buffers.get(kind);
      if (!buffer) {
        const pcm = renderCue(kind, context.sampleRate);
        buffer = context.createBuffer(1, pcm.length, context.sampleRate);
        buffer.getChannelData(0).set(pcm);
        buffers.set(kind, buffer);
      }
      const source = context.createBufferSource();
      source.buffer = buffer;
      source.connect(master);
      source.onended = () => { voices.delete(source); source.disconnect(); };
      voices.add(source);
      try { source.start(); }
      catch (error) { voices.delete(source); source.disconnect(); throw error; }
      if (kind === "navigate") lastNavigation = timestamp;
      else if (kind === "error") lastError = timestamp;
      else lastAction = timestamp;
      return true;
    }

    function play(name) {
      const kind = canonicalCue(name);
      if (!kind || !canPlay()) return false;
      try {
        if (!context) {
          const Context = host.AudioContext || host.webkitAudioContext;
          if (!options.contextFactory && !Context) return false;
          context = options.contextFactory ? options.contextFactory() : new Context();
          master = context.createGain();
          master.gain.value = volume;
          master.connect(context.destination);
        }
        if (context.state === "running") return emit(kind, now());
        // Keep only the current input while the device resumes; never queue repeats.
        if (context.state !== "suspended" || pending) return false;
        pending = true;
        const requestedAt = now(), version = epoch;
        Promise.resolve(context.resume()).then(() => {
          if (epoch === version && now() - requestedAt <= 120) emit(kind, now());
        }).catch(() => {}).finally(() => { pending = false; });
        return true;
      } catch (_) { pending = false; return false; /* cues never interrupt interaction */ }
    }

    return {
      play, unlock, stop,
      // Startup stays silent. These retain the launcher's skip/startup call contract.
      bootSounds() {}, stopBoot() {},
      get enabled() { return enabled; },
      get volume() { return volume; },
      toggle() {
        enabled = !enabled;
        write("gamestrike-sounds", enabled ? "on" : "off");
        if (!enabled) stop();
        else play("confirm");
      },
      setVolume(value) {
        volume = clampVolume(value, volume);
        write("gamestrike-sounds-volume", String(volume));
        if (master) master.gain.setValueAtTime(volume, context.currentTime);
        if (volume === 0) stop();
      },
      hold(on) { held = Boolean(on); if (held) stop(); },
      dispose() {
        disposed = true;
        stop();
        for (const name of ["keydown", "pointerdown", "mousedown"]) host.removeEventListener?.(name, input, true);
        document?.removeEventListener("visibilitychange", visibility);
        buffers.clear();
        if (context) context.close().catch(() => {});
      },
    };
  }
  return Object.freeze({ create, renderCue, canonicalCue, CUES });
});
