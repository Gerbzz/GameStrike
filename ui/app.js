// The Collection style: a dense cover grid with a filter sidebar inside one bevelled frame, a game
// detail view with the cover in a 3D cradle, and the bottom bar with the selected game and prompts.
// Talks to the trusted Rust launcher only through window.ipc and window.GameStrike.receive().
(() => {
  "use strict";
  if (document.documentElement.hasAttribute("data-leaving")) return;

  const core = window.GameStrikeCore;
  const $ = (id) => document.getElementById(id);
  const send = (message) => window.ipc.postMessage(JSON.stringify(message));
  const SORTS = [["recent", "Recent"], ["title-asc", "Title A–Z"], ["title-desc", "Title Z–A"], ["added", "Newest"]];
  const BOOT_MS = 3200;
  // The WebGL startup (boot.js): the frame starts assembling at 4.2 s and is at rest at 5.5 s.
  const CINEMA = { assemble: 4200, end: 5500 };
  const reduceMotion = () => Boolean(window.matchMedia?.("(prefers-reduced-motion: reduce)")?.matches) || document.body.classList.contains("reduce-motion");

  const stored = (key, fallback) => { try { return localStorage.getItem(key) ?? fallback; } catch (_) { return fallback; } };
  const store = (key, value) => { try { localStorage.setItem(key, value); } catch (_) { /* optional preference */ } };

  const state = {
    loaded: false,
    booted: false,
    games: [],
    problems: [],
    running: null,
    filter: "all",
    query: "",
    sort: stored("gamestrike-sort", "recent"),
    style: core.styleFor(stored("gamestrike-style", "collection-orange")),
    zone: "grid", // "grid" or "sidebar"
    gridIndex: 0,
    sideIndex: 1,
    detailId: null,
    saves: false,
    visible: [],
    filters: [],
    importing: false,
    launching: false,
    stores: [],
    scanning: false,
    refreshRequested: false,
    dialog: null,
    capturing: false,
    metrics: core.collectionMetrics(innerWidth, innerHeight),
  };
  if (!SORTS.some(([key]) => key === state.sort)) state.sort = "recent";
  document.body.dataset.variant = state.style.variant || "orange";

  const el = {
    boot: $("boot"), grid: $("grid"), filters: $("filters"), search: $("search"), sidebar: $("sidebar"),
    title: $("view-title"), count: $("view-count"), empty: $("empty"), toast: $("toast"),
    detail: $("detail"), frame: $("frame"), frameArt: $("frame-art"),
    nowTitle: $("now-title"), nowStatus: $("now-status"), nowIcon: $("now-icon"),
  };

  // ---------- interface sounds (synthesized locally, never fetched) ----------
  // Original sounds, built from oscillators and filtered noise; no samples are used. The
  // AudioContext starts only after the player's first key, click, or button press.
  const sound = (() => {
    let enabled = stored("gamestrike-sounds", "on") !== "off";
    let context = null;
    let master = null;
    function ready() {
      if (!enabled || !window.AudioContext || state.capturing) return null;
      if (!context) {
        context = new AudioContext();
        master = context.createDynamicsCompressor();
        master.connect(context.destination);
      }
      if (context.state === "suspended") context.resume().catch(() => {});
      return context;
    }
    function voice(at, { from, to = from, length, volume = 0.05, type = "sawtooth", cutoff = 2400, q = 6, out = null }) {
      const ctx = context;
      const gain = ctx.createGain();
      const filter = ctx.createBiquadFilter();
      filter.type = "lowpass";
      filter.Q.value = q;
      filter.frequency.setValueAtTime(cutoff, at);
      filter.frequency.exponentialRampToValueAtTime(Math.max(120, cutoff * 0.25), at + length);
      gain.gain.setValueAtTime(0.0001, at);
      gain.gain.exponentialRampToValueAtTime(volume, at + 0.008);
      gain.gain.exponentialRampToValueAtTime(0.0001, at + length);
      for (const detune of [-7, 7]) {
        const osc = ctx.createOscillator();
        osc.type = type;
        osc.detune.value = detune;
        osc.frequency.setValueAtTime(from, at);
        osc.frequency.exponentialRampToValueAtTime(to, at + length);
        osc.connect(filter);
        osc.start(at);
        osc.stop(at + length + 0.03);
      }
      filter.connect(gain).connect(out || master);
    }
    function whoosh(at, length, from, to, volume, out = null) {
      const ctx = context;
      const buffer = ctx.createBuffer(1, Math.ceil(ctx.sampleRate * length), ctx.sampleRate);
      const data = buffer.getChannelData(0);
      for (let i = 0; i < data.length; i += 1) data[i] = Math.random() * 2 - 1;
      const source = ctx.createBufferSource();
      source.buffer = buffer;
      const filter = ctx.createBiquadFilter();
      filter.type = "bandpass";
      filter.Q.value = 1.4;
      filter.frequency.setValueAtTime(from, at);
      filter.frequency.exponentialRampToValueAtTime(to, at + length);
      const gain = ctx.createGain();
      gain.gain.setValueAtTime(0.0001, at);
      gain.gain.exponentialRampToValueAtTime(volume, at + length * 0.35);
      gain.gain.exponentialRampToValueAtTime(0.0001, at + length);
      source.connect(filter).connect(gain).connect(out || master);
      source.start(at);
    }
    // ---- the startup's sounds: a low swell, a shimmer as the core forms, the impact of the
    // flash, mechanical clunks as the frame bolts together, and a pressure hiss as the tubes
    // connect. Scheduled against the startup's own clock; a skip fades the bus out.
    let bootBus = null;
    function noise(at, length, { type = "bandpass", from = 800, to = from, q = 1, volume = 0.05, attack = 0.005, out }) {
      const ctx = context;
      const buffer = ctx.createBuffer(1, Math.ceil(ctx.sampleRate * length), ctx.sampleRate);
      const data = buffer.getChannelData(0);
      for (let i = 0; i < data.length; i += 1) data[i] = Math.random() * 2 - 1;
      const source = ctx.createBufferSource();
      source.buffer = buffer;
      const filter = ctx.createBiquadFilter();
      filter.type = type;
      filter.Q.value = q;
      filter.frequency.setValueAtTime(from, at);
      filter.frequency.exponentialRampToValueAtTime(Math.max(40, to), at + length);
      const gain = ctx.createGain();
      gain.gain.setValueAtTime(0.0001, at);
      gain.gain.exponentialRampToValueAtTime(volume, at + attack);
      gain.gain.exponentialRampToValueAtTime(0.0001, at + length);
      source.connect(filter).connect(gain).connect(out);
      source.start(at);
    }
    function swell(at, length, out) {
      const ctx = context;
      const filter = ctx.createBiquadFilter();
      filter.type = "lowpass";
      filter.Q.value = 5;
      filter.frequency.setValueAtTime(130, at);
      filter.frequency.exponentialRampToValueAtTime(2600, at + length);
      const gain = ctx.createGain();
      gain.gain.setValueAtTime(0.0001, at);
      gain.gain.exponentialRampToValueAtTime(0.055, at + length * 0.9);
      gain.gain.exponentialRampToValueAtTime(0.0001, at + length + 0.2);
      for (const [freq, detune, type] of [[55, -9, "sawtooth"], [55, 9, "sawtooth"], [82.41, 0, "sawtooth"], [110, 4, "triangle"]]) {
        const osc = ctx.createOscillator();
        osc.type = type;
        osc.detune.value = detune;
        osc.frequency.setValueAtTime(freq, at);
        osc.frequency.exponentialRampToValueAtTime(freq * 1.5, at + length);
        osc.connect(filter);
        osc.start(at);
        osc.stop(at + length + 0.3);
      }
      filter.connect(gain).connect(out);
    }
    function shimmer(at, length, out) {
      const ctx = context;
      [1318.5, 1760, 2093, 2637].forEach((freq, i) => {
        const osc = ctx.createOscillator();
        osc.type = "sine";
        osc.frequency.value = freq;
        const trem = ctx.createOscillator();
        trem.frequency.value = 5 + i * 1.7;
        const depth = ctx.createGain();
        depth.gain.value = 0.006;
        const gain = ctx.createGain();
        gain.gain.setValueAtTime(0.0001, at);
        gain.gain.exponentialRampToValueAtTime(0.012, at + length * 0.7);
        gain.gain.exponentialRampToValueAtTime(0.0001, at + length);
        trem.connect(depth).connect(gain.gain);
        osc.connect(gain).connect(out);
        osc.start(at + i * 0.12);
        trem.start(at);
        osc.stop(at + length + 0.05);
        trem.stop(at + length + 0.05);
      });
    }
    function bootSounds(start) {
      try {
        if (!ready()) return;
        const schedule = () => {
          if (context.state !== "running" || bootBus === false) return;
          const elapsed = (performance.now() - start) / 1000;
          if (elapsed > 2.5) return;
          const t0 = context.currentTime - elapsed;
          const at = (seconds) => Math.max(context.currentTime + 0.01, t0 + seconds);
          bootBus = context.createGain();
          bootBus.connect(master);
          const out = bootBus;
          swell(at(0.15), 2.85, out);
          shimmer(at(1.2), 1.9, out);
          noise(at(3.0), 0.9, { type: "lowpass", from: 2400, to: 160, q: 0.7, volume: 0.14, out });
          voice(at(3.0), { from: 52, to: 30, length: 0.7, volume: 0.16, type: "sine", cutoff: 400, q: 1, out });
          voice(at(3.02), { from: 1046.5, length: 0.55, volume: 0.03, type: "triangle", cutoff: 6000, out });
          for (const time of [4.45, 4.62, 4.78, 4.92, 5.05]) {
            noise(at(time), 0.09, { from: 420, to: 260, q: 2.2, volume: 0.08, out });
            voice(at(time), { from: 96, to: 58, length: 0.13, volume: 0.1, type: "sine", cutoff: 500, q: 1, out });
          }
          for (const time of [5.12, 5.17, 5.22, 5.27]) noise(at(time), 0.035, { from: 2600, q: 3, volume: 0.035, out });
          noise(at(4.85), 0.65, { from: 5200, to: 3200, q: 0.8, volume: 0.032, attack: 0.03, out });
        };
        if (context.state === "running") schedule();
        else context.resume().then(schedule).catch(() => {});
      } catch (_) { /* sound is decoration only */ }
    }
    function stopBoot() {
      if (bootBus && context) bootBus.gain.setTargetAtTime(0.0001, context.currentTime, 0.04);
      bootBus = false;
    }
    const SOUNDS = {
      move: (t) => voice(t, { from: 1320, to: 990, length: 0.05, volume: 0.025, type: "triangle", cutoff: 5200 }),
      zone: (t) => { voice(t, { from: 520, to: 780, length: 0.12, volume: 0.03, type: "triangle" }); whoosh(t, 0.18, 600, 2400, 0.02); },
      select: (t) => { voice(t, { from: 660, length: 0.09, volume: 0.04 }); voice(t + 0.07, { from: 990, length: 0.16, volume: 0.04 }); },
      open: (t) => { whoosh(t, 0.35, 300, 3200, 0.035); voice(t + 0.05, { from: 440, to: 880, length: 0.28, volume: 0.03, type: "triangle", cutoff: 3000 }); },
      back: (t) => { voice(t, { from: 740, to: 370, length: 0.16, volume: 0.035, type: "triangle", cutoff: 2600 }); whoosh(t, 0.2, 2400, 500, 0.015); },
      launch: (t) => {
        whoosh(t, 0.9, 200, 5200, 0.05);
        [523.25, 659.25, 783.99, 1046.5].forEach((note, i) => voice(t + i * 0.075, { from: note, length: 0.5 - i * 0.06, volume: 0.035, cutoff: 3800 }));
        voice(t, { from: 65.4, to: 130.8, length: 0.9, volume: 0.05, type: "sine", cutoff: 800, q: 1 });
      },
      error: (t) => { voice(t, { from: 220, length: 0.12, volume: 0.045, type: "square", cutoff: 900 }); voice(t + 0.13, { from: 196, length: 0.18, volume: 0.045, type: "square", cutoff: 700 }); },
    };
    function play(kind) {
      if (kind === "boot") return;
      try {
        if (!ready()) return;
        (SOUNDS[kind] || SOUNDS.move)(context.currentTime + 0.005);
        started = true;
        hum(true);
      } catch (_) { /* sound is decoration only */ }
    }
    // The dome's ambient hum: two low, slightly detuned tones and a breath of filtered noise under
    // a slow swell, very quiet. It starts with the first sound, and stops while a game runs, while
    // the window is hidden, and when sounds are off.
    let drone = null;
    let held = false;
    let started = false;
    function hum(on) {
      try {
        const want = on && enabled && !held && !document.hidden;
        if (want && !drone && ready()) {
          const ctx = context;
          const gain = ctx.createGain();
          gain.gain.setValueAtTime(0.0001, ctx.currentTime);
          gain.gain.exponentialRampToValueAtTime(0.014, ctx.currentTime + 2.5);
          const filter = ctx.createBiquadFilter();
          filter.type = "lowpass";
          filter.frequency.value = 340;
          const swell = ctx.createOscillator();
          const depth = ctx.createGain();
          swell.frequency.value = 0.09;
          depth.gain.value = 0.005;
          swell.connect(depth).connect(gain.gain);
          const sources = [55, 82.6, 110.3].map((frequency, index) => {
            const osc = ctx.createOscillator();
            osc.type = index === 2 ? "triangle" : "sine";
            osc.frequency.value = frequency;
            const level = ctx.createGain();
            level.gain.value = index === 2 ? 0.25 : 0.6;
            osc.connect(level).connect(filter);
            return osc;
          });
          const noise = ctx.createBufferSource();
          const buffer = ctx.createBuffer(1, ctx.sampleRate * 2, ctx.sampleRate);
          const data = buffer.getChannelData(0);
          for (let i = 0; i < data.length; i += 1) data[i] = (Math.random() * 2 - 1) * 0.18;
          noise.buffer = buffer;
          noise.loop = true;
          noise.connect(filter);
          filter.connect(gain).connect(master);
          [...sources, noise, swell].forEach((node) => node.start());
          drone = { gain, nodes: [...sources, noise, swell] };
        } else if (!want && drone) {
          const { gain, nodes } = drone;
          drone = null;
          const end = context.currentTime + 0.8;
          gain.gain.cancelScheduledValues(context.currentTime);
          gain.gain.setValueAtTime(Math.max(0.0001, gain.gain.value), context.currentTime);
          gain.gain.exponentialRampToValueAtTime(0.0001, end);
          nodes.forEach((node) => node.stop(end + 0.05));
        }
      } catch (_) { /* sound is decoration only */ }
    }
    document.addEventListener("visibilitychange", () => { if (started) hum(!document.hidden); });
    return {
      play,
      bootSounds,
      stopBoot,
      get enabled() { return enabled; },
      toggle() { enabled = !enabled; store("gamestrike-sounds", enabled ? "on" : "off"); if (enabled) play("select"); else hum(false); },
      // While a game runs the launcher stays quiet.
      hold(on) { held = on; if (started) hum(!on); },
    };
  })();

  // ---------- 3D scene ----------
  // Requests the scene may have missed because it started later; scene.js reads them when it starts.
  const scenePending = (window.GameStrikeScenePending = {});
  // scene.js hears where the camera should go, which cover the cradle holds, and pause requests.
  function setView(view, instant = false) {
    document.body.dataset.view = view;
    window.dispatchEvent(new CustomEvent("gamestrike:view", { detail: { view, instant } }));
  }
  function pauseScene(paused) {
    sound.hold(paused);
    window.dispatchEvent(new CustomEvent("gamestrike:pause", { detail: { paused } }));
  }
  function cradle(game) {
    if (!game) return;
    const url = core.coverUrl(game);
    const canvas = window.GameStrikeCovers?.canvasFor(game, kickerFor(game), 352, 472);
    // Kept as well as sent: on a slow start (macOS in CI) the scene can start after this.
    scenePending.cradle = { id: game.id, url, canvas };
    window.dispatchEvent(new CustomEvent("gamestrike:cradle", { detail: scenePending.cradle }));
    const card = document.querySelector(".cradle-card");
    if (card) card.replaceChildren(coverNode(game, 352, 472));
  }

  // ---------- layout ----------
  // Scales the frame to the window (measured from the reference at 1672 x 941) and redraws it.
  let frameQueued = false;
  function drawFrame() {
    if (frameQueued) return;
    frameQueued = true;
    requestAnimationFrame(() => {
      frameQueued = false;
      window.GameStrikeFrame?.draw(el.frameArt, el.frame);
      window.dispatchEvent(new CustomEvent("gamestrike:layout", { detail: { frame: el.frame.getBoundingClientRect() } }));
    });
  }
  function applyMetrics() {
    state.metrics = core.collectionMetrics(innerWidth, innerHeight);
    const rootStyle = document.documentElement.style;
    rootStyle.setProperty("--u", String(state.metrics.u));
    rootStyle.setProperty("--t", String(state.metrics.t));
    el.grid.style.setProperty("--cols", String(state.metrics.columns));
    drawFrame();
  }

  // ---------- startup ----------
  // With WebGL the startup is the 3D sequence in boot.js; the page shows the wordmark, then at
  // 4.2 s reveals the frame while its header, rows, covers, and bottom bar slide and flip in. Without
  // WebGL it is the earlier CSS sequence; with reduced motion, a short fade.
  let bootTimer = null;
  let assembleTimer = null;
  function signalScene(detail) {
    window.GameStrikeScenePending = Object.assign(window.GameStrikeScenePending || {}, { boot: detail });
    window.dispatchEvent(new CustomEvent("gamestrike:boot", { detail }));
  }
  // Staggers for the assembly: each row and each row of covers starts a little later.
  function stagger() {
    [...document.querySelectorAll(".sidebar .row")].forEach((row, index) => row.style.setProperty("--i", String(index)));
    const columns = state.metrics.columns || 7;
    [...el.grid.children].forEach((node, index) => node.style.setProperty("--row", String(Math.floor(index / columns))));
  }
  function endAssembly() {
    clearTimeout(assembleTimer);
    document.body.classList.remove("assembling");
    delete document.body.dataset.boot;
  }
  function skipBoot() {
    sound.stopBoot();
    signalScene({ action: "skip" });
    if (!state.booted) finishBoot();
    endAssembly();
  }
  function finishBoot({ assemble = false } = {}) {
    if (state.booted) return;
    state.booted = true;
    clearTimeout(bootTimer);
    if (assemble) {
      document.body.classList.add("assembling");
      stagger();
    }
    el.boot.classList.add("done");
    setView(state.detailId ? "detail" : "library", state.capturing);
    drawFrame();
    if (state.loaded) render({ focus: !state.dialog });
  }
  function startBoot() {
    let switched = false;
    try { switched = sessionStorage.getItem("gamestrike-skip-boot") === "1"; sessionStorage.removeItem("gamestrike-skip-boot"); } catch (_) { /* plays the boot */ }
    const cinema = !switched && !reduceMotion() && Boolean(window.WebGL2RenderingContext);
    if (cinema) {
      document.body.dataset.boot = "cinema";
      const start = performance.now();
      signalScene({ action: "play", start });
      sound.bootSounds(start);
      bootTimer = setTimeout(() => finishBoot({ assemble: true }), CINEMA.assemble);
      assembleTimer = setTimeout(endAssembly, CINEMA.end + 200);
    } else {
      signalScene({ action: "skip" });
      bootTimer = setTimeout(finishBoot, switched || reduceMotion() ? 0 : BOOT_MS);
      // Reduced motion: no flight and no assembly, only a short fade in.
      if (!switched && reduceMotion()) document.getElementById("stage")?.animate?.([{ opacity: 0 }, { opacity: 1 }], { duration: 400, easing: "ease-out" });
    }
    for (const type of ["keydown", "mousedown"]) {
      window.addEventListener(type, (event) => {
        if (state.booted && !document.body.classList.contains("assembling")) return;
        event.preventDefault();
        event.stopImmediatePropagation();
        skipBoot();
      }, { capture: true });
    }
  }

  // ---------- helpers ----------
  const gameById = (id) => state.games.find((game) => game.id === id) || null;
  const isStore = (game) => game?.kind === "store";
  const kindLabel = (game) => (game.source_kind === "zip" ? "ZIP archive" : "HTML file");
  const kickerFor = (game) => (isStore(game) ? core.storeName(game.store) : core.cleanSource(game.source) || kindLabel(game));
  const label = (list, key) => list.find(([k]) => k === key)?.[1] || "";
  const plural = (count, one, many) => `${count} ${count === 1 ? one : many}`;

  function toast(message, isError = false) {
    el.toast.textContent = message;
    el.toast.classList.toggle("error", isError);
    el.toast.hidden = false;
    if (isError) sound.play("error");
    clearTimeout(toast.timer);
    toast.timer = setTimeout(() => { el.toast.hidden = true; }, isError ? 9000 : 5000);
  }

  // GameStrike's own line icons. No store or platform logos: GameStrike supports none of them.
  const ICONS = {
    all: '<path d="M4 4h7v7H4zM13 4h7v7h-7zM4 13h7v7H4zM13 13h7v7h-7z"/>',
    recent: '<circle cx="12" cy="12" r="8.5"/><path d="M12 7v5l3.5 2"/>',
    star: '<path d="m12 3.5 2.6 5.4 5.9.8-4.3 4.1 1 5.8L12 16.8l-5.2 2.8 1-5.8-4.3-4.1 5.9-.8z"/>',
    file: '<path d="M6 3.5h8l4 4V20.5H6z"/><path d="M14 3.5v4h4M9.5 12.5 7.5 14.5l2 2M14.5 12.5l2 2-2 2"/>',
    zip: '<path d="M6 3.5h12v17H6z"/><path d="M12 3.5v2M12 7.5v2M12 11.5v2M10.5 15.5h3v3h-3z"/>',
    globe: '<circle cx="12" cy="12" r="8.5"/><path d="M3.5 12h17M12 3.5c3 3 3 14 0 17M12 3.5c-3 3-3 14 0 17"/>',
    tag: '<path d="M3.5 12.5V4.5h8l9 9-8 8z"/><circle cx="8" cy="8.5" r="1.5"/>',
    pad: '<path d="M7 8h10a4 4 0 0 1 4 4.5l-.6 3A2.6 2.6 0 0 1 16 17l-2-2h-4l-2 2a2.6 2.6 0 0 1-4.4-1.5l-.6-3A4 4 0 0 1 7 8z"/><path d="M8 11v3M6.5 12.5h3M15.5 12h.01M17.5 13.5h.01"/>',
    bolt: '<path d="M13 3 5 13.5h6L10 21l8-10.5h-6z"/>',
    // Store rows. Original glyphs: vapour, a shield, a diamond, a bag, an orbit, a hexagon, an
    // arrow, and a window. None imitates a store's logo.
    "store-steam": '<path d="M7 20c-2-3 2-5 0-8s2-5 0-8M12 20c-2-3 2-5 0-8s2-5 0-8M17 20c-2-3 2-5 0-8s2-5 0-8"/>',
    "store-epic": '<path d="M12 3 19.5 6v6c0 4.4-3.2 7.6-7.5 9-4.3-1.4-7.5-4.6-7.5-9V6z"/><path d="M12 8v8"/>',
    "store-ea": '<path d="M12 3l9 9-9 9-9-9z"/><path d="M12 8l4 4-4 4-4-4z"/>',
    "store-xbox": '<path d="M5 8h14l-1.2 12.5H6.2z"/><path d="M9 8V6.5a3 3 0 0 1 6 0V8"/>',
    "store-gog": '<circle cx="12" cy="12" r="3.5"/><ellipse cx="12" cy="12" rx="9.5" ry="4" transform="rotate(-20 12 12)"/>',
    "store-ubisoft": '<path d="M12 3l7.8 4.5v9L12 21l-7.8-4.5v-9z"/><path d="M12 8.5l3 1.7v3.6l-3 1.7-3-1.7v-3.6z"/>',
    "store-battlenet": '<path d="M5 19 16 8M9 5h10v10"/><path d="M4 15.5l4.5 4.5"/>',
    "store-custom": '<path d="M4 5h16v14H4z"/><path d="M4 9h16M7.5 12.5l2 1.5-2 1.5M12 15.5h4"/>',
    html: '<path d="M8.5 8 4.5 12l4 4M15.5 8l4 4-4 4M13.5 5.5l-3 13"/>',
    hidden: '<path d="M3 12s3.5-6 9-6 9 6 9 6-3.5 6-9 6-9-6-9-6z"/><circle cx="12" cy="12" r="2.5"/><path d="M4 20 20 4"/>',
  };
  const SOURCE_ICONS = ["globe", "tag", "pad", "bolt"];
  function iconFor(filter) {
    if (filter.icon === "tag") {
      let hash = 0;
      for (const char of filter.label) hash = (hash * 31 + char.charCodeAt(0)) >>> 0;
      return ICONS[SOURCE_ICONS[hash % SOURCE_ICONS.length]];
    }
    return ICONS[filter.icon] || ICONS.tag;
  }
  function svgIcon(markup, className = "row-icon") {
    const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
    svg.setAttribute("viewBox", "0 0 24 24");
    svg.setAttribute("class", className);
    svg.setAttribute("aria-hidden", "true");
    svg.innerHTML = markup;
    return svg;
  }

  // ---------- sidebar ----------
  function sidebarItems() {
    return [document.querySelector(".search-row"), ...el.filters.querySelectorAll(".row"), $("add-game"), $("settings"), $("power")];
  }

  function renderFilters() {
    state.filters = core.buildFilters(state.games);
    if (!state.filters.some((filter) => filter.key === state.filter)) state.filter = "all";
    const nodes = state.filters.map((filter) => {
      const button = document.createElement("button");
      button.type = "button";
      button.className = "row";
      button.dataset.filter = filter.key;
      button.tabIndex = -1;
      button.setAttribute("aria-current", String(filter.key === state.filter));
      button.setAttribute("aria-label", `${filter.label}, ${plural(filter.count, "game", "games")}`);
      const name = document.createElement("span");
      name.className = "row-label";
      name.textContent = filter.label;
      const count = document.createElement("span");
      count.className = "count";
      count.textContent = String(filter.count);
      button.append(svgIcon(iconFor(filter)), name, count);
      button.addEventListener("click", () => { setFilter(filter.key); state.zone = "grid"; layoutFocus(true); });
      return button;
    });
    el.filters.replaceChildren(...nodes);
    requestAnimationFrame(fitFilters);
  }

  // With many stores and sources the filter list scrolls: show the scroll marks and keep the chosen
  // filter in view, again whenever the list changes size (window resizes, the frame's scale).
  function fitFilters() {
    el.sidebar.classList.toggle("scrolls", el.filters.scrollHeight > el.filters.clientHeight + 2);
    // Only the list scrolls (scrollIntoView could also move the page while the sidebar is away).
    const list = el.filters;
    const current = list.querySelector('[aria-current="true"]');
    if (!current) return;
    const top = current.offsetParent === list ? current.offsetTop : current.offsetTop - list.offsetTop;
    const bottom = top + current.offsetHeight;
    if (top < list.scrollTop) list.scrollTop = Math.max(0, top - 8);
    else if (bottom > list.scrollTop + list.clientHeight) list.scrollTop = bottom - list.clientHeight + 8;
  }

  function setFilter(key) {
    if (state.filter === key) return;
    state.filter = key;
    state.gridIndex = 0;
    sound.play("select");
    render();
  }

  // ---------- grid ----------
  function renderGrid() {
    state.visible = core.applyFilter(state.games, state.filter, state.query, state.sort);
    const filter = state.filters.find((entry) => entry.key === state.filter);
    el.title.textContent = state.query ? `Search: ${state.query}` : state.filter === "all" ? "Game Collection" : filter?.label || "Game Collection";
    el.count.textContent = `${plural(state.visible.length, "game", "games")} · ${label(SORTS, state.sort)}${state.scanning ? " · Scanning stores" : ""}`;
    el.grid.replaceChildren(...state.visible.map((game, index) => tile(game, index)));
    if (document.body.classList.contains("assembling")) stagger();
    const empty = !state.visible.length;
    el.empty.hidden = !empty || !state.loaded;
    el.grid.hidden = empty;
    if (empty) {
      const none = !state.games.length;
      $("empty-title").textContent = none ? (state.scanning ? "Looking for your games" : "No games found yet") : state.query ? "No games match" : "Nothing here yet";
      $("empty-copy").textContent = none
        ? "GameStrike looks for games installed with Steam, Epic Games, the EA app, the Microsoft Store, GOG Galaxy, Ubisoft Connect, and Battle.net, and found none. Add an HTML game or a program, or refresh the store libraries."
        : state.query ? `No game title contains “${state.query}”.` : state.filter === "favorites" ? "Mark a game as a favorite in its details." : "Play a game and it will show up here.";
      $("empty-action").textContent = none ? "Add a game" : state.query ? "Clear search" : "Show all games";
      $("empty-action").dataset.action = none ? "add" : state.query ? "clear" : "all";
    }
    state.gridIndex = Math.min(Math.max(state.gridIndex, 0), Math.max(0, state.visible.length - 1));
  }

  // A cover: the stored image when there is one, otherwise box art drawn from the title.
  function coverNode(game, width, height) {
    const url = core.coverUrl(game);
    if (url) {
      const image = document.createElement("img");
      image.alt = "";
      image.decoding = "async";
      image.src = url;
      image.addEventListener("error", () => image.replaceWith(generated(game, width, height)), { once: true });
      return image;
    }
    return generated(game, width, height);
  }
  function generated(game, width, height) {
    const ratio = Math.min(2, window.devicePixelRatio || 1);
    if (!window.GameStrikeCovers) return document.createElement("canvas");
    return window.GameStrikeCovers.canvasFor(game, kickerFor(game), width * ratio, height * ratio);
  }
  function tileSize() {
    const first = el.grid.querySelector(".tile-art");
    if (first?.clientWidth) return [first.clientWidth, first.clientHeight];
    const width = (el.grid.clientWidth || 900) / state.metrics.columns;
    return [Math.round(width * 0.88), Math.round(width * 0.88 * 236 / 176)];
  }

  function tile(game, index) {
    const node = document.createElement("button");
    node.type = "button";
    node.className = "tile";
    node.dataset.index = String(index);
    node.dataset.id = game.id;
    node.setAttribute("role", "option");
    node.tabIndex = -1;
    const frame = document.createElement("span");
    frame.className = "tile-frame";
    const art = document.createElement("span");
    art.className = "tile-art";
    const badges = document.createElement("span");
    badges.className = "tile-badges";
    if (game.favorite) badges.append(badge("fav", "★"));
    if (game.id === state.running) badges.append(badge("playing", "Playing"));
    if (isStore(game) && !game.installed) badges.append(badge("missing", "Not installed"));
    else if (!game.files_ok) badges.append(badge("missing", "Files missing"));
    node.append(frame, art, badges);
    node.setAttribute("aria-label", `${game.title}. ${statusFor(game)}.${game.favorite ? " Favorite." : ""}`);
    node.addEventListener("click", () => {
      if (state.zone === "grid" && Number(node.dataset.index) === state.gridIndex) launch(game);
      else { state.zone = "grid"; state.gridIndex = Number(node.dataset.index); sound.play("move"); layoutFocus(true); }
    });
    node.addEventListener("dblclick", () => launch(game, false));
    node.addEventListener("contextmenu", (event) => { event.preventDefault(); state.zone = "grid"; state.gridIndex = Number(node.dataset.index); openDetail(); });
    return node;
  }
  // Covers are painted once the tiles have a size, nearest first.
  function paintCovers() {
    const [width, height] = tileSize();
    const tiles = [...el.grid.children];
    tiles.forEach((node, index) => {
      const art = node.querySelector(".tile-art");
      if (!art || art.firstChild) return;
      const game = state.visible[index];
      if (game) art.append(coverNode(game, width, height));
    });
  }

  function badge(kind, text) {
    const node = document.createElement("span");
    node.className = `badge ${kind}`;
    node.textContent = text;
    return node;
  }

  function gridColumns() {
    const tiles = el.grid.children;
    if (tiles.length < 2) return 1;
    const top = tiles[0].offsetTop;
    let columns = 0;
    for (const node of tiles) { if (node.offsetTop !== top) break; columns += 1; }
    return Math.max(1, columns);
  }

  function statusFor(game) {
    return core.gameStatus(game, state.running);
  }

  // The bottom bar: the selected game's title and its status, or the focused sidebar row.
  function updateNow() {
    const game = focusedGame();
    const status = el.nowStatus.parentElement;
    status.classList.remove("warn");
    if (state.zone === "sidebar" && !state.detailId) {
      const node = sidebarItems()[state.sideIndex];
      el.nowTitle.textContent = node?.querySelector(".row-label")?.textContent || (node?.classList.contains("search-row") ? "Search games" : "");
      el.nowStatus.textContent = node?.dataset.filter ? `${node.querySelector(".count")?.textContent || 0} games` : node?.classList.contains("search-row") ? "Type to search by title" : "";
      el.nowIcon.innerHTML = ICONS.all;
      return;
    }
    if (!game) {
      el.nowTitle.textContent = state.games.length ? "" : "No games yet";
      el.nowStatus.textContent = state.games.length ? "" : "Add a game to begin";
      return;
    }
    el.nowTitle.textContent = game.title;
    const plays = game.play_count ? ` · ${plural(game.play_count, "play", "plays")}` : "";
    el.nowStatus.textContent = `${statusFor(game)}${plays}`;
    status.classList.toggle("warn", !game.files_ok);
    el.nowIcon.innerHTML = isStore(game) ? ICONS[`store-${game.store}`] : game.source_kind === "zip" ? ICONS.zip : ICONS.file;
  }

  // Prompts for the current context; keys after keyboard input, ringed buttons after pad input.
  function updatePrompts() {
    const set = (button, text, key, on = true) => {
      $(`label-${button}`).textContent = text;
      if (key) $(`key-${button}`).textContent = key;
      document.querySelector(`.prompt[data-button="${button}"]`).classList.toggle("off", !on);
    };
    const game = focusedGame();
    if (state.detailId) {
      set("a", "Select", "Enter");
      set("b", "Back", "Esc");
      if (isStore(game)) set("x", "Folder", "O", Boolean(game?.install_path));
      else set("x", "Network", "N", !state.running);
      set("y", "Favorite", "F");
      set("menu", "Details", "I", false);
    } else if (state.zone === "sidebar") {
      set("a", "Select", "Enter");
      set("b", "Back", "Esc");
      set("x", "Games", "Tab", state.visible.length > 0);
      set("y", "Sort", "S", state.filter !== "recent");
      set("menu", "Details", "I", false);
    } else {
      set("a", "Launch", "Enter", Boolean(game) && (isStore(game) ? game.installed : game.files_ok));
      set("b", "Back", "Esc");
      set("x", "Filter", "Tab");
      set("y", "Sort", "S", state.filter !== "recent");
      set("menu", "Details", "I", Boolean(game));
    }
  }

  // Moves the visible focus (and keyboard focus) to the current zone's item.
  function layoutFocus(focus = true) {
    document.body.dataset.zone = state.zone;
    const tiles = [...el.grid.children];
    const inGrid = state.zone === "grid" && tiles.length > 0;
    el.grid.classList.toggle("has-focus", inGrid);
    tiles.forEach((node, index) => {
      const on = inGrid && index === state.gridIndex;
      node.classList.toggle("focused", on);
      node.setAttribute("aria-selected", String(index === state.gridIndex));
      node.tabIndex = index === state.gridIndex ? 0 : -1;
    });
    const items = sidebarItems();
    state.sideIndex = Math.min(Math.max(state.sideIndex, 0), items.length - 1);
    items.forEach((node, index) => {
      node.classList.toggle("focused", state.zone === "sidebar" && index === state.sideIndex);
      if (node.parentElement === el.filters) node.tabIndex = state.zone === "sidebar" && index === state.sideIndex ? 0 : -1;
    });
    updateNow();
    updatePrompts();
    if (!focus || state.dialog || state.detailId || !state.booted) return;
    const target = state.zone === "grid" ? tiles[state.gridIndex] : items[state.sideIndex];
    if (target) {
      const focusTarget = target.classList.contains("search-row") ? el.search : target;
      focusTarget.focus({ preventScroll: true });
      target.scrollIntoView({ block: "nearest", inline: "nearest", behavior: reduceMotion() || state.capturing ? "auto" : "smooth" });
    } else if (state.zone === "grid") {
      $("empty-action").focus({ preventScroll: true });
    }
  }

  function focusedGame() {
    if (state.detailId) return gameById(state.detailId);
    return state.zone === "grid" ? state.visible[state.gridIndex] || null : null;
  }

  function render({ focus = true } = {}) {
    const keep = state.zone === "grid" ? state.visible[state.gridIndex]?.id : null;
    renderFilters();
    renderGrid();
    if (keep) {
      const again = state.visible.findIndex((game) => game.id === keep);
      if (again >= 0) state.gridIndex = again;
    }
    if (state.detailId && !gameById(state.detailId)) closeDetail({ restore: false, silent: true });
    if (state.detailId) renderDetail();
    layoutFocus(focus);
    requestAnimationFrame(paintCovers);
  }

  // ---------- navigation ----------
  function navigate(direction) {
    if (state.detailId) return moveWithin(el.detail, direction);
    if (state.zone === "sidebar") {
      const items = sidebarItems();
      if (direction === "right") { if (state.visible.length) { state.zone = "grid"; sound.play("zone"); } }
      else state.sideIndex = core.listMove(state.sideIndex, items.length, direction);
      if (direction === "up" || direction === "down") sound.play("move");
      return layoutFocus(true);
    }
    const columns = gridColumns();
    if (direction === "left" && state.gridIndex % columns === 0) return toSidebar();
    const next = core.gridMove(state.gridIndex, state.visible.length, columns, direction);
    if (next >= 0 && next !== state.gridIndex) { state.gridIndex = next; sound.play("move"); }
    layoutFocus(true);
  }

  function toSidebar() {
    state.zone = "sidebar";
    state.sideIndex = Math.max(0, sidebarItems().findIndex((node) => node.dataset.filter === state.filter));
    sound.play("zone");
    layoutFocus(true);
  }
  function toggleZone() {
    if (state.zone === "sidebar") { if (state.visible.length) { state.zone = "grid"; sound.play("zone"); layoutFocus(true); } }
    else toSidebar();
  }

  function moveWithin(container, direction) {
    const items = [...container.querySelectorAll("button, input")].filter((node) => !node.disabled && node.offsetParent !== null && !node.closest("[hidden]"));
    if (!items.length) return;
    const index = items.indexOf(document.activeElement);
    const step = direction === "left" || direction === "up" ? -1 : 1;
    items[(index + step + items.length) % items.length].focus();
    sound.play("move");
  }

  function activate() {
    if (state.zone === "sidebar") {
      const node = sidebarItems()[state.sideIndex];
      if (!node) return;
      if (node.classList.contains("search-row")) { el.search.focus(); el.search.select(); return; }
      if (node.dataset.filter) { setFilter(node.dataset.filter); state.zone = "grid"; layoutFocus(true); }
      else node.click();
      return;
    }
    if (focusedGame()) launch(focusedGame());
    else $("empty-action").click();
  }

  function back() {
    if (state.detailId) return state.saves ? showSaves(false) : closeDetail();
    if (state.zone === "sidebar") { if (state.visible.length) { state.zone = "grid"; sound.play("back"); layoutFocus(true); } return; }
    if (state.query) { setQuery(""); sound.play("back"); return; }
    if (state.filter !== "all") { setFilter("all"); sound.play("back"); }
  }

  function setQuery(query) {
    state.query = query.trim().slice(0, 80);
    if (el.search.value.trim() !== state.query) el.search.value = state.query;
    state.gridIndex = 0;
    render({ focus: false });
  }

  // ---------- detail: the cover in its cradle, actions, and the info panel ----------
  function openDetail() {
    const game = state.zone === "grid" ? state.visible[state.gridIndex] : null;
    if (!game) return;
    state.detailId = game.id;
    state.saves = false;
    renderDetail();
    cradle(game);
    el.detail.removeAttribute("inert");
    el.detail.setAttribute("aria-hidden", "false");
    setView("detail");
    sound.play("open");
    $("d-play").focus({ preventScroll: true });
    updateNow();
    updatePrompts();
  }

  function closeDetail({ restore = true, silent = false } = {}) {
    if (!state.detailId) return;
    state.detailId = null;
    state.saves = false;
    $("saves").hidden = true;
    $("info").hidden = false;
    el.detail.setAttribute("inert", "");
    el.detail.setAttribute("aria-hidden", "true");
    if (!state.dialog) setView("library");
    if (!silent) sound.play("back");
    renderGrid();
    paintCovers();
    if (restore) layoutFocus(true);
  }

  function showSaves(on) {
    state.saves = on;
    $("saves").hidden = !on;
    $("info").hidden = on;
    window.dispatchEvent(new CustomEvent("gamestrike:view", { detail: { view: on ? "saves" : "detail", instant: false } }));
    sound.play(on ? "open" : "back");
    renderDetail();
    (on ? $("s-clear").disabled ? $("s-remove") : $("s-clear") : $("d-saves")).focus({ preventScroll: true });
  }

  // A fact marked "path" keeps its real case (paths are case-sensitive on Linux and macOS) and
  // spans both columns.
  function facts(list) {
    return list.map(([name, value, kind]) => {
      const box = document.createElement("div");
      const dt = document.createElement("dt");
      dt.textContent = name;
      const dd = document.createElement("dd");
      dd.textContent = value;
      if (kind === "path") {
        box.className = "wide";
        dd.className = "path";
      }
      box.append(dt, dd);
      return box;
    });
  }

  function renderDetail() {
    const game = gameById(state.detailId);
    if (!game) return;
    // The header tab carries the game, as the reference list panel carries its collection name.
    el.title.textContent = game.title;
    el.count.textContent = `${state.visible.findIndex((g) => g.id === game.id) + 1} of ${state.visible.length}`;
    $("detail-kicker").textContent = kickerFor(game);
    $("detail-title").textContent = game.title;
    const store = isStore(game);
    const custom = store && game.store === "custom";
    for (const id of ["d-network", "d-source", "d-saves"]) $(id).hidden = store;
    for (const id of ["d-folder", "d-hide"]) $(id).hidden = !store;
    for (const id of ["d-args", "d-workdir", "d-remove-program"]) $(id).hidden = !custom;
    $("detail-facts").replaceChildren(...facts(store ? [
      ["Store", core.storeName(game.store)],
      [custom ? "Program folder" : "Install folder", game.install_path || "Not found", game.install_path ? "path" : ""],
      ["Plays", String(game.play_count || 0)],
      ["Last played", core.formatWhen(game.last_played)],
      ...(custom ? [["Arguments", (game.args || []).join(" ") || "None", game.args?.length ? "path" : ""], ["Working folder", game.working_dir || "The program's folder", game.working_dir ? "path" : ""]] : [["Starts with", game.launch_kind === "uri" ? core.storeName(game.store) : "Its program"]]),
      ["Found", game.added_at ? core.formatWhen(game.added_at) : "Earlier"],
    ] : [
      ["Source", core.cleanSource(game.source) || "Not set"],
      ["Imported as", kindLabel(game)],
      ["Plays", String(game.play_count || 0)],
      ["Last played", core.formatWhen(game.last_played)],
      ["Game data", core.formatBytes(game.save_bytes)],
      ["Added", game.added_at ? core.formatWhen(game.added_at) : "Earlier"],
    ]));
    const note = $("detail-note");
    note.classList.toggle("warn", !game.files_ok);
    const coverText = game.cover === "custom" ? "your image" : game.cover === "auto" ? "from the game's files" : game.cover === "store" ? `${core.storeName(game.store)}'s own cached art` : "drawn by GameStrike from the title";
    note.textContent = store
      ? (game.installed ? `Cover: ${coverText}. GameStrike counts its own launches; it does not track play time.` : `Not installed. Install it with ${core.storeName(game.store)}, then refresh the store libraries.`)
      : !game.files_ok ? "This game's files are missing. Remove it, then add the original file again."
      : game.id === state.running ? "Playing now in its own window."
      : state.running ? "Another game is open. Close it to play this one."
      : `Cover: ${coverText}. Controller and network needs are not checked.`;
    const busy = Boolean(state.running) || state.launching;
    $("d-play").textContent = store ? "Play" : "Play offline";
    $("d-play").disabled = store ? !game.installed : busy || !game.files_ok;
    $("d-network").disabled = busy || !game.files_ok;
    $("d-folder").disabled = !game.install_path;
    $("d-hide").textContent = game.hidden ? "Show in library" : "Hide game";
    $("d-favorite").textContent = game.favorite ? "★ Favorite" : "☆ Add to favorites";
    $("d-favorite").setAttribute("aria-pressed", String(Boolean(game.favorite)));
    $("d-undo-cover").disabled = !game.cover_undo;
    $("d-reset-cover").disabled = game.cover !== "custom";
    $("saves-title").textContent = game.title;
    $("saves-facts").replaceChildren(...facts([
      ["Game data", core.formatBytes(game.save_bytes)],
      ["Stored as", "Local storage, IndexedDB, cookies"],
      ["Last played", core.formatWhen(game.last_played)],
      ["Plays", String(game.play_count || 0)],
    ]));
    $("s-clear").disabled = !game.save_bytes || game.id === state.running;
    $("s-remove").disabled = game.id === state.running;
  }

  // ---------- actions ----------
  function importGame() {
    if (state.importing) return;
    state.importing = true;
    sound.play("open");
    send({ type: "pick_and_import" });
  }

  function launch(game, network = false) {
    if (!game) return;
    if (isStore(game)) {
      // Store games start through their own launcher (or the program the player chose).
      if (!game.installed) { toast(`${game.title} is not installed. Install it with ${core.storeName(game.store)}, then refresh the store libraries.`, true); return; }
      sound.play("launch");
      send({ type: "launch_store_game", id: game.id });
      return;
    }
    if (state.running) { toast(game.id === state.running ? "This game is already open in its own window." : "Another game is open. Close it first.", true); return; }
    if (state.launching) return;
    if (!game.files_ok) { toast("This game's files are missing. Remove it, then add the original file again.", true); return; }
    state.launching = true;
    sound.play("launch");
    send({ type: "launch_game", id: game.id, network });
  }

  function toggleFavorite(game) {
    if (!game) return;
    sound.play("select");
    send({ type: "set_favorite", id: game.id, favorite: !game.favorite });
  }

  function cycleSort() {
    if (state.filter === "recent") return;
    state.sort = SORTS[(SORTS.findIndex(([key]) => key === state.sort) + 1) % SORTS.length][0];
    if (!state.capturing) store("gamestrike-sort", state.sort);
    sound.play("select");
    render({ focus: false });
    toast(`Sorted by ${label(SORTS, state.sort).toLowerCase()}.`);
  }

  // Applies a style. The earlier styles live on classic.html, so choosing one opens that page.
  function applyStyle(style, persist) {
    state.style = style;
    if (persist && !state.capturing) store("gamestrike-style", style.key);
    if (style.page === "classic") {
      if (!persist) return;
      try { sessionStorage.setItem("gamestrike-skip-boot", "1"); } catch (_) { /* the boot then plays once */ }
      location.replace("/classic.html");
      return;
    }
    document.body.dataset.variant = style.variant;
    window.GameStrikeCovers?.clearCache();
    window.dispatchEvent(new CustomEvent("gamestrike:variant", { detail: { variant: style.variant } }));
    drawFrame();
  }

  // ---------- dialogs ----------
  const dialogs = {
    network: { node: $("network-dialog"), first: "network-cancel" },
    rename: { node: $("rename-dialog"), first: "rename-input" },
    source: { node: $("source-dialog"), first: "source-input" },
    reset: { node: $("reset-dialog"), first: "reset-cancel" },
    remove: { node: $("remove-dialog"), first: "remove-cancel" },
    settings: { node: $("settings-dialog"), first: "settings-style" },
    problems: { node: $("problems-dialog"), first: "problems-close" },
    about: { node: $("about-dialog"), first: "close-about" },
    add: { node: $("add-dialog"), first: "add-program" },
    stores: { node: $("stores-dialog"), first: "stores-refresh" },
    args: { node: $("args-dialog"), first: "args-input" },
  };
  const GAME_DIALOGS = ["network", "rename", "source", "reset", "remove", "args"];
  let dialogGame = null;
  let returnFocus = null;

  function settingsText() {
    $("settings-style").textContent = `Style: ${state.style.label}`;
    $("settings-sound").textContent = `Sounds: ${sound.enabled ? "On" : "Off"}`;
    $("settings-sound").setAttribute("aria-pressed", String(sound.enabled));
  }

  function openDialog(name) {
    const game = GAME_DIALOGS.includes(name) ? focusedGame() : null;
    if (GAME_DIALOGS.includes(name) && !game) return;
    if (name === "network" && (state.running || state.launching || !game.files_ok)) { if (state.running) toast("Another game is open. Close it first.", true); return; }
    if ((name === "reset" || name === "remove") && game.id === state.running) return;
    if (name === "reset" && !game.save_bytes) return;
    if (state.dialog) closeDialog({ restore: false, silent: true });
    else returnFocus = document.activeElement;
    dialogGame = game;
    if (name === "network") $("network-title").textContent = `Play ${game.title} with network?`;
    if (name === "rename") { $("rename-input").value = game.title; $("rename-error").hidden = true; }
    if (name === "source") { $("source-input").value = game.source || ""; $("source-error").hidden = true; $("source-title").textContent = `Where is ${game.title} from?`; }
    if (name === "reset") $("reset-title").textContent = `Clear saves for ${game.title}?`;
    if (name === "remove") {
      $("remove-title").textContent = `Remove ${game.title}?`;
      $("remove-keep").checked = true;
      $("remove-keep-row").hidden = isStore(game);
      $("remove-copy").textContent = isStore(game)
        ? "GameStrike forgets this program and its cover and plays. The program itself is not touched."
        : "GameStrike deletes its copy of the game files. Your original download is not touched.";
    }
    if (name === "args") { $("args-input").value = (game.args || []).map((arg) => (/\s/.test(arg) ? `"${arg}"` : arg)).join(" "); $("args-title").textContent = `Arguments for ${game.title}`; }
    if (name === "stores") renderStoreSettings();
    if (name === "settings") {
      settingsText();
      $("settings-problems").hidden = !state.problems.length;
      $("settings-problems").textContent = `${plural(state.problems.length, "unreadable folder", "unreadable folders")}`;
    }
    if (name === "problems") {
      $("problems-list").replaceChildren(...state.problems.map((problem) => {
        const li = document.createElement("li");
        li.textContent = problem;
        return li;
      }));
    }
    state.dialog = name;
    dialogs[name].node.hidden = false;
    window.dispatchEvent(new CustomEvent("gamestrike:view", { detail: { view: { settings: "settings", problems: "settings", about: "about" }[name] || (state.detailId ? "saves" : "settings"), instant: state.capturing } }));
    sound.play("open");
    const first = $(dialogs[name].first);
    first.focus();
    if (name === "rename" || name === "source" || name === "args") first.select();
  }

  function closeDialog({ restore = true, silent = false } = {}) {
    if (!state.dialog) return;
    dialogs[state.dialog].node.hidden = true;
    state.dialog = null;
    window.dispatchEvent(new CustomEvent("gamestrike:view", { detail: { view: state.detailId ? (state.saves ? "saves" : "detail") : "library", instant: false } }));
    if (!silent) sound.play("back");
    if (!restore) return;
    if (state.detailId && returnFocus && el.detail.contains(returnFocus) && !returnFocus.disabled) returnFocus.focus();
    else if (state.detailId) $("d-play").focus();
    else layoutFocus(true);
  }

  function moveDialogFocus(step) {
    if (!state.dialog) return;
    moveWithin(dialogs[state.dialog].node, step < 0 ? "up" : "down");
  }

  $("network-cancel").addEventListener("click", () => closeDialog());
  $("network-confirm").addEventListener("click", () => { const game = dialogGame; closeDialog({ silent: true }); closeDetail({ restore: false, silent: true }); launch(game, true); layoutFocus(true); });
  $("rename-cancel").addEventListener("click", () => closeDialog());
  $("rename-form").addEventListener("submit", (event) => {
    event.preventDefault();
    const game = dialogGame;
    const title = core.cleanTitle($("rename-input").value);
    if (!game || !title) { $("rename-error").hidden = false; $("rename-input").focus(); sound.play("error"); return; }
    closeDialog({ silent: true });
    sound.play("select");
    if (title !== game.title) send({ type: "rename_game", id: game.id, title });
  });
  $("source-cancel").addEventListener("click", () => closeDialog());
  $("source-form").addEventListener("submit", (event) => {
    event.preventDefault();
    const game = dialogGame;
    const raw = $("source-input").value;
    const source = raw.trim() ? core.cleanSource(raw) : "";
    if (!game || source === null) { $("source-error").hidden = false; $("source-input").focus(); sound.play("error"); return; }
    closeDialog({ silent: true });
    sound.play("select");
    if (source !== (game.source || "")) send({ type: "set_source", id: game.id, source });
  });
  $("reset-cancel").addEventListener("click", () => closeDialog());
  $("reset-confirm").addEventListener("click", () => { const game = dialogGame; closeDialog({ silent: true }); if (game) send({ type: "reset_saves", id: game.id }); });
  $("remove-cancel").addEventListener("click", () => closeDialog());
  $("remove-confirm").addEventListener("click", () => {
    const game = dialogGame;
    const keep = $("remove-keep").checked;
    closeDialog({ silent: true });
    closeDetail({ restore: false, silent: true });
    if (game && isStore(game)) send({ type: "remove_program", id: game.id });
    else if (game) send({ type: "remove_game", id: game.id, keep_saves: keep });
    layoutFocus(true);
  });
  $("settings-style").addEventListener("click", () => { sound.play("select"); applyStyle(core.nextStyle(state.style.key), true); settingsText(); });
  $("settings-sound").addEventListener("click", () => { sound.toggle(); settingsText(); });
  $("settings-problems").addEventListener("click", () => openDialog("problems"));
  $("settings-stores").addEventListener("click", () => openDialog("stores"));
  // Add game: an HTML game, a program, or a fresh look at the store libraries.
  function refreshStores() {
    state.refreshRequested = true;
    sound.play("select");
    send({ type: "refresh_stores" });
  }
  $("add-html").addEventListener("click", () => { closeDialog({ silent: true }); importGame(); });
  $("add-program").addEventListener("click", () => { closeDialog({ silent: true }); sound.play("open"); send({ type: "add_program" }); });
  $("add-refresh").addEventListener("click", () => { closeDialog({ silent: true }); refreshStores(); });
  $("add-close").addEventListener("click", () => closeDialog());
  $("stores-refresh").addEventListener("click", () => { closeDialog({ silent: true }); refreshStores(); });
  $("stores-close").addEventListener("click", () => closeDialog());
  // Settings: each store can be turned off (it is then not scanned or shown).
  function renderStoreSettings() {
    const list = $("stores-list");
    list.replaceChildren(...state.stores.map((status) => {
      const button = document.createElement("button");
      button.type = "button";
      button.className = "slab";
      button.disabled = !status.supported;
      const where = status.supported ? (status.enabled ? `${plural(status.games, "game", "games")}` : "Off") : "Windows only";
      button.textContent = `${status.name}: ${status.enabled ? "On" : "Off"} · ${where}`;
      button.setAttribute("aria-pressed", String(status.enabled));
      button.addEventListener("click", () => {
        status.enabled = !status.enabled;
        sound.play("select");
        send({ type: "set_store_enabled", store: status.key, enabled: status.enabled });
        renderStoreSettings();
        $("stores-list").querySelector(`[data-store="${status.key}"]`)?.focus();
      });
      button.dataset.store = status.key;
      return button;
    }));
  }
  $("args-cancel").addEventListener("click", () => closeDialog());
  $("args-form").addEventListener("submit", (event) => {
    event.preventDefault();
    const game = dialogGame;
    const value = $("args-input").value;
    if (!game || value.length > 600) { $("args-error").hidden = false; sound.play("error"); return; }
    closeDialog({ silent: true });
    sound.play("select");
    send({ type: "set_program", id: game.id, args: value });
  });
  $("settings-about").addEventListener("click", () => openDialog("about"));
  $("settings-close").addEventListener("click", () => closeDialog());
  $("problems-close").addEventListener("click", () => closeDialog());
  $("close-about").addEventListener("click", () => closeDialog());
  for (const { node } of Object.values(dialogs)) {
    node.addEventListener("mousedown", (event) => { if (event.target === node) closeDialog(); });
  }

  // Detail actions.
  const detailGame = () => gameById(state.detailId);
  $("d-play").addEventListener("click", () => { const game = detailGame(); closeDetail({ restore: false, silent: true }); launch(game, false); layoutFocus(true); });
  $("d-network").addEventListener("click", () => openDialog("network"));
  $("d-favorite").addEventListener("click", () => toggleFavorite(detailGame()));
  $("d-cover").addEventListener("click", () => { const game = detailGame(); if (game) { sound.play("open"); send({ type: "pick_cover", id: game.id }); } });
  $("d-undo-cover").addEventListener("click", () => { const game = detailGame(); if (game) send({ type: "undo_cover", id: game.id }); });
  $("d-reset-cover").addEventListener("click", () => { const game = detailGame(); if (game) send({ type: "reset_cover", id: game.id }); });
  $("d-source").addEventListener("click", () => openDialog("source"));
  $("d-rename").addEventListener("click", () => openDialog("rename"));
  $("d-saves").addEventListener("click", () => showSaves(true));
  $("d-close").addEventListener("click", () => closeDetail());
  $("d-folder").addEventListener("click", () => { const game = detailGame(); if (game) send({ type: "open_install_folder", id: game.id }); });
  $("d-hide").addEventListener("click", () => {
    const game = detailGame();
    if (!game) return;
    sound.play("select");
    send({ type: "set_hidden", id: game.id, hidden: !game.hidden });
    if (!game.hidden) closeDetail();
  });
  $("d-args").addEventListener("click", () => openDialog("args"));
  $("d-workdir").addEventListener("click", () => { const game = detailGame(); if (game) { sound.play("open"); send({ type: "pick_working_folder", id: game.id }); } });
  $("d-remove-program").addEventListener("click", () => openDialog("remove"));
  $("s-clear").addEventListener("click", () => openDialog("reset"));
  $("s-remove").addEventListener("click", () => openDialog("remove"));
  $("s-back").addEventListener("click", () => showSaves(false));

  // Sidebar controls.
  $("add-game").addEventListener("click", () => openDialog("add"));
  $("settings").addEventListener("click", () => openDialog("settings"));
  $("power").addEventListener("click", () => { if (state.running) toast("Close the game first.", true); else send({ type: "quit" }); });
  el.search.addEventListener("input", () => setQuery(el.search.value));
  el.search.addEventListener("focus", () => { state.zone = "sidebar"; state.sideIndex = 0; document.body.dataset.zone = "sidebar"; });
  el.search.addEventListener("keydown", (event) => {
    if (event.key === "Enter" || event.key === "ArrowDown") { event.preventDefault(); event.stopPropagation(); if (event.key === "ArrowDown") { state.sideIndex = 1; layoutFocus(true); } else if (state.visible.length) { state.zone = "grid"; layoutFocus(true); } }
    if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); if (el.search.value) setQuery(""); else if (state.visible.length) { state.zone = "grid"; layoutFocus(true); } }
  });
  $("empty-action").addEventListener("click", () => {
    const action = $("empty-action").dataset.action;
    if (action === "add") openDialog("add");
    else if (action === "clear") setQuery("");
    else setFilter("all");
  });
  for (const prompt of document.querySelectorAll(".prompt")) prompt.addEventListener("click", () => handleAction(prompt.dataset.button));

  // ---------- keyboard ----------
  document.addEventListener("keydown", (event) => {
    document.body.dataset.input = "keyboard";
    if (state.dialog) {
      if (event.key === "Escape") { event.preventDefault(); closeDialog(); return; }
      if (event.key === "Tab") { event.preventDefault(); moveDialogFocus(event.shiftKey ? -1 : 1); return; }
      const typing = document.activeElement?.tagName === "INPUT" && document.activeElement.type !== "checkbox";
      if (!typing && (event.key === "ArrowLeft" || event.key === "ArrowUp")) { event.preventDefault(); moveDialogFocus(-1); }
      if (!typing && (event.key === "ArrowRight" || event.key === "ArrowDown")) { event.preventDefault(); moveDialogFocus(1); }
      return;
    }
    if (document.activeElement === el.search) return;
    if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "f") { event.preventDefault(); focusSearch(); return; }
    if (event.ctrlKey || event.altKey || event.metaKey) return;
    if (state.detailId) {
      if (event.key === "Escape" || event.key === "Backspace") { event.preventDefault(); back(); return; }
      if (event.key === "Tab") { event.preventDefault(); moveWithin(el.detail, event.shiftKey ? "up" : "down"); return; }
      const arrows = { ArrowLeft: "left", ArrowRight: "right", ArrowUp: "up", ArrowDown: "down" };
      if (arrows[event.key]) { event.preventDefault(); moveWithin(el.detail, arrows[event.key]); return; }
      if (event.key.toLowerCase() === "f") { event.preventDefault(); toggleFavorite(detailGame()); }
      if (event.key.toLowerCase() === "n" && !isStore(detailGame())) { event.preventDefault(); openDialog("network"); }
      if (event.key.toLowerCase() === "o" && isStore(detailGame())) { event.preventDefault(); $("d-folder").click(); }
      return;
    }
    const arrows = { ArrowLeft: "left", ArrowRight: "right", ArrowUp: "up", ArrowDown: "down", Home: "home", End: "end" };
    if (arrows[event.key]) { event.preventDefault(); navigate(arrows[event.key]); return; }
    switch (event.key) {
      case "Enter": case " ": event.preventDefault(); activate(); break;
      case "Tab": event.preventDefault(); toggleZone(); break;
      case "i": case "I": case "ContextMenu": event.preventDefault(); if (state.zone === "grid") openDetail(); break;
      case "n": case "N": event.preventDefault(); if (state.zone === "grid" && !isStore(focusedGame())) openDialog("network"); break;
      case "f": case "F": event.preventDefault(); if (state.zone === "grid") toggleFavorite(focusedGame()); break;
      case "s": case "S": event.preventDefault(); cycleSort(); break;
      case "F2": event.preventDefault(); if (state.zone === "grid") openDialog("rename"); break;
      case "Delete": event.preventDefault(); if (state.zone === "grid") openDialog("remove"); break;
      case "/": event.preventDefault(); focusSearch(); break;
      case "m": case "M": event.preventDefault(); sound.toggle(); toast(`Sounds ${sound.enabled ? "on" : "off"}.`); break;
      case "Escape": case "Backspace": event.preventDefault(); back(); break;
    }
  });

  function focusSearch() {
    state.zone = "sidebar";
    state.sideIndex = 0;
    layoutFocus(false);
    el.search.focus();
    el.search.select();
  }

  // ---------- controller (launcher navigation only; games handle their own input) ----------
  const readPad = core.createPadReader();
  function pollPad(now) {
    const pad = Array.from(navigator.getGamepads?.() || []).find(Boolean) || null;
    const actions = readPad(pad, now);
    if (actions.length && (!state.booted || document.body.classList.contains("assembling"))) { skipBoot(); requestAnimationFrame(pollPad); return; }
    if (actions.length && !state.running && document.hasFocus()) {
      for (const action of actions) { document.body.dataset.input = "pad"; handleAction(action); }
    }
    requestAnimationFrame(pollPad);
  }

  // One action from a controller button or a clicked prompt.
  function handleAction(action) {
    if (state.dialog) {
      if (action === "b") closeDialog();
      else if (action === "a") document.activeElement?.click();
      else if (action === "left" || action === "up") moveDialogFocus(-1);
      else if (action === "right" || action === "down") moveDialogFocus(1);
      return;
    }
    if (state.detailId) {
      if (action === "b") back();
      else if (action === "a") document.activeElement?.click();
      else if (action === "x") { if (isStore(detailGame())) $("d-folder").click(); else openDialog("network"); }
      else if (action === "y") toggleFavorite(detailGame());
      else if (["left", "right", "up", "down"].includes(action)) moveWithin(el.detail, action);
      return;
    }
    if (["left", "right", "up", "down"].includes(action)) navigate(action);
    else if (action === "a") activate();
    else if (action === "x") toggleZone();
    else if (action === "y") cycleSort();
    else if (action === "menu") { if (state.zone === "grid") openDetail(); }
    else if (action === "b") back();
  }

  // ---------- messages from the trusted Rust launcher ----------
  function receive(message) {
    switch (message.type) {
      case "library": {
        state.games = core.mergeLibrary(message.games, message.store_games);
        state.stores = Array.isArray(message.stores) ? message.stores : [];
        state.scanning = message.scanning === true;
        state.problems = Array.isArray(message.problems) ? message.problems : [];
        $("synthetic").hidden = message.synthetic !== true;
        state.running = message.running || null;
        pauseScene(Boolean(state.running));
        state.loaded = true;
        render({ focus: !state.dialog && state.booted && !state.detailId && document.activeElement !== el.search });
        if (state.detailId) cradle(gameById(state.detailId));
        if (state.pendingFocus) {
          const index = state.visible.findIndex((game) => game.id === state.pendingFocus);
          state.pendingFocus = null;
          if (index >= 0) { state.zone = "grid"; state.gridIndex = index; layoutFocus(true); }
        }
        if (state.dialog === "stores") renderStoreSettings();
        drawFrame();
        break;
      }
      case "stores_refreshed":
        if (state.refreshRequested) toast(`Store libraries refreshed: ${plural(message.installed, "installed game", "installed games")} found.`);
        state.refreshRequested = false;
        break;
      case "store_started":
        toast(message.message);
        break;
      case "program_added":
        state.query = "";
        el.search.value = "";
        state.filter = "all";
        state.pendingFocus = message.id;
        sound.play("select");
        toast(message.message);
        break;
      case "importing":
        toast(`Adding ${message.name}…`);
        break;
      case "pick_cancelled":
        state.importing = false;
        break;
      case "imported":
        state.importing = false;
        state.query = "";
        el.search.value = "";
        state.filter = "all";
        state.pendingFocus = message.game.id;
        sound.play("select");
        toast(message.already_present
          ? `${message.game.title} is already in your library. Its saves were kept.`
          : message.restored_saves
            ? `${message.game.title} is back, with the saves you kept.`
            : `${message.game.title} was added to your library.`);
        break;
      case "player_started":
        state.launching = false;
        state.running = message.id;
        pauseScene(true);
        toast(message.network ? "Game opened with network access." : "Game opened offline in its own window.");
        render();
        break;
      case "player_exited":
        if (state.running === message.id) state.running = null;
        pauseScene(Boolean(state.running));
        state.launching = false;
        toast(message.message, !message.ok);
        render({ focus: !state.dialog && !state.detailId });
        break;
      case "managed":
      case "removed":
        toast(message.message);
        break;
      case "error":
        if (message.operation === "import") state.importing = false;
        if (message.operation === "launch") state.launching = false;
        toast(message.message, true);
        break;
    }
  }

  // Used only by the launcher's explicit --capture-view option to stage a screenshot. Captures
  // start from the default look and never change saved preferences.
  function capture(view) {
    state.capturing = true;
    applyStyle(core.styleFor(view === "green" ? "collection-green" : "collection-orange"), false);
    const bootSecond = view === "boot" ? 3.5 : /^boot-\d+(\.\d+)?$/.test(view) ? Number(view.slice(5)) : null;
    if (bootSecond !== null) {
      clearTimeout(bootTimer);
      clearTimeout(assembleTimer);
      sound.stopBoot();
      // The interface may exist even when the scene could not acquire a GL context.
      if (!window.WebGL2RenderingContext || ["static", "none"].includes(document.body.dataset.scene)) {
        delete document.body.dataset.boot;
        el.boot.style.setProperty("--seek", "2.9s");
        el.boot.classList.remove("done");
        el.boot.classList.add("frozen");
        setView("boot", true);
        return;
      }
      state.bootCapture = bootSecond;
      document.body.dataset.boot = "cinema";
      document.body.classList.add("boot-frozen");
      document.documentElement.style.setProperty("--seek", `${bootSecond}s`);
      document.documentElement.style.setProperty("--aseek", `${Math.max(0, bootSecond - CINEMA.assemble / 1000)}s`);
      signalScene({ action: "freeze", t: bootSecond });
      // The page's startup animations began when the page loaded; restart them, held, so that
      // their delays (which count from the startup's first frame, less --seek) place this second.
      // Timers, not animation frames: a capture window may be hidden, and hidden webviews pause them.
      // Only seek our CSS startup animations. getAnimations() also returns transitions;
      // rewinding the stage's opacity transition hides the library in WKWebView captures.
      const hold = () => {
        void document.body.offsetWidth;
        for (const animation of document.getAnimations?.() || []) {
          if (!animation.animationName?.startsWith("gs-")) continue;
          animation.pause();
          animation.currentTime = 0;
        }
      };
      hold();
      for (const delay of [0, 40, 120]) setTimeout(hold, delay);
      if (bootSecond >= CINEMA.assemble / 1000) {
        state.booted = false;
        finishBoot({ assemble: true });
        if (bootSecond >= CINEMA.end / 1000) endAssembly();
      } else {
        el.boot.classList.remove("done");
        setView("boot", true);
      }
      return;
    }
    // A fast machine reaches capture mode while the 3D boot is still playing. Finish the
    // scene as well as the page, otherwise the screenshot contains its central emblem.
    skipBoot();
    document.body.classList.add("reduce-motion");
    // Controller prompts are the default look; the focus view shows the keyboard prompts instead.
    document.body.dataset.input = view === "focus" ? "keyboard" : "pad";
    state.filter = view === "favorites" ? "favorites" : view === "stores" ? "store:steam" : "all";
    if (view === "filters") {
      const source = core.buildFilters(state.games).find((filter) => filter.key.startsWith("source:"));
      if (source) state.filter = source.key;
    }
    if (view === "search") { el.search.value = "orbit"; setQuery("orbit"); }
    state.zone = view === "filters" ? "sidebar" : "grid";
    state.gridIndex = 0;
    render({ focus: false });
    if (view === "filters") state.sideIndex = Math.max(0, sidebarItems().findIndex((node) => node.dataset.filter === state.filter));
    if (view === "long") {
      let best = 0;
      state.visible.forEach((game, index) => { if (game.title.length > state.visible[best].title.length) best = index; });
      state.gridIndex = best;
    } else if (view === "store-detail" || view === "stores") {
      // A store game with the store's own cached art.
      const covered = state.visible.findIndex((game) => isStore(game) && game.cover === "store");
      state.gridIndex = Math.max(0, covered);
    } else if (view === "focus" || view === "controller" || view === "detail" || view === "saves" || view === "moving" || view === "launch") {
      // A cover away from the edge, preferring one with a stored image.
      const covered = state.visible.findIndex((game) => game.cover === "custom");
      state.gridIndex = covered > 0 ? covered : Math.min(1, state.visible.length - 1);
    }
    layoutFocus(false);
    el.grid.children[state.gridIndex]?.scrollIntoView({ block: "nearest", inline: "nearest" });
    paintCovers();
    if (view === "detail" || view === "saves" || view === "launch" || view === "moving" || view === "store-detail") openDetail();
    if (view === "add-menu") openDialog("add");
    if (view === "store-settings") openDialog("stores");
    if (view === "saves") showSaves(true);
    if (view === "launch") openDialog("network");
    if (view === "settings") openDialog("settings");
    if (view === "about") openDialog("about");
    if (view === "moving") {
      // Halfway through the flight from the grid to the cradle, frozen for the screenshot.
      document.body.classList.add("mid-move");
      scenePending.freeze = { from: "library", to: "detail", progress: 0.5 };
      window.dispatchEvent(new CustomEvent("gamestrike:freeze", { detail: scenePending.freeze }));
      return;
    }
    // Captures show the camera at its station, not partway through a flight.
    window.dispatchEvent(new CustomEvent("gamestrike:view", { detail: { view: state.saves ? "saves" : document.body.dataset.view, instant: true } }));
    drawFrame();
  }

  // Used only by --capture-view: true once a staged view has drawn (fonts, the covers in view,
  // and the 3D scene or its fallback), so the launcher never captures a half-drawn view.
  let readySince = 0;
  function ready() {
    if (!state.capturing) return true;
    if (!drawn()) { readySince = 0; return false; }
    // Ready only once it has stayed drawn across two checks, so the last image has painted.
    if (!readySince) readySince = performance.now();
    return performance.now() - readySince >= 200;
  }
  function drawn() {
    if (document.fonts && document.fonts.status !== "loaded") return false;
    if (!document.body.dataset.scene) return false;
    // A late capture can return to an early boot frame after natural startup has finished.
    // That view intentionally hides the library; every staged library/detail view must show it.
    if (document.body.dataset.view !== "boot") {
      const stage = getComputedStyle(el.frame.parentElement);
      if (Number(stage.opacity) < 0.99 || stage.visibility === "hidden" || stage.display === "none") return false;
    }
    if (state.bootCapture !== undefined && document.body.dataset.scene === "webgl") {
      if (document.body.dataset.bootFrame !== String(state.bootCapture)) return false;
      // The emblem shows the logo from 3 s to 5 s; wait until its image has decoded.
      if (state.bootCapture >= 3 && state.bootCapture < 5 && document.body.dataset.bootLogo !== "ready") return false;
    }
    if (state.detailId && document.body.dataset.scene === "webgl" && (!document.body.dataset.cradle || document.body.dataset.cradle === "loading")) return false;
    if (state.booted && !el.grid.hidden) {
      const tiles = [...el.grid.children].slice(0, state.metrics.columns * 4);
      for (const node of tiles) {
        const art = node.querySelector(".tile-art")?.firstChild;
        if (!art || (art.tagName === "IMG" && !(art.complete && art.naturalWidth > 0))) return false;
      }
    }
    return true;
  }

  window.GameStrike = Object.freeze({ receive, capture, ready });
  window.addEventListener("resize", () => { applyMetrics(); requestAnimationFrame(() => { for (const art of el.grid.querySelectorAll(".tile-art")) art.replaceChildren(); paintCovers(); }); });
  if (window.ResizeObserver) new ResizeObserver(fitFilters).observe(el.filters);
  // Covers and the frame use the bundled fonts; draw again once they have loaded.
  document.fonts?.ready?.then(() => { window.GameStrikeCovers?.clearCache(); for (const art of el.grid.querySelectorAll(".tile-art")) art.replaceChildren(); paintCovers(); drawFrame(); });
  applyMetrics();
  startBoot();
  requestAnimationFrame(pollPad);
  send({ type: "ready" });
})();
