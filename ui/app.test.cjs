const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");
const vm = require("node:vm");

function launcherHarness(games = [{ id: "game-1", title: "Safe Fixture", source_kind: "single_html" }], reducedMotion = false) {
  const nodes = new Map();
  const frames = [];
  const messages = [];
  let now = 0;

  function node(id = "") {
    const listeners = new Map();
    return {
      id,
      className: "",
      dataset: {},
      children: [],
      hidden: id === "launch-dialog" || id === "about-dialog" || id === "toast",
      disabled: false,
      value: id === "library-sort" ? "title-asc" : "",
      classList: { toggle() {} },
      addEventListener(type, callback) { listeners.set(type, callback); },
      dispatch(type) { listeners.get(type)?.({ target: this }); },
      append(...children) { this.children.push(...children); },
      replaceChildren(...children) { this.children = children; },
      setAttribute() {},
      scrollIntoView(options) { this.lastScrollOptions = options; },
      querySelectorAll(selector) {
        const found = [];
        function visit(item) {
          if (item.className?.split(" ").includes(selector.slice(1))) found.push(item);
          for (const child of item.children || []) visit(child);
        }
        visit(this);
        return found;
      },
      focus() { document.activeElement = this; listeners.get("focus")?.(); },
      click() { this.onclick?.(); listeners.get("click")?.({ target: this }); },
    };
  }

  const ids = ["shell", "primary-action", "add-button", "about-button", "about-dialog", "close-about", "library-content", "library-search", "library-sort", "visible-count", "launch-dialog", "toast", "hero-index", "hero-label", "hero-title", "hero-description", "selected-details", "detail-format", "detail-entry", "game-count", "dialog-title", "confirm-launch", "cancel-launch"];
  for (const id of ids) nodes.set(id, node(id));
  const document = {
    activeElement: nodes.get("primary-action"),
    getElementById: (id) => nodes.get(id),
    createElement: () => node(),
    createTextNode: (text) => ({ textContent: text }),
    addEventListener() {},
    contains: () => true,
  };
  const pad = { buttons: Array.from({ length: 16 }, () => ({ pressed: false })), axes: [0] };
  const storage = new Map();
  const context = {
    document,
    window: { ipc: { postMessage: (value) => messages.push(JSON.parse(value)) }, matchMedia: () => ({ matches: reducedMotion }) },
    navigator: { getGamepads: () => [pad] },
    performance: { now: () => now },
    requestAnimationFrame: (callback) => frames.push(callback),
    sessionStorage: { getItem: (key) => storage.get(key), setItem: (key, value) => storage.set(key, value) },
    setTimeout: () => 1,
    clearTimeout() {},
  };
  vm.runInNewContext(fs.readFileSync(path.join(__dirname, "app.js"), "utf8"), context);
  context.window.GameStrike.receive({ type: "library", games });
  return {
    document,
    nodes,
    pad,
    messages,
    receive: context.window.GameStrike.receive,
    frame(time) { now = time; assert.ok(frames.length, "a gamepad poll must be scheduled"); frames.shift()(); },
  };
}

test("controller focus respects reduced motion preference", () => {
  const games = [
    { id: "game-1", title: "First", source_kind: "single_html" },
    { id: "game-2", title: "Second", source_kind: "single_html" },
  ];
  for (const [reducedMotion, behavior] of [[false, "smooth"], [true, "auto"]]) {
    const app = launcherHarness(games, reducedMotion);
    app.pad.buttons[15].pressed = true;
    app.frame(0);
    const card = app.nodes.get("library-content").querySelectorAll(".game-card")[1];
    assert.equal(card.lastScrollOptions.behavior, behavior);
  }
});

