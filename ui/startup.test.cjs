const assert = require("node:assert/strict");
const test = require("node:test");
const fs = require("node:fs");
const vm = require("node:vm");
const core = require("./core.js");

// Run the page's real startup/capture controller with only the browser/IPC boundary replaced.
// WebGL pixels and CSS interpolation are also checked in the native capture artifacts.
function page() {
  const source = fs.readFileSync(`${__dirname}/app.js`, "utf8");
  const section = (from, to) => source.slice(source.indexOf(from), source.indexOf(to));
  const classes = new Set();
  const node = () => ({
    style: { setProperty() {}, removeProperty() {} },
    classList: { add() {}, remove() {} },
    children: [], hidden: false,
  });
  const animations = [
    { animationName: "gs-in-left", currentTime: 100, pause() { this.paused = true; } },
    { transitionProperty: "opacity", currentTime: 600, pause() { this.paused = true; } },
  ];
  const timers = [];
  const body = {
    dataset: { view: "boot", scene: "webgl", bootLogo: "ready" },
    classList: { add: (...names) => names.forEach((name) => classes.add(name)), remove: (...names) => names.forEach((name) => classes.delete(name)), contains: (name) => classes.has(name) },
    offsetWidth: 1250,
  };
  const state = { loaded: false, booted: false, metrics: { columns: 6 }, games: [], visible: [], detailId: null };
  const el = { boot: node(), grid: node(), search: node(), frame: node() };
  el.frame.stage = { opacity: "1", visibility: "visible", display: "block" };
  el.frame.parentElement = el.frame.stage;
  const window = { WebGL2RenderingContext: function () {}, addEventListener() {}, dispatchEvent() {} };
  const context = vm.createContext({
    window, document: { body, documentElement: node(), querySelectorAll: () => [], getElementById: () => node(), getAnimations: () => animations },
    state, el, core, CINEMA: { assemble: 4200, end: 5500 }, BOOT_MS: 3200,
    performance: { now: () => 2600 }, sessionStorage: { getItem() {}, removeItem() {} },
    sound: { stopBoot() {}, bootSounds() {} }, CustomEvent: class { constructor(type, options) { this.type = type; this.detail = options.detail; } },
    setTimeout: (fn) => { timers.push(fn); return timers.length; }, clearTimeout() {},
    getComputedStyle: (element) => element,
    reduceMotion: () => false, render() {}, drawFrame() {}, applyStyle() {}, layoutFocus() {}, paintCovers() {},
  });
  vm.runInContext([
    section("  function setView(", "  function pauseScene("),
    section("  let bootTimer =", "  // ---------- helpers ----------"),
    section("  function capture(view)", "  // Used only by --capture-view: true"),
    section("  let readySince =", "  window.GameStrike = Object.freeze"),
    "this.start = startBoot; this.capture = capture; this.drawn = drawn;",
  ].join("\n"), context);
  return { context, window, body, state, el, timers, animations };
}

test("a library capture before 5.5 s ends the 3D boot instead of capturing its central emblem", () => {
  const p = page();
  p.context.start();
  assert.equal(p.window.GameStrikeScenePending.boot.action, "play");
  p.context.capture("library");
  assert.equal(p.window.GameStrikeScenePending.boot.action, "skip");
  assert.equal(p.body.dataset.view, "library");
  assert.equal(p.body.dataset.boot, undefined);
});

test("holding a boot frame does not rewind the stage's opacity transition to its invisible start", () => {
  const p = page();
  p.context.capture("boot-4.5");
  p.timers.forEach((fn) => fn());
  assert.equal(p.animations[1].currentTime, 600);
  assert.equal(p.animations[1].paused, undefined);
  assert.equal(p.animations[0].currentTime, 0);
});

test("a completed boot capture is not ready while the DOM stage is invisible", () => {
  const p = page();
  p.state.booted = true;
  p.el.frame.stage.opacity = "0";
  assert.equal(p.context.drawn(), false);
  p.el.frame.stage.opacity = "1";
  assert.equal(p.context.drawn(), true);
});

test("the native capture readiness probe fails closed on a missing or throwing page controller", () => {
  const rust = fs.readFileSync(`${__dirname}/../src/launcher.rs`, "utf8");
  const expression = JSON.parse(rust.match(/const CAPTURE_READY: &str = (".*");/)[1]);
  for (const [controller, expected] of [
    [undefined, false],
    [{ ready() { throw new Error("not drawn"); } }, false],
    [{ ready: () => false }, false],
    [{ ready: () => true }, true],
    [{ receive() {}, capture() {} }, true], // classic page has no scene readiness method
  ]) {
    assert.equal(vm.runInNewContext(expression, { window: { GameStrike: controller } }), expected);
  }
});

test("an unavailable WebGL context restores the visible CSS startup", () => {
  const source = fs.readFileSync(`${__dirname}/scene.js`, "utf8");
  const fallback = source.slice(source.indexOf("function startStatic(reason)"), source.indexOf("\nif (leaving || !canvas)"));
  const body = { dataset: { boot: "cinema", chrome: "3d" }, classList: { add() {} } };
  const context = vm.createContext({
    body, core: null, setMode() {},
    document: { createElement: () => ({ setAttribute() {}, getContext: () => null }), getElementById: () => null },
  });
  vm.runInContext(`${fallback}\nstartStatic('webgl2 unavailable');`, context);
  assert.equal(body.dataset.boot, undefined);
  assert.equal(body.dataset.chrome, undefined);
});
