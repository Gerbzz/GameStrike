const assert = require("node:assert/strict");
const test = require("node:test");
const { create, renderCue, CUES } = require("./ui-sounds.js");

function fixture({ state = "running", preference = {}, suppressed = false } = {}) {
  let time = 0, constructions = 0, isSuppressed = suppressed;
  const listeners = new Map(), events = [], sources = [];
  const document = {
    hidden: false,
    addEventListener(type, fn) { listeners.set(type, fn); },
    removeEventListener(type) { listeners.delete(type); },
  };
  const host = { document, addEventListener(type, fn) { listeners.set(type, fn); }, removeEventListener(type) { listeners.delete(type); } };
  const context = {
    state, sampleRate: 48000, currentTime: 0, destination: {},
    createGain() { return { gain: { value: 1, setValueAtTime(value) { this.value = value; } }, connect() {} }; },
    createBuffer(channels, length) { const pcm = new Float32Array(length); return { getChannelData: () => pcm }; },
    createBufferSource() {
      const source = { connect() {}, start() { events.push([time, this.buffer.getChannelData(0).length]); }, stop() { this.stopped = true; }, disconnect() { this.disconnected = true; } };
      sources.push(source); return source;
    },
    resume() { this.state = "running"; return Promise.resolve(); },
    close() { this.closed = true; return Promise.resolve(); },
  };
  const sound = create({ host, now: () => time, isSuppressed: () => isSuppressed, contextFactory: () => { constructions += 1; return context; }, readPreference: (key, fallback) => preference[key] ?? fallback, writePreference: (key, value) => { preference[key] = value; } });
  return { sound, context, document, sources, events, preference, listeners, get constructions() { return constructions; }, at(value) { time = value; }, suppress(value) { isSuppressed = value; }, finish() { sources.forEach(s => s.onended?.()); }, input(trusted = true) { listeners.get("keydown")({ isTrusted: trusted }); } };
}

test("startup, passive events, and untrusted input never open an audio context", () => {
  const f = fixture();
  f.sound.bootSounds(0); f.sound.stopBoot();
  assert.equal(f.sound.play("move"), false);
  f.input(false); assert.equal(f.sound.play("error"), false);
  assert.equal(f.listeners.has("pointerenter"), false);
  assert.equal(f.constructions, 0);
  f.input(); assert.equal(f.sound.play("move"), true);
  assert.equal(f.constructions, 1);
});

test("held navigation and zone transitions share one 120 ms gate without a delayed queue", () => {
  const f = fixture(); f.input();
  for (let ms = 0; ms < 1000; ms += 20) { f.at(ms); f.sound.play(ms % 40 ? "zone" : "move"); f.finish(); }
  assert.deepEqual(f.events.map(event => event[0]), [0, 120, 240, 360, 480, 600, 720, 840, 960]);
  f.at(5000); assert.equal(f.events.length, 9, "nothing plays after the user stops moving");
});

test("confirm is distinct, repeated action callbacks coalesce, and errors are rare", () => {
  const f = fixture(); f.input();
  assert.equal(f.sound.play("open"), true);
  assert.equal(f.sound.play("select"), false);
  f.at(100); f.finish(); assert.equal(f.sound.play("launch"), true);
  f.at(200); f.finish(); assert.equal(f.sound.play("error"), true);
  f.at(300); f.finish(); assert.equal(f.sound.play("error"), false);
  f.at(999); assert.equal(f.sound.play("error"), false);
  f.at(1000); assert.equal(f.sound.play("error"), true);
  assert.equal(f.sound.play("boot"), false);
  assert.equal(f.sound.play("unknown"), false);
  assert.notDeepEqual(renderCue("move"), renderCue("select"));
});

test("voice count and sample peaks bound rapid mixed interactions", () => {
  const f = fixture(); f.input();
  assert.equal(f.sound.play("move"), true);
  f.at(1); assert.equal(f.sound.play("confirm"), true);
  f.at(2); assert.equal(f.sound.play("error"), false, "at most two sources can overlap");
  assert.equal(f.sources.length, 2);
  for (const kind of Object.keys(CUES)) {
    const pcm = renderCue(kind);
    assert.equal(Math.abs(pcm[0]), 0); assert.equal(Math.abs(pcm.at(-1)), 0);
    assert(pcm.every(Number.isFinite));
    assert(Math.max(...pcm.map(Math.abs)) <= 0.05);
    assert(pcm.length <= 48000 * 0.08);
    assert(Math.max(...pcm.map(Math.abs)) > 0.005, "cue contains signal");
  }
  assert.throws(() => renderCue("move", Infinity), RangeError);
  assert.throws(() => renderCue("move", 1000000000), RangeError);
});

test("mute, volume zero, and game hold immediately stop every active cue", () => {
  for (const mode of ["mute", "volume", "hold"]) {
    const f = fixture(); f.input(); f.sound.play("confirm");
    if (mode === "mute") f.sound.toggle();
    else if (mode === "volume") f.sound.setVolume(0);
    else f.sound.hold(true);
    assert(f.sources.every(source => source.stopped && source.disconnected), mode);
    f.at(1000); assert.equal(f.sound.play("back"), false, mode);
  }
  const f = fixture({ preference: { "gamestrike-sounds": "off" } });
  f.input(); assert.equal(f.sound.play("confirm"), false); assert.equal(f.constructions, 0);
});

test("capture, hidden windows, invalid volume, and disposal stay silent", () => {
  const f = fixture(); f.input(); f.suppress(true);
  assert.equal(f.sound.play("move"), false); assert.equal(f.constructions, 0);
  f.suppress(false); f.sound.play("move");
  f.document.hidden = true; f.listeners.get("visibilitychange")();
  assert(f.sources.every(source => source.stopped));
  f.at(500); assert.equal(f.sound.play("move"), false);
  f.document.hidden = false; f.sound.setVolume(9); assert.equal(f.sound.volume, 1);
  f.sound.setVolume(NaN); assert.equal(f.sound.volume, 1);
  f.sound.dispose(); assert.equal(f.listeners.size, 0); assert.equal(f.context.closed, true);
  assert.equal(f.sound.play("confirm"), false);
});

test("resume races cannot replay a cue after mute, hidden, hold, capture, or a stale input", async () => {
  for (const interruption of ["mute", "hidden", "hold", "capture", "stale"]) {
    const f = fixture({ state: "suspended" }); f.input();
    let resume; f.context.resume = () => new Promise(resolve => { resume = () => { f.context.state = "running"; resolve(); }; });
    assert.equal(f.sound.play("confirm"), true);
    assert.equal(f.sound.play("confirm"), false);
    if (interruption === "mute") f.sound.toggle();
    else if (interruption === "hidden") { f.document.hidden = true; f.listeners.get("visibilitychange")(); }
    else if (interruption === "hold") f.sound.hold(true);
    else if (interruption === "capture") f.suppress(true);
    else f.at(121);
    resume(); await new Promise(setImmediate);
    assert.equal(f.events.length, 0, interruption);
  }
});