test("a held controller A cannot accept network access and a fresh A activates Go back", () => {
  const app = launcherHarness();
  app.pad.buttons[0].pressed = true;
  app.frame(0);
  assert.equal(app.nodes.get("launch-dialog").hidden, false);
  assert.equal(app.document.activeElement, app.nodes.get("cancel-launch"));
  app.frame(300);
  app.frame(600);
  assert.equal(app.messages.filter((message) => message.type === "launch_game").length, 0);
  assert.equal(app.nodes.get("launch-dialog").hidden, false);

  app.pad.buttons[0].pressed = false;
  app.frame(620);
  app.pad.buttons[0].pressed = true;
  app.frame(680);
  assert.equal(app.messages.filter((message) => message.type === "launch_game").length, 0);
  assert.equal(app.nodes.get("launch-dialog").hidden, true);
  app.frame(1000);
  assert.equal(app.messages.filter((message) => message.type === "launch_game").length, 0);
  assert.equal(app.nodes.get("launch-dialog").hidden, true);
});

test("controller direction focuses Allow network and a separate fresh A confirms it", () => {
  const app = launcherHarness();
  app.pad.buttons[0].pressed = true;
  app.frame(0);
  app.pad.buttons[0].pressed = false;
  app.frame(20);
  app.pad.buttons[15].pressed = true;
  app.frame(40);
  assert.equal(app.document.activeElement, app.nodes.get("confirm-launch"));
  app.pad.buttons[0].pressed = true;
  app.frame(60);
  assert.equal(app.messages.filter((message) => message.type === "launch_game").length, 1);
  app.frame(400);
  assert.equal(app.messages.filter((message) => message.type === "launch_game").length, 1);
});

test("pressing A and right together cannot skip the default Go back choice", () => {
  const app = launcherHarness();
  app.pad.buttons[0].pressed = true;
  app.frame(0);
  app.pad.buttons[0].pressed = false;
  app.frame(20);
  app.pad.buttons[0].pressed = true;
  app.pad.buttons[15].pressed = true;
  app.frame(40);
  assert.equal(app.nodes.get("launch-dialog").hidden, true);
  assert.equal(app.messages.filter((message) => message.type === "launch_game").length, 0);
});

test("holding a direction repeats shelf movement without repeating A", () => {
  const app = launcherHarness([
    { id: "game-1", title: "First", source_kind: "single_html" },
    { id: "game-2", title: "Second", source_kind: "zip" },
  ]);
  app.pad.buttons[15].pressed = true;
  app.frame(0);
  assert.equal(app.nodes.get("hero-title").children[0].textContent, "Second");
  app.frame(100);
  assert.equal(app.nodes.get("hero-title").children[0].textContent, "Second");
  app.frame(300);
  assert.equal(app.nodes.get("hero-title").children[0].textContent, "First");
  assert.equal(app.messages.filter((message) => message.type === "launch_game").length, 0);
});

test("About keeps publisher credit away from play and controller A only closes it", () => {
  const app = launcherHarness();
  app.nodes.get("about-button").click();
  assert.equal(app.nodes.get("about-dialog").hidden, false);
  assert.equal(app.document.activeElement, app.nodes.get("close-about"));
  app.pad.buttons[0].pressed = true;
  app.frame(0);
  assert.equal(app.nodes.get("about-dialog").hidden, true);
  assert.equal(app.nodes.get("launch-dialog").hidden, true);
  assert.equal(app.messages.filter((message) => message.type === "launch_game").length, 0);
});

test("search filters real library cards and clearing it restores selection", () => {
  const app = launcherHarness([
    { id: "game-1", title: "Asteroid Run", source_kind: "single_html", entry: "index.html" },
    { id: "game-2", title: "Puzzle Box", source_kind: "zip", entry: "index.html" },
  ]);
  const search = app.nodes.get("library-search");
  search.value = "puzzle";
  search.dispatch("input");
  assert.equal(app.nodes.get("library-content").querySelectorAll(".game-card").length, 1);
  assert.equal(app.nodes.get("hero-title").children[0].textContent, "Puzzle Box");
  assert.equal(app.nodes.get("detail-format").textContent, "ZIP archive");
  assert.equal(app.nodes.get("visible-count").textContent, "1 of 2 shown");
  search.value = "missing";
  search.dispatch("input");
  assert.equal(app.nodes.get("library-content").querySelectorAll(".game-card").length, 0);
  assert.equal(app.nodes.get("selected-details").hidden, true);
  app.nodes.get("primary-action").click();
  assert.equal(search.value, "");
  assert.equal(app.nodes.get("library-content").querySelectorAll(".game-card").length, 2);
});

