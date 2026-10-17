(() => {
  "use strict";

  const core = window.GameStrikeCore;
  const $ = (id) => document.getElementById(id);
  const send = (message) => window.ipc.postMessage(JSON.stringify(message));
  const startedAt = performance.now();

  const state = {
    loaded: false,
    games: [],
    visible: [],
    problems: [],
    running: null,
    selectedId: null,
    importing: false,
    launching: false,
    dialog: null,
    returnFocus: null,
  };

  const el = {
    shell: $("shell"), splash: $("splash"), content: $("library-content"), details: $("details"),
    search: $("library-search"), sort: $("library-sort"), add: $("add-button"), about: $("about-button"),
    count: $("game-count"), visibleCount: $("visible-count"), problems: $("problems-button"),
    play: $("play-button"), network: $("network-button"), rename: $("rename-button"), reset: $("reset-button"),
    remove: $("remove-button"), note: $("detail-note"), nowPlaying: $("now-playing"), toast: $("toast"),
  };

  // ---------- helpers ----------
  function selected() { return state.games.find((game) => game.id === state.selectedId) || null; }
  function gameById(id) { return state.games.find((game) => game.id === id) || null; }

  function toast(message, isError = false) {
    el.toast.textContent = message;
    el.toast.classList.toggle("error", isError);
    el.toast.hidden = false;
    clearTimeout(toast.timer);
    toast.timer = setTimeout(() => { el.toast.hidden = true; }, isError ? 9000 : 5000);
  }

  function setCover(node, game) {
    const art = core.cover(game.id, game.title);
    node.className = `cover p${art.pattern}${node.classList.contains("large") ? " large" : ""}`;
    node.style.setProperty("--h1", art.hue);
    node.style.setProperty("--h2", art.hue2);
    node.style.setProperty("--a", `${art.angle}deg`);
    node.querySelector("span").textContent = art.initials;
  }

  function badgesFor(game) {
    const list = [];
    if (game.id === state.running) list.push(["playing", "Playing"]);
    if (!game.files_ok) list.push(["missing", "Files missing"]);
    return list;
  }

  function renderBadges(container, game) {
    container.replaceChildren(...badgesFor(game).map(([kind, text]) => {
      const badge = document.createElement("span");
      badge.className = `badge ${kind}`;
      badge.textContent = text;
      return badge;
    }));
  }

  function sourceLabel(game) { return game.source_kind === "zip" ? "ZIP archive" : "HTML file"; }

  function cardMeta(game) {
    if (!game.files_ok) return `${sourceLabel(game)} · needs re-adding`;
    return game.last_played ? `Played ${core.formatWhen(game.last_played).toLocaleLowerCase()}` : `${sourceLabel(game)} · Not played yet`;
  }

  // ---------- library rendering ----------
  function renderLibrary({ focus = false } = {}) {
    const cardHadFocus = document.activeElement?.classList?.contains("card");
    state.visible = core.filterAndSort(state.games, el.search.value, el.sort.value);
    const empty = state.games.length === 0;
    el.shell.classList.toggle("empty", empty);
    el.count.textContent = empty ? "" : `${state.games.length} ${state.games.length === 1 ? "game" : "games"}`;
    el.visibleCount.textContent = !empty && state.visible.length !== state.games.length ? `${state.visible.length} shown` : "";
    el.problems.hidden = state.problems.length === 0;
    el.problems.textContent = `${state.problems.length} unreadable ${state.problems.length === 1 ? "folder" : "folders"}`;
    el.search.disabled = empty;
    el.sort.disabled = empty;

    if (empty) {
      state.selectedId = null;
      el.details.hidden = true;
      el.content.replaceChildren(welcome());
      return;
    }
    if (!state.visible.length) {
      el.details.hidden = true;
      const box = document.createElement("div");
      box.className = "no-match";
      const heading = document.createElement("h2");
      heading.textContent = "No games match your search";
      const body = document.createElement("p");
      body.textContent = "Press Esc to clear the search and see your whole library.";
      box.append(heading, body);
      el.content.replaceChildren(box);
      return;
    }

    const grid = document.createElement("div");
    grid.className = "grid";
    grid.setAttribute("role", "listbox");
    grid.setAttribute("aria-label", "Games");
    for (const game of state.visible) grid.append(card(game));
    el.content.replaceChildren(grid);

    if (!state.visible.some((game) => game.id === state.selectedId)) {
      let remembered = null;
      try { remembered = sessionStorage.getItem("gamestrike-selected"); } catch (_) { /* optional */ }
      state.selectedId = state.visible.some((game) => game.id === remembered) ? remembered : state.visible[0].id;
    }
    select(state.selectedId, { focus: focus || cardHadFocus });
  }

  function card(game) {
    const node = document.createElement("button");
    node.type = "button";
    node.className = "card";
    node.dataset.id = game.id;
    node.setAttribute("role", "option");
    node.tabIndex = -1;
    node.setAttribute("aria-label", `${game.title}. ${cardMeta(game)}.${game.id === state.running ? " Playing now." : ""}`);
    const art = document.createElement("div");
    art.className = "cover";
    art.setAttribute("aria-hidden", "true");
    art.append(document.createElement("span"));
    setCover(art, game);
    const badges = document.createElement("div");
    badges.className = "badges";
    renderBadges(badges, game);
    const body = document.createElement("div");
    body.className = "card-body";
    const title = document.createElement("span");
    title.className = "card-title";
    title.textContent = game.title;
    const meta = document.createElement("span");
    meta.className = "card-meta";
    meta.textContent = cardMeta(game);
    body.append(title, meta);
    node.append(art, badges, body);
    node.addEventListener("click", () => select(game.id, { focus: true }));
    node.addEventListener("dblclick", () => playOffline());
    node.addEventListener("focus", () => { if (state.selectedId !== game.id) select(game.id); });
    return node;
  }

  function welcome() {
    const box = document.createElement("section");
    box.className = "welcome";
    const mark = document.createElement("img");
    mark.src = "/mark.svg";
    mark.alt = "";
    const copy = document.createElement("div");
    const heading = document.createElement("h2");
    heading.textContent = "Bring your HTML games home";
    const text = document.createElement("p");
    text.textContent = "Add a game you downloaded, as a single .html file or a .zip with index.html inside. GameStrike keeps a copy on this PC and opens it in its own player window.";
    const list = document.createElement("ul");
    for (const item of ["Offline play by default, network only when you allow it", "Every game keeps its own saves", "Nothing is uploaded and no account is needed"]) {
      const li = document.createElement("li");
      li.textContent = item;
      list.append(li);
    }
    const button = document.createElement("button");
    button.type = "button";
    button.id = "welcome-add";
    button.className = "button primary large";
    button.textContent = "Add your first game";
    button.disabled = state.importing;
    button.addEventListener("click", importGame);
    copy.append(heading, text, list, button);
    box.append(mark, copy);
    return box;
  }

  function select(id, { focus = false } = {}) {
    const game = gameById(id);
    if (!game) return;
    state.selectedId = id;
    try { sessionStorage.setItem("gamestrike-selected", id); } catch (_) { /* optional */ }
    for (const node of el.content.querySelectorAll(".card")) {
      const active = node.dataset.id === id;
      node.classList.toggle("selected", active);
      node.setAttribute("aria-selected", String(active));
      node.tabIndex = active ? 0 : -1;
      if (active && focus) {
        node.focus({ preventScroll: true });
        const reduce = window.matchMedia?.("(prefers-reduced-motion: reduce)")?.matches;
        node.scrollIntoView({ block: "nearest", behavior: reduce ? "auto" : "smooth" });
      }
    }
    renderDetails();
  }

  function renderDetails() {
    const game = selected();
    if (!game) { el.details.hidden = true; return; }
    el.details.hidden = false;
    setCover($("detail-cover"), game);
    renderBadges($("detail-badges"), game);
    $("detail-title").textContent = game.title;
    $("detail-source").textContent = sourceLabel(game);
    $("detail-added").textContent = game.added_at ? core.formatWhen(game.added_at) : "Before play history";
    $("detail-played").textContent = game.play_count ? `${core.formatWhen(game.last_played)} · ${game.play_count} ${game.play_count === 1 ? "time" : "times"}` : "Never";
    $("detail-saves").textContent = core.formatBytes(game.save_bytes);
    const isRunning = game.id === state.running;
    const busy = Boolean(state.running) || state.launching;
    el.play.disabled = busy || !game.files_ok;
    el.network.disabled = busy || !game.files_ok;
    el.reset.disabled = isRunning || !game.save_bytes;
    el.remove.disabled = isRunning;
    el.note.classList.toggle("warn", !game.files_ok || isRunning);
    el.note.textContent = !game.files_ok
      ? "This game's files are missing from the library folder. Remove it, then add the original file again."
      : isRunning
        ? "Playing now in its own window. Close that window to return here."
        : state.running
          ? "Another game is open. Close it to play this one."
          : "Offline play blocks the game's network requests. Its saves stay in this game's own browser profile.";
  }

  function columns() {
    const cards = el.content.querySelectorAll(".card");
    if (cards.length < 2) return 1;
    const top = cards[0].offsetTop;
    let count = 0;
    for (const node of cards) { if (node.offsetTop !== top) break; count += 1; }
    return Math.max(1, count);
  }

  function move(direction) {
    if (!state.visible.length) return;
    const index = state.visible.findIndex((game) => game.id === state.selectedId);
    const next = core.gridMove(index, state.visible.length, columns(), direction);
    select(state.visible[next].id, { focus: true });
  }

  // ---------- actions ----------
  function importGame() {
    if (state.importing) return;
    state.importing = true;
    syncBusy();
    send({ type: "pick_and_import" });
  }

  function syncBusy() {
    el.add.disabled = state.importing;
    const welcomeAdd = $("welcome-add");
    if (welcomeAdd) welcomeAdd.disabled = state.importing;
    renderDetails();
  }

  function launch(network) {
    const game = selected();
    if (!game || state.running || state.launching || !game.files_ok) return;
    state.launching = true;
    renderDetails();
    send({ type: "launch_game", id: game.id, network });
  }

  function playOffline() { if (!state.dialog) launch(false); }

  // ---------- dialogs ----------
  const dialogs = {
    network: { node: $("network-dialog"), first: "network-cancel" },
    manage: { node: $("manage-dialog"), first: "manage-rename" },
    rename: { node: $("rename-dialog"), first: "rename-input" },
    reset: { node: $("reset-dialog"), first: "reset-cancel" },
    remove: { node: $("remove-dialog"), first: "remove-cancel" },
    problems: { node: $("problems-dialog"), first: "problems-close" },
    about: { node: $("about-dialog"), first: "close-about" },
  };

  function openDialog(name) {
    const game = selected();
    const needsGame = ["network", "manage", "rename", "reset", "remove"].includes(name);
    if (needsGame && !game) return;
    if (name === "network" && (state.running || state.launching || !game.files_ok)) return;
    if ((name === "reset" || name === "remove") && game.id === state.running) return;
    if (name === "reset" && !game.save_bytes) return;
    if (state.dialog) closeDialog({ restore: false });
    else state.returnFocus = document.activeElement;
    if (name === "network") $("network-title").textContent = `Play ${game.title} with network?`;
    if (name === "manage") {
      $("manage-title").textContent = game.title;
      $("manage-reset").disabled = !game.save_bytes || game.id === state.running;
      $("manage-remove").disabled = game.id === state.running;
    }
    if (name === "rename") {
      $("rename-input").value = game.title;
      $("rename-error").hidden = true;
    }
    if (name === "reset") $("reset-title").textContent = `Clear saves for ${game.title}?`;
    if (name === "remove") {
      $("remove-title").textContent = `Remove ${game.title}?`;
      $("remove-keep").checked = true;
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
    const first = $(dialogs[name].first);
    first.focus();
    if (name === "rename") first.select();
  }

  function closeDialog({ restore = true } = {}) {
    if (!state.dialog) return;
    dialogs[state.dialog].node.hidden = true;
    state.dialog = null;
    if (!restore) return;
    const target = state.returnFocus && document.contains(state.returnFocus) && !state.returnFocus.disabled ? state.returnFocus : null;
    if (target) target.focus();
    else el.content.querySelector(".card.selected")?.focus();
  }

  function dialogFocusables() {
    if (!state.dialog) return [];
    return [...dialogs[state.dialog].node.querySelectorAll("button, input, select")].filter((node) => !node.disabled && !node.hidden && node.offsetParent !== null);
  }

  function moveDialogFocus(step) {
    const items = dialogFocusables();
    if (!items.length) return;
    const index = items.indexOf(document.activeElement);
    items[(index + step + items.length) % items.length].focus();
  }

  $("network-cancel").addEventListener("click", () => closeDialog());
  $("network-confirm").addEventListener("click", () => { closeDialog(); launch(true); });
  $("manage-rename").addEventListener("click", () => openDialog("rename"));
  $("manage-reset").addEventListener("click", () => openDialog("reset"));
  $("manage-remove").addEventListener("click", () => openDialog("remove"));
  $("manage-close").addEventListener("click", () => closeDialog());
  $("rename-cancel").addEventListener("click", () => closeDialog());
  $("rename-form").addEventListener("submit", (event) => {
    event.preventDefault();
    const game = selected();
    const title = core.cleanTitle($("rename-input").value);
    if (!game || !title) { $("rename-error").hidden = false; $("rename-input").focus(); return; }
    closeDialog();
    if (title !== game.title) send({ type: "rename_game", id: game.id, title });
  });
  $("reset-cancel").addEventListener("click", () => closeDialog());
  $("reset-confirm").addEventListener("click", () => {
    const game = selected();
    closeDialog();
    if (game) send({ type: "reset_saves", id: game.id });
  });
  $("remove-cancel").addEventListener("click", () => closeDialog());
  $("remove-confirm").addEventListener("click", () => {
    const game = selected();
    const keep = $("remove-keep").checked;
    closeDialog({ restore: false });
    if (game) send({ type: "remove_game", id: game.id, keep_saves: keep });
  });
  $("problems-close").addEventListener("click", () => closeDialog());
  $("close-about").addEventListener("click", () => closeDialog());
  for (const { node } of Object.values(dialogs)) {
    node.addEventListener("mousedown", (event) => { if (event.target === node) closeDialog(); });
  }

  el.add.addEventListener("click", importGame);
  el.about.addEventListener("click", () => openDialog("about"));
  el.problems.addEventListener("click", () => openDialog("problems"));
  el.play.addEventListener("click", playOffline);
  el.network.addEventListener("click", () => openDialog("network"));
  el.rename.addEventListener("click", () => openDialog("rename"));
  el.reset.addEventListener("click", () => openDialog("reset"));
  el.remove.addEventListener("click", () => openDialog("remove"));
  el.search.addEventListener("input", () => renderLibrary());
  el.sort.addEventListener("change", () => renderLibrary());

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
    const target = document.activeElement;
    if (target === el.search || target === el.sort) {
      if (event.key === "Escape") {
        event.preventDefault();
        if (target === el.search && el.search.value) { el.search.value = ""; renderLibrary(); }
        else el.content.querySelector(".card.selected")?.focus();
      }
      if (event.key === "ArrowDown" && target === el.search) { event.preventDefault(); el.content.querySelector(".card.selected")?.focus(); }
      return;
    }
    if ((event.key === "f" && event.ctrlKey) || event.key === "/") { event.preventDefault(); el.search.focus(); return; }
    if (event.ctrlKey || event.altKey || event.metaKey) return;
    const onCard = target?.classList?.contains("card");
    const directions = { ArrowLeft: "left", ArrowRight: "right", ArrowUp: "up", ArrowDown: "down", Home: "home", End: "end" };
    if (directions[event.key] && (onCard || target === document.body)) { event.preventDefault(); move(directions[event.key]); return; }
    if (event.key === "Enter" && onCard) { event.preventDefault(); playOffline(); return; }
    if (!onCard && target !== document.body) return;
    if (event.key === "n" || event.key === "N") { event.preventDefault(); openDialog("network"); }
    else if (event.key === "F2") { event.preventDefault(); openDialog("rename"); }
    else if (event.key === "Delete") { event.preventDefault(); openDialog("remove"); }
    else if (event.key === "Escape" && el.search.value) { event.preventDefault(); el.search.value = ""; renderLibrary({ focus: true }); }
  });

  // ---------- controller (launcher navigation only; games handle their own input) ----------
  const readPad = core.createPadReader();
  function pollPad(now) {
    const pad = Array.from(navigator.getGamepads?.() || []).find(Boolean) || null;
    const actions = readPad(pad, now);
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
    if (!state.games.length) { if (action === "a") importGame(); return; }
    if (["left", "right", "up", "down"].includes(action)) move(action);
    else if (action === "a") playOffline();
    else if (action === "x") openDialog("network");
    else if (action === "y") openDialog("manage");
    else if (action === "b" && el.search.value) { el.search.value = ""; renderLibrary({ focus: true }); }
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
        renderLibrary({ focus: first });
        el.nowPlaying.textContent = state.running ? `Playing ${gameById(state.running)?.title || "a game"}` : "";
        if (first) {
          const wait = Math.max(0, 650 - (performance.now() - startedAt));
          setTimeout(() => el.splash.classList.add("done"), wait);
        }
        break;
      }
      case "importing":
        toast(`Adding ${message.name}…`);
        break;
      case "pick_cancelled":
        state.importing = false;
        syncBusy();
        break;
      case "imported": {
        state.importing = false;
        el.search.value = "";
        state.selectedId = message.game.id;
        const title = message.game.title;
        toast(message.already_present
          ? `${title} is already in your library. Its saves were kept.`
          : message.restored_saves
            ? `${title} is back, with the saves you kept.`
            : `${title} was added to your library.`);
        syncBusy();
        break;
      }
      case "player_started":
        state.launching = false;
        state.running = message.id;
        toast(message.network ? "Game opened with network access." : "Game opened offline in its own window.");
        renderLibrary();
        break;
      case "player_exited":
        if (state.running === message.id) state.running = null;
        state.launching = false;
        toast(message.message, !message.ok);
        renderLibrary({ focus: !state.dialog });
        break;
      case "managed":
        toast(message.message);
        break;
      case "removed": {
        const index = state.visible.findIndex((game) => game.id === message.id);
        const neighbour = state.visible[index + 1] || state.visible[index - 1];
        state.selectedId = neighbour ? neighbour.id : null;
        toast(message.message);
        break;
      }
      case "error":
        if (message.operation === "import") { state.importing = false; syncBusy(); }
        if (message.operation === "launch") { state.launching = false; renderDetails(); }
        toast(message.message, true);
        break;
    }
  }

  // Used only by the launcher's explicit --capture-view option to stage a screenshot.
  function capture(view) {
    if (view === "long") {
      const longest = [...state.visible].sort((a, b) => b.title.length - a.title.length)[0];
      if (longest) select(longest.id, { focus: true });
    } else if (view === "launch") {
      openDialog("network");
    } else if (view === "manage") {
      openDialog("manage");
    } else if (view === "about") {
      openDialog("about");
    }
  }

  window.GameStrike = Object.freeze({ receive, capture });
  requestAnimationFrame(pollPad);
  send({ type: "ready" });
})();
