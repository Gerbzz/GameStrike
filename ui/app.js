(() => {
  "use strict";

  const core = window.GameStrikeCore;
  const $ = (id) => document.getElementById(id);
  const send = (message) => window.ipc.postMessage(JSON.stringify(message));
  const SORTS = [["recent", "Recent"], ["title-asc", "Title A–Z"], ["title-desc", "Title Z–A"], ["added", "Newest"]];
  const BOOT_MS = 3200;
  const reduceMotion = () => Boolean(window.matchMedia?.("(prefers-reduced-motion: reduce)")?.matches);

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
    collapsed: stored("gamestrike-sidebar", "open") === "collapsed",
    zone: "grid", // "grid" or "sidebar"
    gridIndex: 0,
    sideIndex: 0,
    detailId: null,
    visible: [],
    filters: [],
    importing: false,
    launching: false,
    dialog: null,
    capturing: false,
  };
  if (!SORTS.some(([key]) => key === state.sort)) state.sort = "recent";
  document.body.classList.toggle("collapsed", state.collapsed);

  const el = {
    boot: $("boot"), grid: $("grid"), filters: $("filters"), search: $("search"), sort: $("sort"),
    title: $("view-title"), count: $("view-count"), empty: $("empty"), status: $("status"), toast: $("toast"),
    detail: $("detail"), collapse: $("collapse"), hintA: $("hint-a"),
  };

  // ---------- interface sounds (synthesized locally, never fetched) ----------
  const sound = (() => {
    let enabled = stored("gamestrike-sounds", "on") !== "off";
    let context = null;
    const tones = { move: [880, 0.03, 0.03], zone: [660, 0.06, 0.04], select: [740, 0.1, 0.06], back: [440, 0.08, 0.05], open: [600, 0.12, 0.05], error: [200, 0.18, 0.06] };
    function play(kind) {
      if (!enabled || !window.AudioContext || state.capturing) return;
      try {
        context = context || new AudioContext();
        const [frequency, length, volume] = tones[kind] || tones.move;
        const now = context.currentTime;
        const osc = context.createOscillator();
        const gain = context.createGain();
        osc.type = "triangle";
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

  // ---------- startup ----------
  let bootTimer = null;
  function finishBoot() {
    if (state.booted) return;
    state.booted = true;
    clearTimeout(bootTimer);
    el.boot.classList.add("done");
    if (state.loaded) render({ focus: !state.dialog });
  }
  function startBoot() {
    bootTimer = setTimeout(finishBoot, reduceMotion() ? 0 : BOOT_MS);
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
  const plural = (count, one, many) => `${count} ${count === 1 ? one : many}`;

  function toast(message, isError = false) {
    el.toast.textContent = message;
    el.toast.classList.toggle("error", isError);
    el.toast.hidden = false;
    if (isError) sound.play("error");
    clearTimeout(toast.timer);
    toast.timer = setTimeout(() => { el.toast.hidden = true; }, isError ? 9000 : 5000);
  }

  const ICONS = {
    all: '<svg viewBox="0 0 24 24"><rect x="3.5" y="3.5" width="7" height="7" rx="1.5"/><rect x="13.5" y="3.5" width="7" height="7" rx="1.5"/><rect x="3.5" y="13.5" width="7" height="7" rx="1.5"/><rect x="13.5" y="13.5" width="7" height="7" rx="1.5"/></svg>',
    recent: '<svg viewBox="0 0 24 24"><circle cx="12" cy="12" r="8.5"/><path d="M12 7v5l3.5 2"/></svg>',
    star: '<svg viewBox="0 0 24 24"><path d="m12 3.5 2.6 5.4 5.9.8-4.3 4.1 1 5.8L12 16.8l-5.2 2.8 1-5.8-4.3-4.1 5.9-.8z"/></svg>',
    file: '<svg viewBox="0 0 24 24"><path d="M6 3.5h8l4 4V20.5H6z"/><path d="M14 3.5v4h4M9 13l-2 2 2 2M15 13l2 2-2 2"/></svg>',
    zip: '<svg viewBox="0 0 24 24"><path d="M6 3.5h12v17H6z"/><path d="M12 3.5v2M12 7.5v2M12 11.5v2M10.5 15.5h3v3h-3z"/></svg>',
    tag: '<svg viewBox="0 0 24 24"><path d="M3.5 12.5V4.5a1 1 0 0 1 1-1h8l8 8-9 9z"/><circle cx="8" cy="8" r="1.5"/></svg>',
  };

  // ---------- sidebar ----------
  function sidebarItems() {
    return [...el.filters.querySelectorAll(".side-item"), $("add-game"), $("settings"), $("power")];
  }

  function renderFilters() {
    state.filters = core.buildFilters(state.games);
    if (!state.filters.some((filter) => filter.key === state.filter)) state.filter = "all";
    const nodes = [];
    let group = null;
    for (const filter of state.filters) {
      if (filter.group && filter.group !== group) {
        group = filter.group;
        const heading = document.createElement("div");
        heading.className = "filter-group";
        heading.textContent = group;
        nodes.push(heading);
      }
      const button = document.createElement("button");
      button.type = "button";
      button.className = "side-item";
      button.dataset.filter = filter.key;
      button.tabIndex = -1;
      button.setAttribute("aria-current", String(filter.key === state.filter));
      button.setAttribute("aria-label", `${filter.label}, ${plural(filter.count, "game", "games")}`);
      button.innerHTML = ICONS[filter.icon] || ICONS.tag;
      const name = document.createElement("span");
      name.textContent = filter.label;
      const count = document.createElement("span");
      count.className = "count";
      count.textContent = String(filter.count);
      button.append(name, count);
      button.addEventListener("click", () => { setFilter(filter.key); state.zone = "grid"; layoutFocus(true); });
      nodes.push(button);
    }
    el.filters.replaceChildren(...nodes);
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
    el.title.textContent = state.query ? `“${state.query}”` : filter?.label || "All games";
    el.count.textContent = `${plural(state.visible.length, "game", "games")}${state.query ? ` in ${filter?.label || "All games"}` : ""}`;
    el.sort.textContent = `Sort: ${label(SORTS, state.sort)}`;
    el.sort.disabled = state.filter === "recent";
    el.grid.replaceChildren(...state.visible.map((game, index) => tile(game, index)));
    const empty = !state.visible.length;
    el.empty.hidden = !empty || !state.loaded;
    el.grid.hidden = empty;
    if (empty) {
      const none = !state.games.length;
      $("empty-title").textContent = none ? "Your library is empty" : state.query ? "No games match" : "Nothing here yet";
      $("empty-copy").textContent = none
        ? "Add a game from an .html file or a .zip. GameStrike copies it into its own library; your download is never changed."
        : state.query ? `No game title contains “${state.query}”.` : state.filter === "favorites" ? "Mark a game as a favorite in its details (F)." : "Play a game and it will show up here.";
      $("empty-action").textContent = none ? "Add a game" : state.query ? "Clear search" : "Show all games";
      $("empty-action").dataset.action = none ? "add" : state.query ? "clear" : "all";
    }
    state.gridIndex = Math.min(Math.max(state.gridIndex, 0), Math.max(0, state.visible.length - 1));
  }

  // A cover: the stored image when there is one, otherwise one drawn from the title.
  function coverNode(game) {
    const url = core.coverUrl(game);
    if (url) {
      const image = document.createElement("img");
      image.className = "cover-img";
      image.alt = "";
      image.decoding = "async";
      image.src = url;
      image.addEventListener("error", () => image.replaceWith(generatedCover(game)), { once: true });
      return image;
    }
    return generatedCover(game);
  }

  function generatedCover(game) {
    const cover = core.cover(game.id, game.title);
    const node = document.createElement("div");
    node.className = "gen";
    node.dataset.pattern = String(cover.pattern);
    node.style.setProperty("--h1", cover.hue);
    node.style.setProperty("--h2", cover.hue2);
    node.style.setProperty("--angle", `${cover.angle}deg`);
    const initials = document.createElement("div");
    initials.className = "gen-initials";
    initials.textContent = cover.initials;
    const title = document.createElement("div");
    title.className = "gen-title";
    title.textContent = game.title;
    node.append(initials, title);
    return node;
  }

  function badge(kind, text) {
    const node = document.createElement("span");
    node.className = `badge ${kind}`;
    node.textContent = text;
    return node;
  }

  function tileMeta(game) {
    if (!game.files_ok) return "Files missing";
    const origin = core.cleanSource(game.source) || sourceLabel(game);
    return game.last_played ? `${origin} · ${core.formatWhen(game.last_played)}` : `${origin} · New`;
  }

  function tile(game, index) {
    const node = document.createElement("button");
    node.type = "button";
    node.className = "tile";
    node.dataset.index = String(index);
    node.dataset.id = game.id;
    node.setAttribute("role", "option");
    node.tabIndex = -1;
    const badges = document.createElement("div");
    badges.className = "tile-badges";
    if (game.favorite) badges.append(badge("fav", "★"));
    if (game.id === state.running) badges.append(badge("playing", "Playing"));
    if (!game.files_ok) badges.append(badge("missing", "Files missing"));
    const labelBox = document.createElement("div");
    labelBox.className = "tile-label";
    const title = document.createElement("div");
    title.className = "tile-title";
    title.textContent = game.title;
    const meta = document.createElement("div");
    meta.className = "tile-meta";
    meta.textContent = tileMeta(game);
    labelBox.append(title, meta);
    node.append(coverNode(game), badges, labelBox);
    node.setAttribute("aria-label", `${game.title}. ${tileMeta(game)}.${game.favorite ? " Favorite." : ""}${game.id === state.running ? " Playing now." : ""}`);
    node.addEventListener("click", () => {
      if (state.zone === "grid" && Number(node.dataset.index) === state.gridIndex) launch(game, false);
      else { state.zone = "grid"; state.gridIndex = Number(node.dataset.index); sound.play("move"); layoutFocus(true); }
    });
    node.addEventListener("dblclick", () => launch(game, false));
    node.addEventListener("contextmenu", (event) => { event.preventDefault(); state.gridIndex = Number(node.dataset.index); openDetail(); });
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

  // Moves the visible focus (and keyboard focus) to the current zone's item.
  function layoutFocus(focus = true) {
    const tiles = [...el.grid.children];
    tiles.forEach((node, index) => {
      const on = state.zone === "grid" && index === state.gridIndex;
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
    const game = focusedGame();
    el.hintA.textContent = state.zone === "grid" && game ? "Play offline" : "Select";
    describe(game);
    if (!focus || state.dialog || state.detailId || !state.booted) return;
    const target = state.zone === "grid" ? tiles[state.gridIndex] : items[state.sideIndex];
    if (target) {
      target.focus({ preventScroll: true });
      target.scrollIntoView({ block: "nearest", inline: "nearest", behavior: reduceMotion() || state.capturing ? "auto" : "smooth" });
    } else if (state.zone === "grid") {
      $("empty-action").focus({ preventScroll: true });
    }
  }

  function describe(game) {
    if (!game) { el.status.textContent = ""; return; }
    if (!game.files_ok) { el.status.textContent = "This game's files are missing. Remove it, then add the original file again."; return; }
    if (game.id === state.running) { el.status.textContent = "Playing now in its own window. Close it to come back here."; return; }
    if (state.running) { el.status.textContent = "Another game is open. Close it to play this one."; return; }
    const plays = game.play_count ? plural(game.play_count, "play", "plays") : "Never played";
    const data = game.save_bytes ? `game data ${core.formatBytes(game.save_bytes)}` : "no game data yet";
    el.status.textContent = `${game.title} · ${plays} · ${data}`;
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
  }

  // ---------- navigation ----------
  function navigate(direction) {
    if (state.detailId) return moveWithin($("detail-actions"), direction);
    if (state.zone === "sidebar") {
      const items = sidebarItems();
      if (direction === "right") { if (state.visible.length) { state.zone = "grid"; sound.play("zone"); } }
      else state.sideIndex = core.listMove(state.sideIndex, items.length, direction);
      if (direction === "up" || direction === "down") sound.play("move");
      return layoutFocus(true);
    }
    const columns = gridColumns();
    if (direction === "left" && state.gridIndex % columns === 0) {
      state.zone = "sidebar";
      state.sideIndex = Math.max(0, sidebarItems().findIndex((node) => node.dataset.filter === state.filter));
      sound.play("zone");
      return layoutFocus(true);
    }
    const next = core.gridMove(state.gridIndex, state.visible.length, columns, direction);
    if (next >= 0 && next !== state.gridIndex) { state.gridIndex = next; sound.play("move"); }
    layoutFocus(true);
  }

  function moveWithin(container, direction) {
    const items = [...container.querySelectorAll("button, input")].filter((node) => !node.disabled && node.offsetParent !== null);
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
      if (node.dataset.filter) { setFilter(node.dataset.filter); state.zone = "grid"; layoutFocus(true); }
      else node.click();
      return;
    }
    const game = focusedGame();
    if (game) launch(game, false);
    else $("empty-action").click();
  }

  function back() {
    if (state.detailId) return closeDetail();
    if (state.zone === "sidebar") { state.zone = "grid"; sound.play("back"); return layoutFocus(true); }
    if (state.query) { setQuery(""); sound.play("back"); return; }
    if (state.filter !== "all") { setFilter("all"); sound.play("back"); }
  }

  function toggleSidebar(force) {
    state.collapsed = typeof force === "boolean" ? force : !state.collapsed;
    document.body.classList.toggle("collapsed", state.collapsed);
    el.collapse.setAttribute("aria-expanded", String(!state.collapsed));
    el.collapse.setAttribute("aria-label", state.collapsed ? "Expand the sidebar" : "Collapse the sidebar");
    if (!state.capturing) store("gamestrike-sidebar", state.collapsed ? "collapsed" : "open");
  }

  function setQuery(query) {
    state.query = query.trim().slice(0, 80);
    if (el.search.value.trim() !== state.query) el.search.value = state.query;
    state.gridIndex = 0;
    render({ focus: false });
  }

  // ---------- detail blade ----------
  function openDetail() {
    const game = state.zone === "grid" ? state.visible[state.gridIndex] : null;
    if (!game) return;
    state.detailId = game.id;
    renderDetail();
    el.detail.classList.add("open");
    el.detail.removeAttribute("inert");
    el.detail.setAttribute("aria-hidden", "false");
    sound.play("open");
    $("d-play").focus({ preventScroll: true });
  }

  function closeDetail({ restore = true, silent = false } = {}) {
    if (!state.detailId) return;
    state.detailId = null;
    el.detail.classList.remove("open");
    el.detail.setAttribute("inert", "");
    el.detail.setAttribute("aria-hidden", "true");
    if (!silent) sound.play("back");
    if (restore) layoutFocus(true);
  }

  function renderDetail() {
    const game = gameById(state.detailId);
    if (!game) return;
    const cover = document.createElement("div");
    cover.className = "tile";
    cover.append(coverNode(game));
    $("detail-cover").replaceChildren(cover);
    $("detail-kicker").textContent = core.cleanSource(game.source) || sourceLabel(game);
    $("detail-title").textContent = game.title;
    const facts = [
      ["Imported as", sourceLabel(game)],
      ["Source", core.cleanSource(game.source) || "Not set"],
      ["Last played", core.formatWhen(game.last_played)],
      ["Plays", String(game.play_count || 0)],
      ["Game data", core.formatBytes(game.save_bytes)],
      ["Added", game.added_at ? core.formatWhen(game.added_at) : "Earlier"],
      ["Cover", game.cover === "custom" ? "Your image" : game.cover === "auto" ? "From the game's files" : "Drawn from the title"],
      ["Needs", "Controller and network needs not checked"],
    ];
    $("detail-facts").replaceChildren(...facts.flatMap(([name, value]) => {
      const dt = document.createElement("dt");
      dt.textContent = name;
      const dd = document.createElement("dd");
      dd.textContent = value;
      return [dt, dd];
    }));
    const note = $("detail-note");
    note.classList.toggle("warn", !game.files_ok);
    note.textContent = !game.files_ok ? "This game's files are missing. Remove it, then add the original file again."
      : game.id === state.running ? "Playing now in its own window."
      : state.running ? "Another game is open. Close it to play this one." : "";
    const busy = Boolean(state.running) || state.launching;
    $("d-play").disabled = busy || !game.files_ok;
    $("d-network").disabled = busy || !game.files_ok;
    $("d-favorite").textContent = game.favorite ? "★ Favorite" : "☆ Add to favorites";
    $("d-favorite").setAttribute("aria-pressed", String(Boolean(game.favorite)));
    $("d-undo-cover").disabled = !game.cover_undo;
    $("d-reset-cover").disabled = game.cover !== "custom";
    $("d-reset").disabled = !game.save_bytes || game.id === state.running;
    $("d-remove").disabled = game.id === state.running;
  }

  // ---------- actions ----------
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

  function toggleFavorite(game) {
    if (!game) return;
    sound.play("select");
    send({ type: "set_favorite", id: game.id, favorite: !game.favorite });
  }

  function cycleSort() {
    if (state.filter === "recent") return;
    state.sort = SORTS[(SORTS.findIndex(([key]) => key === state.sort) + 1) % SORTS.length][0];
    store("gamestrike-sort", state.sort);
    sound.play("select");
    render({ focus: false });
  }

  // ---------- dialogs ----------
  const dialogs = {
    network: { node: $("network-dialog"), first: "network-cancel" },
    rename: { node: $("rename-dialog"), first: "rename-input" },
    source: { node: $("source-dialog"), first: "source-input" },
    reset: { node: $("reset-dialog"), first: "reset-cancel" },
    remove: { node: $("remove-dialog"), first: "remove-cancel" },
    settings: { node: $("settings-dialog"), first: "settings-sound" },
    problems: { node: $("problems-dialog"), first: "problems-close" },
    about: { node: $("about-dialog"), first: "close-about" },
  };
  const GAME_DIALOGS = ["network", "rename", "source", "reset", "remove"];
  let dialogGame = null;
  let returnFocus = null;

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
    if (name === "remove") { $("remove-title").textContent = `Remove ${game.title}?`; $("remove-keep").checked = true; }
    if (name === "settings") {
      $("settings-sound").textContent = `Sounds: ${sound.enabled ? "On" : "Off"}`;
      $("settings-sound").setAttribute("aria-pressed", String(sound.enabled));
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
    sound.play("open");
    const first = $(dialogs[name].first);
    first.focus();
    if (name === "rename" || name === "source") first.select();
  }

  function closeDialog({ restore = true, silent = false } = {}) {
    if (!state.dialog) return;
    dialogs[state.dialog].node.hidden = true;
    state.dialog = null;
    if (!silent) sound.play("back");
    if (!restore) return;
    if (state.detailId && returnFocus && el.detail.contains(returnFocus) && !returnFocus.disabled) returnFocus.focus();
    else if (state.detailId) $("d-close").focus();
    else layoutFocus(true);
  }

  function moveDialogFocus(step) {
    if (!state.dialog) return;
    moveWithin(dialogs[state.dialog].node, step < 0 ? "up" : "down");
  }

  $("network-cancel").addEventListener("click", () => closeDialog());
  $("network-confirm").addEventListener("click", () => { const game = dialogGame; closeDialog({ silent: true }); closeDetail({ restore: false, silent: true }); launch(game, true); });
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
    if (game) send({ type: "remove_game", id: game.id, keep_saves: keep });
    layoutFocus(true);
  });
  $("settings-sound").addEventListener("click", () => {
    sound.toggle();
    $("settings-sound").textContent = `Sounds: ${sound.enabled ? "On" : "Off"}`;
    $("settings-sound").setAttribute("aria-pressed", String(sound.enabled));
  });
  $("settings-problems").addEventListener("click", () => openDialog("problems"));
  $("settings-about").addEventListener("click", () => openDialog("about"));
  $("settings-close").addEventListener("click", () => closeDialog());
  $("problems-close").addEventListener("click", () => closeDialog());
  $("close-about").addEventListener("click", () => closeDialog());
  for (const { node } of Object.values(dialogs)) {
    node.addEventListener("mousedown", (event) => { if (event.target === node) closeDialog(); });
  }

  // Detail blade buttons.
  const detailGame = () => gameById(state.detailId);
  $("d-play").addEventListener("click", () => { const game = detailGame(); closeDetail({ restore: false, silent: true }); launch(game, false); layoutFocus(true); });
  $("d-network").addEventListener("click", () => openDialog("network"));
  $("d-favorite").addEventListener("click", () => toggleFavorite(detailGame()));
  $("d-cover").addEventListener("click", () => { const game = detailGame(); if (game) { sound.play("open"); send({ type: "pick_cover", id: game.id }); } });
  $("d-undo-cover").addEventListener("click", () => { const game = detailGame(); if (game) send({ type: "undo_cover", id: game.id }); });
  $("d-reset-cover").addEventListener("click", () => { const game = detailGame(); if (game) send({ type: "reset_cover", id: game.id }); });
  $("d-source").addEventListener("click", () => openDialog("source"));
  $("d-rename").addEventListener("click", () => openDialog("rename"));
  $("d-reset").addEventListener("click", () => openDialog("reset"));
  $("d-remove").addEventListener("click", () => openDialog("remove"));
  $("d-close").addEventListener("click", () => closeDetail());

  // Sidebar and header controls.
  $("add-game").addEventListener("click", importGame);
  $("settings").addEventListener("click", () => openDialog("settings"));
  $("power").addEventListener("click", () => { if (state.running) toast("Close the game first.", true); else send({ type: "quit" }); });
  el.collapse.addEventListener("click", () => { toggleSidebar(); sound.play("zone"); });
  el.sort.addEventListener("click", cycleSort);
  el.search.addEventListener("input", () => setQuery(el.search.value));
  el.search.addEventListener("keydown", (event) => {
    if (event.key === "Enter" || event.key === "ArrowDown") { event.preventDefault(); event.stopPropagation(); state.zone = "grid"; layoutFocus(true); }
    if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); if (el.search.value) setQuery(""); else { state.zone = "grid"; layoutFocus(true); } }
  });
  $("empty-action").addEventListener("click", () => {
    const action = $("empty-action").dataset.action;
    if (action === "add") importGame();
    else if (action === "clear") setQuery("");
    else setFilter("all");
  });

  // ---------- keyboard ----------
  document.addEventListener("keydown", (event) => {
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
      if (event.key === "Escape" || event.key === "Backspace") { event.preventDefault(); closeDetail(); return; }
      if (event.key === "Tab") { event.preventDefault(); moveWithin($("detail-actions"), event.shiftKey ? "up" : "down"); return; }
      const arrows = { ArrowLeft: "left", ArrowRight: "right", ArrowUp: "up", ArrowDown: "down" };
      if (arrows[event.key]) { event.preventDefault(); moveWithin($("detail-actions"), arrows[event.key]); return; }
      if (event.key.toLowerCase() === "f") { event.preventDefault(); toggleFavorite(detailGame()); }
      return;
    }
    const arrows = { ArrowLeft: "left", ArrowRight: "right", ArrowUp: "up", ArrowDown: "down", Home: "home", End: "end" };
    if (arrows[event.key]) { event.preventDefault(); navigate(arrows[event.key]); return; }
    switch (event.key) {
      case "Enter": case " ": event.preventDefault(); activate(); break;
      case "i": case "I": case "ContextMenu": event.preventDefault(); openDetail(); break;
      case "n": case "N": event.preventDefault(); if (state.zone === "grid") openDialog("network"); break;
      case "f": case "F": event.preventDefault(); if (state.zone === "grid") toggleFavorite(focusedGame()); break;
      case "F2": event.preventDefault(); if (state.zone === "grid") openDialog("rename"); break;
      case "Delete": event.preventDefault(); if (state.zone === "grid") openDialog("remove"); break;
      case "/": event.preventDefault(); focusSearch(); break;
      case "[": event.preventDefault(); toggleSidebar(); break;
      case "Escape": case "Backspace": event.preventDefault(); back(); break;
    }
  });

  function focusSearch() {
    if (state.collapsed) toggleSidebar(false);
    el.search.focus();
    el.search.select();
  }

  // ---------- controller (launcher navigation only; games handle their own input) ----------
  const readPad = core.createPadReader();
  function pollPad(now) {
    const pad = Array.from(navigator.getGamepads?.() || []).find(Boolean) || null;
    const actions = readPad(pad, now);
    if (actions.length && !state.booted) { finishBoot(); requestAnimationFrame(pollPad); return; }
    // Ignore input while a game window is open or this window is in the background.
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
    if (state.detailId) {
      if (action === "b") closeDetail();
      else if (action === "a") document.activeElement?.click();
      else if (action === "x") openDialog("network");
      else if (["left", "right", "up", "down"].includes(action)) moveWithin($("detail-actions"), action);
      return;
    }
    if (["left", "right", "up", "down"].includes(action)) navigate(action);
    else if (action === "a") activate();
    else if (action === "x" && state.zone === "grid") openDialog("network");
    else if (action === "y" && state.zone === "grid") openDetail();
    else if (action === "b") back();
  }

  // ---------- messages from the trusted Rust launcher ----------
  function receive(message) {
    switch (message.type) {
      case "library": {
        state.games = Array.isArray(message.games) ? message.games : [];
        state.problems = Array.isArray(message.problems) ? message.problems : [];
        state.running = message.running || null;
        state.loaded = true;
        render({ focus: !state.dialog && state.booted && !state.detailId && document.activeElement !== el.search });
        if (state.pendingFocus) {
          const index = state.visible.findIndex((game) => game.id === state.pendingFocus);
          state.pendingFocus = null;
          if (index >= 0) { state.zone = "grid"; state.gridIndex = index; layoutFocus(true); }
        }
        break;
      }
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
        toast(message.network ? "Game opened with network access." : "Game opened offline in its own window.");
        render();
        break;
      case "player_exited":
        if (state.running === message.id) state.running = null;
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

  // Used only by the launcher's explicit --capture-view option to stage a screenshot.
  function capture(view) {
    // Captures start from the same look and never change saved preferences.
    state.capturing = true;
    if (view === "boot") {
      el.boot.style.setProperty("--seek", "2.9s");
      el.boot.classList.add("frozen");
      clearTimeout(bootTimer);
      return;
    }
    finishBoot();
    document.body.classList.add("reduce-motion");
    toggleSidebar(view === "collapsed");
    state.filter = view === "favorites" ? "favorites" : "all";
    if (view === "search") { el.search.value = "orbit"; setQuery("orbit"); }
    state.zone = "grid";
    state.gridIndex = 0;
    render({ focus: false });
    if (view === "long") {
      let best = 0;
      state.visible.forEach((game, index) => { if (game.title.length > state.visible[best].title.length) best = index; });
      state.gridIndex = best;
    }
    layoutFocus(false);
    if (view === "launch") openDialog("network");
    else if (view === "manage") openDetail();
    else if (view === "about") openDialog("about");
  }

  window.GameStrike = Object.freeze({ receive, capture });
  el.collapse.setAttribute("aria-expanded", String(!state.collapsed));
  startBoot();
  requestAnimationFrame(pollPad);
  send({ type: "ready" });
})();
