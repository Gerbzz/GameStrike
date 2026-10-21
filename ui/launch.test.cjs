const assert = require("node:assert/strict");
const test = require("node:test");
const fs = require("node:fs");
const vm = require("node:vm");

// Exercise real tile/launch/message handlers; replace only DOM, audio, and the native IPC boundary.
function page(selected, kind = "store") {
  const source = fs.readFileSync(`${__dirname}/app.js`, "utf8");
  const section = (from, to) => {
    const start = source.indexOf(from), end = source.indexOf(to, start);
    assert.ok(start >= 0 && end > start, `missing page section ${from}`);
    return source.slice(start, end);
  };
  const element = () => ({
    dataset: {}, handlers: {}, append() {}, setAttribute() {},
    addEventListener(name, handler) { this.handlers[name] = handler; },
  });
  const messages = [];
  const state = { zone: "grid", gridIndex: selected ? 0 : 1, launching: false, running: null };
  const context = vm.createContext({
    document: { createElement: element }, state,
    isStore: (game) => game.kind === "store", statusFor: () => "Ready", badge: element,
    sound: { play() {} }, layoutFocus() {}, toast() {},
    send: (message) => messages.push(message),
  });
  vm.runInContext([
    section("  function tile(game, index)", "  // Covers are painted"),
    section("  function launch(game", "  function toggleFavorite("),
    section("  function receive(message)", "  // Used only by the launcher's explicit"),
    "this.tile = tile; this.launch = launch; this.receive = receive;",
  ].join("\n"), context);
  const game = { id: "test-game", title: "Synthetic Test", kind, installed: true, files_ok: true };
  const node = context.tile(game, 0);
  return { context, node, game, state, messages };
}

for (const selected of [true, false]) {
  for (const fastReply of [false, true]) {
    test(`${selected ? "selected" : "unselected"} tile double-click launches once${fastReply ? " with an immediate store reply" : ""}`, () => {
      const p = page(selected);
      p.node.handlers.click({ detail: 1 });
      if (fastReply) p.context.receive({ type: "store_started", message: "Started" });
      p.node.handlers.click({ detail: 2 });
      p.node.handlers.dblclick({ detail: 2 });
      assert.equal(p.messages.length, 1);
      assert.equal(p.messages[0].type, "launch_store_game");
    });
  }
}

test("single-click still selects an unselected tile and activates a selected tile", () => {
  const p = page(false);
  p.node.handlers.click({ detail: 1 });
  assert.equal(p.state.gridIndex, 0);
  assert.equal(p.messages.length, 0);
  p.node.handlers.click({ detail: 1 });
  assert.equal(p.messages.length, 1);
});

test("keyboard/controller activation remains immediate for a store game", () => {
  const p = page(true);
  p.context.launch(p.game);
  assert.equal(p.messages.length, 1);
});

test("an HTML tile double-click still launches exactly one isolated player", () => {
  const p = page(false, "html");
  p.node.handlers.click({ detail: 1 });
  p.node.handlers.click({ detail: 2 });
  p.node.handlers.dblclick({ detail: 2 });
  assert.equal(p.messages.length, 1);
  assert.equal(p.messages[0].type, "launch_game");
});