test("sort changes card order and controller browses the displayed order", () => {
  const app = launcherHarness([
    { id: "game-1", title: "Alpha", source_kind: "single_html" },
    { id: "game-2", title: "Zulu", source_kind: "zip" },
  ]);
  const sort = app.nodes.get("library-sort");
  sort.value = "title-desc";
  sort.dispatch("change");
  const cards = app.nodes.get("library-content").querySelectorAll(".game-card");
  assert.equal(cards[0].dataset.id, "game-2");
  assert.equal(cards[1].dataset.id, "game-1");
  app.pad.buttons[15].pressed = true;
  app.frame(0);
  assert.equal(app.nodes.get("hero-title").children[0].textContent, "Zulu");
});

test("duplicate import reports the existing game and clears a hiding search", () => {
  const app = launcherHarness([{ id: "game-1", title: "Puzzle", source_kind: "zip" }]);
  const search = app.nodes.get("library-search");
  search.value = "other";
  search.dispatch("input");
  app.nodes.get("add-button").click();
  assert.equal(app.messages.at(-1).type, "pick_and_import");
  app.receive({ type: "imported", already_present: true, game: { id: "game-1", title: "Puzzle" } });
  app.receive({ type: "library", games: [{ id: "game-1", title: "Puzzle", source_kind: "zip" }] });
  assert.equal(search.value, "");
  assert.match(app.nodes.get("toast").textContent, /already in your library/);
  assert.equal(app.nodes.get("library-content").querySelectorAll(".game-card").length, 1);
});

test("an import error cannot unlock another launch while a player is running", () => {
  const app = launcherHarness([
    { id: "game-1", title: "First", source_kind: "single_html" },
    { id: "game-2", title: "Second", source_kind: "zip" },
  ]);
  const cards = app.nodes.get("library-content").querySelectorAll(".game-card");
  cards[0].click();
  app.nodes.get("confirm-launch").click();
  assert.equal(app.messages.filter((message) => message.type === "launch_game").length, 1);
  app.receive({ type: "error", operation: "import", message: "Import rejected: unsafe ZIP" });
  cards[1].click();
  assert.equal(app.nodes.get("launch-dialog").hidden, true);
  app.receive({ type: "player_exited", id: "game-1", message: "Player closed. Your library is ready." });
  cards[1].click();
  assert.equal(app.nodes.get("launch-dialog").hidden, false);
});

test("a stale player-exited event cannot unlock the active player", () => {
  const app = launcherHarness([
    { id: "game-1", title: "First", source_kind: "single_html" },
    { id: "game-2", title: "Second", source_kind: "zip" },
  ]);
  const cards = app.nodes.get("library-content").querySelectorAll(".game-card");
  cards[0].click();
  app.nodes.get("confirm-launch").click();
  app.receive({ type: "player_exited", id: "other-game", message: "Player closed. Your library is ready." });
  cards[1].click();
  assert.equal(app.nodes.get("launch-dialog").hidden, true);
});

test("a library refresh restores focus to the selected replacement card", () => {
  const games = [{ id: "game-1", title: "First", source_kind: "single_html" }];
  const app = launcherHarness(games);
  const oldCard = app.nodes.get("library-content").querySelectorAll(".game-card")[0];
  oldCard.focus();
  app.receive({ type: "library", games });
  const newCard = app.nodes.get("library-content").querySelectorAll(".game-card")[0];
  assert.notEqual(newCard, oldCard);
  assert.equal(app.document.activeElement, newCard);
});
