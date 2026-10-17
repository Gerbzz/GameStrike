(() => {
  "use strict";

  const $ = (id) => document.getElementById(id);
  const state = { games: [], visibleGames: [], selectedId: null, launchingId: null, importing: false, pendingId: null, lastFocus: null };
  const send = (message) => window.ipc.postMessage(JSON.stringify(message));
  const shell = $("shell");
  const primary = $("primary-action");
  const add = $("add-button");
  const shelf = $("library-content");
  const search = $("library-search");
  const sort = $("library-sort");
  const dialog = $("launch-dialog");
  const about = $("about-dialog");
  const toast = $("toast");

  function status(message, isError = false) {
    toast.textContent = message;
    toast.classList.toggle("error", isError);
    toast.hidden = false;
    clearTimeout(status.timer);
    if (!isError) status.timer = setTimeout(() => { toast.hidden = true; }, 6000);
  }

  function importGame() {
    if (!state.importing) send({ type: "pick_and_import" });
  }

  function openLaunch(id) {
    const game = state.games.find((entry) => entry.id === id);
    if (!game || state.importing || state.launchingId || !about.hidden) return;
    state.pendingId = id;
    state.lastFocus = document.activeElement;
    $("dialog-title").textContent = `Open ${game.title}?`;
    dialog.hidden = false;
    $("cancel-launch").focus();
  }

  function closeLaunch() {
    dialog.hidden = true;
    state.pendingId = null;
    if (state.lastFocus && document.contains(state.lastFocus)) state.lastFocus.focus();
  }

  function openAbout() {
    if (!dialog.hidden || !about.hidden) return;
    state.lastFocus = document.activeElement;
    about.hidden = false;
    $("close-about").focus();
  }

  function closeAbout() {
    about.hidden = true;
    if (state.lastFocus && document.contains(state.lastFocus)) state.lastFocus.focus();
  }

  function choose(id, focus = false) {
    if (!state.visibleGames.some((game) => game.id === id)) return;
    state.selectedId = id;
    const index = state.visibleGames.findIndex((game) => game.id === id);
    $("hero-index").textContent = `/ ${String(index + 1).padStart(2, "0")}`;
    $("hero-label").textContent = "SELECTED GAME";
    const selected = state.visibleGames[index];
    $("hero-title").replaceChildren(document.createTextNode(selected.title));
    $("hero-description").textContent = "Stored on this PC. Review access before opening this game in its own player window.";
    $("selected-details").hidden = false;
    $("detail-format").textContent = selected.source_kind === "zip" ? "ZIP archive" : "Single HTML file";
    $("detail-entry").textContent = selected.entry || "index.html";
    primary.innerHTML = "Review launch <span aria-hidden=\"true\">↗</span>";
    primary.onclick = () => openLaunch(id);
    for (const card of shelf.querySelectorAll(".game-card")) {
      const active = card.dataset.id === id;
      card.classList.toggle("selected", active);
      card.setAttribute("aria-pressed", String(active));
      if (active && focus) { card.focus(); card.scrollIntoView({ block: "nearest", inline: "nearest", behavior: "smooth" }); }
    }
    try { sessionStorage.setItem("gamestrike-last-game", id); } catch (_) { /* optional preference */ }
  }

  function visibleGames() {
    const query = search.value.trim().toLocaleLowerCase();
    const games = state.games.filter((game) => game.title.toLocaleLowerCase().includes(query));
    games.sort((a, b) => {
      const titles = a.title.localeCompare(b.title, undefined, { sensitivity: "base" }) || a.id.localeCompare(b.id);
      if (sort.value === "title-desc") return -titles;
      if (sort.value === "format") return a.source_kind.localeCompare(b.source_kind) || titles;
      return titles;
    });
    return games;
  }

  function renderLibrary() {
    const games = state.visibleGames = visibleGames();
    const restoreCardFocus = document.activeElement?.className?.split(" ").includes("game-card") || false;
    shell.classList.toggle("has-games", state.games.length > 0);
    $("game-count").textContent = `${state.games.length} ${state.games.length === 1 ? "game" : "games"}`;
    $("visible-count").textContent = games.length === state.games.length ? "" : `${games.length} of ${state.games.length} shown`;
    shelf.replaceChildren();
    if (!games.length) {
      const empty = document.createElement("div"); empty.className = "empty-state";
      const heading = document.createElement("h3"); heading.textContent = state.games.length ? "No matching games" : "Your shelf starts here";
      const body = document.createElement("p"); body.textContent = state.games.length ? "Try a different title, or clear the search to see your collection." : "Choose a downloaded HTML file or a ZIP with an index.html at its root. Your files stay on this PC.";
      empty.append(heading, body); shelf.append(empty);
      state.selectedId = null;
      $("hero-title").innerHTML = state.games.length ? "Nothing found." : "Your games.<br><em>Right here.</em>";
      $("hero-description").textContent = state.games.length ? "Your collection is still here. Clear the search to browse every game." : "A home for the HTML games you keep. Add a downloaded HTML file or ZIP archive, then launch it in its own player window.";
      $("selected-details").hidden = true;
      $("hero-index").textContent = "/ 00";
      $("hero-label").textContent = "WELCOME";
      primary.innerHTML = "+ Add a game <span aria-hidden=\"true\">↗</span>";
      if (state.games.length) primary.textContent = "Clear search";
      primary.onclick = state.games.length ? () => { search.value = ""; renderLibrary(); search.focus(); } : importGame;
      if (restoreCardFocus) primary.focus();
      return;
    }
    const row = document.createElement("div"); row.className = "game-row";
    games.forEach((game, index) => {
      const card = document.createElement("button"); card.type = "button"; card.className = "game-card"; card.dataset.id = game.id;
      card.setAttribute("aria-label", `${game.title}, review launch, controller support unknown, network requirement unknown`);
      const indexText = document.createElement("span"); indexText.className = "card-index"; indexText.textContent = `GAME ${String(index + 1).padStart(2, "0")}`;
      const name = document.createElement("span"); name.className = "card-name"; name.textContent = game.title;
      const details = document.createElement("span"); details.className = "card-details"; details.textContent = `${game.source_kind === "zip" ? "ZIP game" : "HTML game"} · Stored locally`;
      const status = document.createElement("span"); status.className = "card-status"; status.textContent = "Controller unknown · Network unknown";
      card.append(indexText, name, details, status);
      card.addEventListener("focus", () => choose(game.id));
      card.addEventListener("click", () => { choose(game.id); openLaunch(game.id); });
      row.append(card);
    });
    shelf.append(row);
    let previous = state.selectedId;
    if (!previous) { try { previous = sessionStorage.getItem("gamestrike-last-game"); } catch (_) {} }
    choose(games.some((game) => game.id === previous) ? previous : games[0].id, restoreCardFocus);
  }

  function moveSelection(direction) {
    if (dialog.hidden && about.hidden && state.visibleGames.length) {
      const index = Math.max(0, state.visibleGames.findIndex((game) => game.id === state.selectedId));
      choose(state.visibleGames[(index + direction + state.visibleGames.length) % state.visibleGames.length].id, true);
    }
  }

  function moveDirection(direction) {
    if (!about.hidden) return;
    if (!dialog.hidden) {
      $(direction < 0 ? "cancel-launch" : "confirm-launch").focus();
    } else {
      moveSelection(direction);
    }
  }

  function activateLaunchChoice() {
    const focused = document.activeElement;
    (focused === $("confirm-launch") ? $("confirm-launch") : $("cancel-launch")).click();
  }

  window.GameStrike = Object.freeze({ receive(message) {
    switch (message.type) {
      case "library": state.games = Array.isArray(message.games) ? message.games : []; renderLibrary(); break;
      case "importing": state.importing = true; primary.disabled = true; add.disabled = true; status(`Importing ${message.name}…`); break;
      case "imported":
        state.importing = false; primary.disabled = false; add.disabled = false;
        search.value = "";
        state.selectedId = message.game.id;
        status(message.already_present ? `${message.game.title} is already in your library. Its saves were kept.` : `${message.game.title} added to your library.`);
        break;
      case "player_started": state.launchingId = message.id; status("Game opened in its own player window."); break;
      case "player_exited":
        if (message.id === state.launchingId) {
          state.launchingId = null;
          status(message.message, !message.message.startsWith("Player closed"));
        }
        break;
      case "error":
        if (message.operation === "import") {
          state.importing = false; primary.disabled = false; add.disabled = false;
        } else if (message.operation === "launch" && message.id === state.launchingId) {
          state.launchingId = null;
        }
        status(message.message, true);
        break;
    }
  }});

  primary.onclick = importGame;
  add.addEventListener("click", importGame);
  search.addEventListener("input", renderLibrary);
  sort.addEventListener("change", renderLibrary);
  $("about-button").addEventListener("click", openAbout);
  $("close-about").addEventListener("click", closeAbout);
  about.addEventListener("click", (event) => { if (event.target === about) closeAbout(); });
  $("cancel-launch").addEventListener("click", closeLaunch);
  $("confirm-launch").addEventListener("click", () => {
    if (state.pendingId) {
      state.launchingId = state.pendingId;
      send({ type: "launch_game", id: state.pendingId, allow_network: true });
    }
    closeLaunch();
  });
  dialog.addEventListener("click", (event) => { if (event.target === dialog) closeLaunch(); });
  document.addEventListener("keydown", (event) => {
    if (!about.hidden) {
      if (event.key === "Escape") { event.preventDefault(); closeAbout(); }
      if (event.key === "Tab") { event.preventDefault(); $("close-about").focus(); }
      return;
    }
    if (!dialog.hidden) {
      if (event.key === "Escape") { event.preventDefault(); closeLaunch(); }
      if (event.key === "ArrowLeft" || event.key === "ArrowRight") {
        event.preventDefault(); moveDirection(event.key === "ArrowLeft" ? -1 : 1);
      }
      if (event.key === "Tab") {
        const back = $("cancel-launch"), confirm = $("confirm-launch");
        if (event.shiftKey && document.activeElement === back) { event.preventDefault(); confirm.focus(); }
        else if (!event.shiftKey && document.activeElement === confirm) { event.preventDefault(); back.focus(); }
      }
      return;
    }
    if (document.activeElement === search || document.activeElement === sort) {
      if (event.key === "Escape" && document.activeElement === search && search.value) {
        event.preventDefault(); search.value = ""; renderLibrary();
      }
      return;
    }
    if (event.key === "ArrowRight" || event.key === "ArrowLeft") {
      event.preventDefault(); moveDirection(event.key === "ArrowRight" ? 1 : -1);
    } else if (event.key === "Escape") {
      if (state.games.length) { event.preventDefault(); primary.focus(); }
    }
  });

  // Launcher navigation only. Games decide their own controller support.
  let previousA = false, previousB = false, lastDirection = "", lastDirectionAt = 0;
  function pollGamepad() {
    const pad = Array.from(navigator.getGamepads?.() || []).find((entry) => entry);
    if (pad) {
      const pressed = (i) => Boolean(pad.buttons[i]?.pressed);
      const a = pressed(0), b = pressed(1);
      const direction = pressed(14) || pad.axes[0] < -0.6 ? "left" : pressed(15) || pad.axes[0] > 0.6 ? "right" : "";
      const now = performance.now();
      if (b && !previousB && !about.hidden) closeAbout();
      else if (b && !previousB && !dialog.hidden) closeLaunch();
      else if (a && !previousA) {
        if (!about.hidden) closeAbout();
        else if (!dialog.hidden) activateLaunchChoice();
        else if (state.selectedId) openLaunch(state.selectedId);
        else importGame();
      } else if (direction && (direction !== lastDirection || now - lastDirectionAt > 250)) {
        lastDirectionAt = now;
        lastDirection = direction;
        moveDirection(direction === "left" ? -1 : 1);
      }
      previousA = a;
      previousB = b;
      if (!direction) lastDirection = "";
    } else {
      previousA = false;
      previousB = false;
      lastDirection = "";
    }
    requestAnimationFrame(pollGamepad);
  }
  requestAnimationFrame(pollGamepad);
  send({ type: "ready" });
})();
