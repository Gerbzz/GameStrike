(() => {
  "use strict";

  const core = window.GameStrikeCore;
  const $ = (id) => document.getElementById(id);
  const send = (message) => window.ipc.postMessage(JSON.stringify(message));
  const THEMES = [["lime", "Lime"], ["ocean", "Ocean"], ["sunset", "Sunset"], ["grape", "Grape"], ["graphite", "Graphite"]];
  // Dashboard styles: the blades layout, the 2001-era orb and pill menus, and the blades layout in PWDS orange.
  const STYLES = [["blades", "Blades"], ["original", "Original"], ["pwds", "PWDS Orange"]];
  const BOOT_MS = { blades: 3400, original: 3900, pwds: 4200 };
  const SORTS = [["recent", "Recently played"], ["title-asc", "Title A–Z"], ["title-desc", "Title Z–A"], ["added", "Recently added"]];

  const stored = (key, fallback) => { try { return localStorage.getItem(key) ?? fallback; } catch (_) { return fallback; } };
  const store = (key, value) => { try { localStorage.setItem(key, value); } catch (_) { /* optional preference */ } };

  const state = {
    loaded: false,
    games: [],
    problems: [],
    running: null,
    query: "",
    sort: stored("gamestrike-sort", "recent"),
    theme: stored("gamestrike-theme", "lime"),
    style: stored("gamestrike-style", "blades"),
    booted: false,
    channels: [],
    channel: 0,
    focus: {},
    importing: false,
    launching: false,
    dialog: null,
    returnFocus: null,
  };
  if (!SORTS.some(([key]) => key === state.sort)) state.sort = "recent";
  if (!THEMES.some(([key]) => key === state.theme)) state.theme = "lime";
  if (!STYLES.some(([key]) => key === state.style)) state.style = "blades";
  document.body.dataset.theme = state.theme;
  document.body.dataset.style = state.style;

  const el = {
    boot: $("boot"), og: $("og"), ogHeader: $("og-header"), ogList: $("og-list"), ogDetail: $("og-detail"), channels: $("channels"), stage: $("stage"), counter: $("counter"), note: $("under-note"),
    profileSub: $("profile-sub"), toast: $("toast"),
    hintA: $("hint-a"), hintX: $("hint-x"), hintY: $("hint-y"),
  };

  // ---------- interface sounds (synthesized locally, never fetched) ----------
  const sound = (() => {
    let enabled = stored("gamestrike-sounds", "on") !== "off";
    let context = null;
    const tones = { move: [880, 0.03, 0.035], channel: [660, 0.06, 0.04], select: [740, 0.1, 0.06], back: [440, 0.08, 0.05], open: [600, 0.12, 0.05], error: [200, 0.18, 0.06] };
    function play(kind) {
      if (!enabled || !window.AudioContext) return;
      try {
        context = context || new AudioContext();
        const [frequency, length, volume] = tones[kind] || tones.move;
        const now = context.currentTime;
        const osc = context.createOscillator();
        const gain = context.createGain();
        osc.type = "sine";
        osc.frequency.setValueAtTime(frequency, now);
        if (kind === "select" || kind === "open") osc.frequency.exponentialRampToValueAtTime(frequency * 1.6, now + length);
        if (kind === "back") osc.frequency.exponentialRampToValueAtTime(frequency * 0.7, now + length);
        gain.gain.setValueAtTime(volume, now);
        gain.gain.exponentialRampToValueAtTime(0.0001, now + length);
        osc.connect(gain).connect(context.destination);
        osc.start(now);
        osc.stop(now + length + 0.02);
      } catch (_) { /* sound is decoration only */ }
    }
    return {
      play,
      get enabled() { return enabled; },
      toggle() { enabled = !enabled; store("gamestrike-sounds", enabled ? "on" : "off"); if (enabled) play("select"); },
    };
  })();

  // ---------- startup sequence ----------
  // Plays the active style's boot animation once. Any key, click, or controller button skips it.
  let bootTimer = null;
  function finishBoot() {
    if (state.booted) return;
    state.booted = true;
    clearTimeout(bootTimer);
    el.boot.classList.add("done");
    if (state.loaded) render({ focus: !state.dialog });
  }
  function startBoot() {
    const reduce = window.matchMedia?.("(prefers-reduced-motion: reduce)")?.matches;
    bootTimer = setTimeout(finishBoot, reduce ? 0 : BOOT_MS[state.style]);
    for (const type of ["keydown", "mousedown"]) {
      window.addEventListener(type, (event) => {
        if (state.booted) return;
        event.preventDefault();
        event.stopImmediatePropagation();
        finishBoot();
      }, { capture: true });
    }
  }

  // ---------- helpers ----------
  const gameById = (id) => state.games.find((game) => game.id === id) || null;
  const sourceLabel = (game) => (game.source_kind === "zip" ? "ZIP archive" : "HTML file");
  const label = (list, key) => list.find(([k]) => k === key)?.[1] || "";

  function toast(message, isError = false) {
    el.toast.textContent = message;
    el.toast.classList.toggle("error", isError);
    el.toast.hidden = false;
    if (isError) sound.play("error");
    clearTimeout(toast.timer);
    toast.timer = setTimeout(() => { el.toast.hidden = true; }, isError ? 9000 : 5000);
  }

  const ICONS = {
    add: '<svg viewBox="0 0 24 24"><path d="M12 5v14M5 12h14"/></svg>',
    search: '<svg viewBox="0 0 24 24"><circle cx="10.5" cy="10.5" r="6"/><path d="m15 15 5 5"/></svg>',
    sort: '<svg viewBox="0 0 24 24"><path d="M7 4v16M3 16l4 4 4-4M17 20V4M13 8l4-4 4 4"/></svg>',
    clear: '<svg viewBox="0 0 24 24"><path d="M6 6l12 12M18 6 6 18"/></svg>',
    sound: '<svg viewBox="0 0 24 24"><path d="M4 9h4l5-4v14l-5-4H4z"/><path d="M16.5 8.5a5 5 0 0 1 0 7M19 6a8.5 8.5 0 0 1 0 12"/></svg>',
    mute: '<svg viewBox="0 0 24 24"><path d="M4 9h4l5-4v14l-5-4H4z"/><path d="m16 9 6 6m0-6-6 6"/></svg>',
    theme: '<svg viewBox="0 0 24 24"><circle cx="12" cy="12" r="8"/><path d="M12 4a8 8 0 0 0 0 16z" fill="currentColor"/></svg>',
    about: '<svg viewBox="0 0 24 24"><circle cx="12" cy="12" r="8.5"/><path d="M12 11v6M12 7.5v.1"/></svg>',
    style: '<svg viewBox="0 0 24 24"><rect x="3" y="6" width="9" height="12" rx="1.5"/><rect x="13.5" y="8" width="6" height="8" rx="1"/><path d="M21 9.5v5"/></svg>',
    warn: '<svg viewBox="0 0 24 24"><path d="M12 4 2.5 20h19z"/><path d="M12 10v4M12 17v.1"/></svg>',
  };

  // ---------- channels ----------
  function buildChannels() {
    const channels = [];
    const games = core.filterAndSort(state.games, state.query, state.sort);
    const items = games.map((game) => ({ kind: "game", game }));
    if (state.query) items.unshift({ kind: "clear-search" });
    if (!state.games.length) items.push({ kind: "add-first" });
    else if (!games.length) items.push({ kind: "no-match" });
    const recent = core.filterAndSort(state.games.filter((game) => game.last_played), "", "recent").slice(0, 10);
    if (recent.length) channels.push({ key: "recent", title: "Jump Back In", items: recent.map((game) => ({ kind: "game", game })) });
    channels.push({ key: "find", title: "Add & Find", items: [{ kind: "add" }, { kind: "search" }, { kind: "sort" }] });
    channels.push({ key: "games", title: state.query ? `My Games · “${state.query}”` : "My Games", items });
    const system = [{ kind: "style" }, ...(state.style === "blades" ? [{ kind: "theme" }] : []), { kind: "sound" }, { kind: "about" }];
    if (state.problems.length) system.unshift({ kind: "problems" });
    channels.push({ key: "system", title: "System", items: system });
    return channels;
  }

  function gamesChannel() { return Math.max(0, state.channels.findIndex((channel) => channel.key === "games")); }

  function itemKey(item) { return item.kind === "game" ? `game:${item.game.id}` : item.kind; }

  function current() {
    const channel = state.channels[state.channel];
    const index = Math.min(state.focus[channel.key] || 0, channel.items.length - 1);
    return { channel, index, item: channel.items[index] };
  }

  function render({ direction = 0, focus = true } = {}) {
    const remember = state.channels.length ? itemKey(current().item) : null;
    const key = state.channels[state.channel]?.key || "games";
    state.channels = buildChannels();
    const found = state.channels.findIndex((channel) => channel.key === key);
    state.channel = found >= 0 ? found : gamesChannel();
    const { channel } = current();
    if (remember) {
      const again = channel.items.findIndex((item) => itemKey(item) === remember);
      if (again >= 0) state.focus[channel.key] = again;
    }
    renderChannels();
    renderTiles(direction, focus);
  }

  function renderChannels() {
    el.channels.replaceChildren(...state.channels.map((channel, index) => {
      const node = document.createElement("button");
      node.type = "button";
      node.className = "channel";
      node.setAttribute("role", "tab");
      node.setAttribute("aria-selected", String(index === state.channel));
      node.tabIndex = -1;
      node.textContent = channel.title;
      const d = index - state.channel;
      const layout = d === 0 ? [0, 1, 1] : d === -1 ? [-1.3, 0.6, 0.85] : d === -2 ? [-2.1, 0.5, 0.55] : d === -3 ? [-2.75, 0.42, 0.3] : d < 0 ? [-3.2, 0.36, 0] : [0.9, 0.6, 0];
      node.classList.toggle("current", d === 0);
      node.style.transform = `translateY(${layout[0]}em) scale(${layout[1]})`;
      node.style.opacity = layout[2];
      node.style.pointerEvents = layout[2] > 0.2 ? "auto" : "none";
      node.addEventListener("click", () => switchChannel(index));
      return node;
    }));
  }

  function tile(item, index) {
    const node = document.createElement("button");
    node.type = "button";
    node.className = "tile";
    node.dataset.index = index;
    node.setAttribute("role", "option");
    node.tabIndex = -1;
    const art = document.createElement("div");
    art.className = "tile-art";
    const labelBox = document.createElement("div");
    labelBox.className = "tile-label";
    const title = document.createElement("div");
    title.className = "tile-title";
    const meta = document.createElement("div");
    meta.className = "tile-meta";
    const badges = document.createElement("div");
    badges.className = "tile-badges";
    let text = "";
    let sub = "";
    let aria = "";
    if (item.kind === "game") {
      const game = item.game;
      const cover = core.cover(game.id, game.title);
      node.classList.add("game");
      node.style.setProperty("--h1", cover.hue);
      node.style.setProperty("--h2", cover.hue2);
      const initials = document.createElement("span");
      initials.className = "tile-initials";
      initials.textContent = cover.initials;
      art.append(initials);
      text = game.title;
      sub = !game.files_ok ? "Files missing" : game.last_played ? `${sourceLabel(game)} · Played ${core.formatWhen(game.last_played).toLocaleLowerCase()}` : `${sourceLabel(game)} · Not played yet`;
      if (game.id === state.running) badges.append(badge("playing", "Playing"));
      if (!game.files_ok) badges.append(badge("missing", "Files missing"));
      aria = `${game.title}. ${sub}.${game.id === state.running ? " Playing now." : ""}`;
    } else {
      const spec = {
        "add-first": ["add", "Add your first game", "Choose an .html file or a .zip"],
        add: ["add", "Add a game", "From an .html file or a .zip"],
        search: ["search", "Find a game", state.query ? `Showing “${state.query}”` : "Search by title"],
        "clear-search": ["clear", "Show all games", `Clear the search for “${state.query}”`],
        "no-match": ["search", "No games match", "Press A to search again"],
        sort: ["sort", `Sort: ${label(SORTS, state.sort)}`, "Press A to change the order"],
        sound: [sound.enabled ? "sound" : "mute", `Sounds: ${sound.enabled ? "On" : "Off"}`, "Interface sounds"],
        theme: ["theme", `Color: ${label(THEMES, state.theme)}`, "Press A for the next color"],
        style: ["style", `Style: ${label(STYLES, state.style)}`, "Blades, Original, or PWDS Orange"],
        about: ["about", "About GameStrike", "Version, publisher, and safety notes"],
        problems: ["warn", `${state.problems.length} unreadable ${state.problems.length === 1 ? "folder" : "folders"}`, "See what could not be read"],
      }[item.kind];
      art.innerHTML = ICONS[spec[0]];
      text = spec[1];
      sub = spec[2];
      aria = `${text}. ${sub}.`;
    }
    title.textContent = text;
    meta.textContent = sub;
    labelBox.append(title, meta);
    node.append(art, badges, labelBox);
    node.setAttribute("aria-label", aria);
    node.addEventListener("click", () => {
      if (Number(node.dataset.index) === current().index) activate();
      else focusIndex(Number(node.dataset.index));
    });
    return node;
  }

  function badge(kind, text) {
    const node = document.createElement("span");
    node.className = `badge ${kind}`;
    node.textContent = text;
    return node;
  }

  function renderTiles(direction, focus) {
    const { channel } = current();
    el.stage.setAttribute("aria-label", channel.title);
    el.stage.replaceChildren(...channel.items.map(tile));
    if (direction) {
      el.stage.classList.remove("enter-up", "enter-down");
      void el.stage.offsetWidth;
      el.stage.classList.add(direction < 0 ? "enter-up" : "enter-down");
    }
    layoutTiles(focus);
  }

  function layoutTiles(focus = true) {
    const { channel, index, item } = current();
    for (const node of el.stage.children) {
      const offset = Number(node.dataset.index) - index;
      const layout = core.tileLayout(offset);
      node.style.setProperty("--x", layout.x);
      node.style.setProperty("--y", layout.y);
      node.style.setProperty("--sx", layout.sx);
      node.style.setProperty("--sy", layout.sy);
      node.style.opacity = layout.opacity;
      node.style.zIndex = layout.z;
      node.style.visibility = layout.visible || offset < 0 ? "visible" : "hidden";
      node.style.pointerEvents = layout.visible ? "auto" : "none";
      node.classList.toggle("focused", offset === 0);
      node.classList.toggle("behind", offset > 0);
      node.setAttribute("aria-selected", String(offset === 0));
      node.tabIndex = offset === 0 ? 0 : -1;
      if (offset === 0 && focus && !state.dialog && state.style !== "original") node.focus({ preventScroll: true });
    }
    el.counter.textContent = `${index + 1} of ${channel.items.length}`;
    describe(item);
    renderOriginal(focus);
  }

  function itemText(item) {
    if (item.kind === "game") return [item.game.title, item.game.id === state.running ? "Playing" : !item.game.files_ok ? "Files missing" : item.game.last_played ? core.formatWhen(item.game.last_played) : "New"];
    const tileNode = el.stage.querySelector(`.tile[data-index="${current().channel.items.indexOf(item)}"]`);
    return [tileNode?.querySelector(".tile-title")?.textContent || "", ""];
  }

  function renderOriginal(focus) {
    if (state.style !== "original") return;
    const { channel, index } = current();
    const header = document.createElement("small");
    header.textContent = `${state.channel + 1} / ${state.channels.length} · ◀ ▶ changes menu`;
    el.ogHeader.replaceChildren(document.createTextNode(channel.title), header);
    const windowSize = 6;
    const first = Math.max(0, Math.min(index - 2, channel.items.length - windowSize));
    const rows = [];
    if (first > 0) rows.push(more(`▲ ${first} more`));
    channel.items.slice(first, first + windowSize).forEach((item, offset) => {
      const position = first + offset;
      const node = document.createElement("button");
      node.type = "button";
      node.className = "og-item";
      node.setAttribute("role", "option");
      node.tabIndex = position === index ? 0 : -1;
      node.classList.toggle("focused", position === index);
      node.setAttribute("aria-selected", String(position === index));
      const [name, note] = itemText(item);
      const nameNode = document.createElement("span");
      nameNode.textContent = name;
      const noteNode = document.createElement("small");
      noteNode.textContent = note;
      node.append(nameNode, noteNode);
      node.addEventListener("click", () => { if (position === current().index) activate(); else focusIndex(position); });
      rows.push(node);
    });
    const rest = channel.items.length - (first + windowSize);
    if (rest > 0) rows.push(more(`▼ ${rest} more`));
    el.ogList.replaceChildren(...rows);
    el.ogDetail.textContent = el.note.textContent || `${index + 1} of ${channel.items.length}`;
    if (focus && !state.dialog) el.ogList.querySelector(".og-item.focused")?.focus({ preventScroll: true });
  }

  function more(text) {
    const node = document.createElement("div");
    node.className = "og-more";
    node.textContent = text;
    return node;
  }

  function describe(item) {
    const isGame = item?.kind === "game";
    const game = isGame ? item.game : null;
    el.hintX.hidden = !isGame;
    el.hintY.hidden = !isGame;
    el.hintA.querySelector(".hint-label").textContent = isGame ? "Play offline" : "Select";
    el.note.classList.remove("warn");
    if (!game) { el.note.textContent = ""; return; }
    if (!game.files_ok) { el.note.classList.add("warn"); el.note.textContent = "This game's files are missing. Remove it, then add the original file again."; return; }
    if (game.id === state.running) { el.note.textContent = "Playing now in its own window. Close it to come back here."; return; }
    if (state.running) { el.note.textContent = "Another game is open. Close it to play this one."; return; }
    const plays = game.play_count ? `${game.play_count} ${game.play_count === 1 ? "play" : "plays"}` : "Never played";
    const data = game.save_bytes ? `game data ${core.formatBytes(game.save_bytes)}` : "no game data yet";
    el.note.textContent = `${plays} · ${data} · added ${game.added_at ? core.formatWhen(game.added_at).toLocaleLowerCase() : "earlier"}`;
  }

  function focusIndex(index, quiet = false) {
    const { channel } = current();
    const next = Math.max(0, Math.min(channel.items.length - 1, index));
    if (next === (state.focus[channel.key] || 0)) return;
    state.focus[channel.key] = next;
    if (!quiet) sound.play("move");
    layoutTiles();
  }

  function moveTile(step) { focusIndex((current().index) + step); }

  function switchChannel(target) {
    if (target < 0 || target >= state.channels.length || target === state.channel) return;
    const direction = target > state.channel ? 1 : -1;
    state.channel = target;
    sound.play("channel");
    renderChannels();
    renderTiles(direction, true);
  }

  function updateProfile() {
    const count = state.games.length;
    const time = new Date().toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });
    const playing = state.running ? `Playing ${gameById(state.running)?.title || "a game"}` : `${count} ${count === 1 ? "game" : "games"}`;
    el.profileSub.textContent = `${playing} · ${time}`;
  }

  // ---------- actions ----------
  function activate() {
    const { item } = current();
    if (!item) return;
    switch (item.kind) {
      case "game": return launch(item.game, false);
      case "add":
      case "add-first": return importGame();
      case "search":
      case "no-match": return openDialog("search");
      case "clear-search": state.query = ""; sound.play("back"); return render();
      case "sort": {
        const next = SORTS[(SORTS.findIndex(([key]) => key === state.sort) + 1) % SORTS.length][0];
        state.sort = next;
        store("gamestrike-sort", next);
        sound.play("select");
        return render();
      }
      case "sound": sound.toggle(); return render();
      case "theme": {
        const next = THEMES[(THEMES.findIndex(([key]) => key === state.theme) + 1) % THEMES.length][0];
        state.theme = next;
        document.body.dataset.theme = next;
        store("gamestrike-theme", next);
        sound.play("select");
        return render();
      }
      case "style": {
        const next = STYLES[(STYLES.findIndex(([key]) => key === state.style) + 1) % STYLES.length][0];
        setStyle(next);
        sound.play("select");
        return render();
      }
      case "about": return openDialog("about");
      case "problems": return openDialog("problems");
    }
  }

  function setStyle(next, persist = true) {
    state.style = next;
    document.body.dataset.style = next;
    if (persist) store("gamestrike-style", next);
  }

  function focusedGame() {
    const { item } = current();
    return item?.kind === "game" ? item.game : null;
  }

  function importGame() {
    if (state.importing) return;
    state.importing = true;
    sound.play("open");
    send({ type: "pick_and_import" });
  }

  function launch(game, network) {
    if (!game) return;
    if (state.running) { toast(game.id === state.running ? "This game is already open in its own window." : "Another game is open. Close it first.", true); return; }
    if (state.launching) return;
    if (!game.files_ok) { toast("This game's files are missing. Remove it, then add the original file again.", true); return; }
    state.launching = true;
    sound.play("select");
    send({ type: "launch_game", id: game.id, network });
  }

  // ---------- dialogs ----------
  const dialogs = {
    search: { node: $("search-dialog"), first: "search-input" },
    network: { node: $("network-dialog"), first: "network-cancel" },
    manage: { node: $("manage-dialog"), first: "manage-rename" },
    rename: { node: $("rename-dialog"), first: "rename-input" },
    reset: { node: $("reset-dialog"), first: "reset-cancel" },
    remove: { node: $("remove-dialog"), first: "remove-cancel" },
    problems: { node: $("problems-dialog"), first: "problems-close" },
    about: { node: $("about-dialog"), first: "close-about" },
  };
  let dialogGame = null;

  function openDialog(name) {
    const game = ["network", "manage", "rename", "reset", "remove"].includes(name) ? (state.dialog ? dialogGame : focusedGame()) : null;
    if (["network", "manage", "rename", "reset", "remove"].includes(name) && !game) return;
    if (name === "network" && (state.running || state.launching || !game.files_ok)) { if (state.running) toast("Another game is open. Close it first.", true); return; }
    if ((name === "reset" || name === "remove") && game.id === state.running) return;
    if (name === "reset" && !game.save_bytes) return;
    if (state.dialog) closeDialog({ restore: false, silent: true });
    else state.returnFocus = document.activeElement;
    dialogGame = game;
    if (name === "search") { $("search-input").value = state.query; updateSearchCount(); }
    if (name === "network") $("network-title").textContent = `Play ${game.title} with network?`;
    if (name === "manage") {
      $("manage-title").textContent = game.title;
      $("manage-facts").textContent = `${sourceLabel(game)} · ${game.play_count ? `${game.play_count} ${game.play_count === 1 ? "play" : "plays"}` : "never played"} · ${game.save_bytes ? `game data ${core.formatBytes(game.save_bytes)}` : "no game data"} · controller and network needs not checked`;
      $("manage-reset").disabled = !game.save_bytes || game.id === state.running;
      $("manage-remove").disabled = game.id === state.running;
    }
    if (name === "rename") { $("rename-input").value = game.title; $("rename-error").hidden = true; }
    if (name === "reset") $("reset-title").textContent = `Clear saves for ${game.title}?`;
    if (name === "remove") { $("remove-title").textContent = `Remove ${game.title}?`; $("remove-keep").checked = true; }
    if (name === "problems") {
      $("problems-list").replaceChildren(...state.problems.map((problem) => {
        const li = document.createElement("li");
        li.textContent = problem;
        return li;
      }));
    }
    state.dialog = name;
    dialogs[name].node.hidden = false;
    sound.play("open");
    const first = $(dialogs[name].first);
    first.focus();
    if (name === "rename" || name === "search") first.select();
  }

  function closeDialog({ restore = true, silent = false } = {}) {
    if (!state.dialog) return;
    dialogs[state.dialog].node.hidden = true;
    state.dialog = null;
    if (!silent) sound.play("back");
    if (restore) layoutTiles(true);
  }

  function dialogFocusables() {
    if (!state.dialog) return [];
    return [...dialogs[state.dialog].node.querySelectorAll("button, input")].filter((node) => !node.disabled && node.offsetParent !== null);
  }

  function moveDialogFocus(step) {
    const items = dialogFocusables();
    if (!items.length) return;
    const index = items.indexOf(document.activeElement);
    items[(index + step + items.length) % items.length].focus();
    sound.play("move");
  }

  function updateSearchCount() {
    const query = $("search-input").value.trim();
    const count = core.filterAndSort(state.games, query, state.sort).length;
    $("search-count").textContent = query ? `${count} ${count === 1 ? "game matches" : "games match"}` : `${state.games.length} games in your library`;
  }

  $("search-input").addEventListener("input", updateSearchCount);
  $("search-form").addEventListener("submit", (event) => {
    event.preventDefault();
    state.query = $("search-input").value.trim().slice(0, 80);
    closeDialog({ restore: false, silent: true });
    sound.play("select");
    state.focus.games = state.query ? 1 : 0;
    render();
    switchChannel(gamesChannel());
  });
  $("search-clear").addEventListener("click", () => {
    state.query = "";
    closeDialog({ restore: false, silent: true });
    sound.play("back");
    render();
  });
  $("network-cancel").addEventListener("click", () => closeDialog());
  $("network-confirm").addEventListener("click", () => { const game = dialogGame; closeDialog({ silent: true }); launch(game, true); });
  $("manage-rename").addEventListener("click", () => openDialog("rename"));
  $("manage-reset").addEventListener("click", () => openDialog("reset"));
  $("manage-remove").addEventListener("click", () => openDialog("remove"));
  $("manage-close").addEventListener("click", () => closeDialog());
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
  $("reset-cancel").addEventListener("click", () => closeDialog());
  $("reset-confirm").addEventListener("click", () => { const game = dialogGame; closeDialog({ silent: true }); if (game) send({ type: "reset_saves", id: game.id }); });
  $("remove-cancel").addEventListener("click", () => closeDialog());
  $("remove-confirm").addEventListener("click", () => {
    const game = dialogGame;
    const keep = $("remove-keep").checked;
    closeDialog({ silent: true });
    if (game) send({ type: "remove_game", id: game.id, keep_saves: keep });
  });
  $("problems-close").addEventListener("click", () => closeDialog());
  $("close-about").addEventListener("click", () => closeDialog());
  for (const { node } of Object.values(dialogs)) {
    node.addEventListener("mousedown", (event) => { if (event.target === node) closeDialog(); });
  }

  // ---------- keyboard and mouse ----------
  document.addEventListener("keydown", (event) => {
    if (state.dialog) {
      if (event.key === "Escape") { event.preventDefault(); closeDialog(); return; }
      if (event.key === "Tab") { event.preventDefault(); moveDialogFocus(event.shiftKey ? -1 : 1); return; }
      const typing = document.activeElement?.tagName === "INPUT" && document.activeElement.type !== "checkbox";
      if (!typing && (event.key === "ArrowLeft" || event.key === "ArrowUp")) { event.preventDefault(); moveDialogFocus(-1); }
      if (!typing && (event.key === "ArrowRight" || event.key === "ArrowDown")) { event.preventDefault(); moveDialogFocus(1); }
      return;
    }
    if (event.ctrlKey && event.key.toLowerCase() === "f") { event.preventDefault(); openDialog("search"); return; }
    if (event.ctrlKey || event.altKey || event.metaKey) return;
    switch (event.key) {
      case "ArrowLeft": event.preventDefault(); navigate("left"); break;
      case "ArrowRight": event.preventDefault(); navigate("right"); break;
      case "ArrowUp": event.preventDefault(); navigate("up"); break;
      case "ArrowDown": event.preventDefault(); navigate("down"); break;
      case "Home": event.preventDefault(); focusIndex(0); break;
      case "End": event.preventDefault(); focusIndex(current().channel.items.length - 1); break;
      case "Enter": case " ": event.preventDefault(); activate(); break;
      case "n": case "N": event.preventDefault(); openDialog("network"); break;
      case "F2": event.preventDefault(); openDialog("rename"); break;
      case "Delete": event.preventDefault(); openDialog("remove"); break;
      case "/": event.preventDefault(); openDialog("search"); break;
      case "Escape": case "Backspace": event.preventDefault(); back(); break;
    }
  });

  function navigate(direction) {
    const vertical = direction === "up" || direction === "down";
    const step = direction === "left" || direction === "up" ? -1 : 1;
    // Blades: left/right moves along a channel, up/down changes channel. Original: the reverse.
    if (vertical === (state.style === "original")) moveTile(step);
    else switchChannel(state.channel + step);
  }

  function back() {
    if (state.channel !== gamesChannel()) { switchChannel(gamesChannel()); return; }
    if (state.query) { state.query = ""; sound.play("back"); render(); }
  }

  let wheelAt = 0;
  el.stage.addEventListener("wheel", (event) => {
    event.preventDefault();
    const now = performance.now();
    if (now - wheelAt < 120) return;
    wheelAt = now;
    const delta = Math.abs(event.deltaX) > Math.abs(event.deltaY) ? event.deltaX : event.deltaY;
    moveTile(delta > 0 ? 1 : -1);
  }, { passive: false });
  el.stage.addEventListener("dblclick", () => { if (current().item?.kind === "game") activate(); });

  // ---------- controller (launcher navigation only; games handle their own input) ----------
  const readPad = core.createPadReader();
  function pollPad(now) {
    const pad = Array.from(navigator.getGamepads?.() || []).find(Boolean) || null;
    const actions = readPad(pad, now);
    // Ignore input while a game window is open or this window is in the background.
    if (actions.length && !state.booted) { finishBoot(); requestAnimationFrame(pollPad); return; }
    if (actions.length && !state.running && document.hasFocus()) {
      for (const action of actions) handlePad(action);
    }
    requestAnimationFrame(pollPad);
  }

  function handlePad(action) {
    if (state.dialog) {
      if (action === "b") closeDialog();
      else if (action === "a") document.activeElement?.click();
      else if (action === "left" || action === "up") moveDialogFocus(-1);
      else if (action === "right" || action === "down") moveDialogFocus(1);
      return;
    }
    if (["left", "right", "up", "down"].includes(action)) navigate(action);
    else if (action === "a") activate();
    else if (action === "x") openDialog("network");
    else if (action === "y") openDialog("manage");
    else if (action === "b") back();
  }

  // ---------- messages from the trusted Rust launcher ----------
  function receive(message) {
    switch (message.type) {
      case "library": {
        state.games = Array.isArray(message.games) ? message.games : [];
        state.problems = Array.isArray(message.problems) ? message.problems : [];
        state.running = message.running || null;
        const first = !state.loaded;
        state.loaded = true;
        render({ focus: !state.dialog && state.booted });
        updateProfile();
        if (first && state.booted) render({ focus: true });
        break;
      }
      case "importing":
        toast(`Adding ${message.name}…`);
        break;
      case "pick_cancelled":
        state.importing = false;
        break;
      case "imported": {
        state.importing = false;
        state.query = "";
        state.channel = gamesChannel();
        state.pendingFocus = message.game.id;
        sound.play("select");
        toast(message.already_present
          ? `${message.game.title} is already in your library. Its saves were kept.`
          : message.restored_saves
            ? `${message.game.title} is back, with the saves you kept.`
            : `${message.game.title} was added to your library.`);
        break;
      }
      case "player_started":
        state.launching = false;
        state.running = message.id;
        toast(message.network ? "Game opened with network access." : "Game opened offline in its own window.");
        render();
        updateProfile();
        break;
      case "player_exited":
        if (state.running === message.id) state.running = null;
        state.launching = false;
        toast(message.message, !message.ok);
        render({ focus: !state.dialog });
        updateProfile();
        break;
      case "managed":
        toast(message.message);
        break;
      case "removed":
        toast(message.message);
        break;
      case "error":
        if (message.operation === "import") state.importing = false;
        if (message.operation === "launch") state.launching = false;
        toast(message.message, true);
        break;
    }
    if (message.type === "library" && state.pendingFocus) {
      const index = state.channels[gamesChannel()].items.findIndex((item) => item.kind === "game" && item.game.id === state.pendingFocus);
      state.pendingFocus = null;
      if (index >= 0) { state.channel = gamesChannel(); state.focus.games = index; renderChannels(); renderTiles(0, true); }
    }
  }

  // Used only by the launcher's explicit --capture-view option to stage a screenshot.
  function capture(view) {
    // Captures always start from the same look and never change saved preferences.
    const boot = /^boot-(blades|original|pwds)$/.exec(view);
    state.theme = "lime";
    document.body.dataset.theme = "lime";
    setStyle(boot ? boot[1] : view === "original" || view === "pwds" ? view : "blades", false);
    render({ focus: false });
    if (boot) {
      el.boot.style.setProperty("--seek", { blades: "2.75s", original: "3.45s", pwds: "1.95s" }[boot[1]]);
      el.boot.classList.add("frozen");
      clearTimeout(bootTimer);
      return;
    }
    finishBoot();
    if (view === "original" || view === "pwds") return;
    if (view === "long") {
      const items = state.channels[gamesChannel()].items;
      let best = -1;
      items.forEach((item, index) => { if (item.kind === "game" && (best < 0 || item.game.title.length > items[best].game.title.length)) best = index; });
      if (best >= 0) { state.focus.games = best; layoutTiles(); }
    } else if (view === "launch") {
      openDialog("network");
    } else if (view === "manage") {
      openDialog("manage");
    } else if (view === "about") {
      openDialog("about");
    } else if (view === "system") {
      switchChannel(state.channels.length - 1);
    } else if (view === "find") {
      switchChannel(state.channels.findIndex((channel) => channel.key === "find"));
    }
  }

  window.GameStrike = Object.freeze({ receive, capture });
  startBoot();
  setInterval(updateProfile, 15000);
  requestAnimationFrame(pollPad);
  send({ type: "ready" });
})();
