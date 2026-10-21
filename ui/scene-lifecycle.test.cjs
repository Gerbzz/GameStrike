const assert = require("node:assert/strict");
const test = require("node:test");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");

// Exercise the real scene's event subscriptions and pending image callback at the renderer boundary.
function scene() {
  const source = fs.readFileSync(path.join(process.env.GAMESTRIKE_UI || __dirname, "scene.js"), "utf8");
  const from = source.indexOf("  let coverRequest = 0;");
  const to = source.indexOf("\n  applyVariant();\n  resize();\n  fly(", from);
  assert.ok(from > 0 && to > from);
  const window = new EventTarget();
  const document = new EventTarget();
  const canvas = new EventTarget();
  const frontCanvas = new EventTarget();
  const motionQuery = new EventTarget();
  const body = { dataset: { scene: "webgl" } };
  const work = [];
  const images = [];
  const context = vm.createContext({
    window, document, canvas, frontCanvas, motionQuery, body,
    running: true, frozen: false, paused: false, view: "library", bootStart: null, bootHold: null, BOOT: { end: 5.5 },
    performance: { now: () => 6000 },
    resize() { work.push("resize"); body.dataset.scene = "webgl"; },
    fly() { work.push("fly"); }, setRunning() { work.push("running"); },
    buildChrome() { work.push("chrome"); }, poseBoot() {},
    applyEnvironment() { work.push("environment"); }, applyVariant() { work.push("variant"); },
    renderFrame() { work.push("frame"); }, boot: { setTint() {} }, LINEAR: { orange: { glow: [] } }, variantName: () => "orange",
    startStatic() { work.push("fallback"); body.dataset.scene = "static"; },
    composer: { render() { work.push("render"); } }, front: null,
    world: { cradle: { coverMaterial: {} } },
    THREE: { CanvasTexture: class { constructor() { work.push("texture upload"); } }, SRGBColorSpace: "srgb" },
    Image: class { constructor() { images.push(this); } },
  });
  document.createElement = () => ({ getContext: () => ({ drawImage() {} }), toDataURL() {} });
  const render = source.slice(source.indexOf("  function render() {"), source.indexOf("\n  function renderFrame("));
  const start = "function startWebGL() {";
  const lifetime = source.slice(source.indexOf(start) + start.length, source.indexOf("  let qualityLevel = 0;"));
  vm.runInContext(`${lifetime}\n${render}\n${source.slice(from, to)}\nthis.setCover = setCover; this.draw = render;`, context);
  return { context, window, document, canvas, frontCanvas, motionQuery, body, work, images };
}

test("a lost main context stays on the static fallback after later window and scene events", () => {
  const p = scene();
  p.window.dispatchEvent(new Event("resize"));
  assert.deepEqual(p.work, ["resize"]);
  const loss = new Event("webglcontextlost", { cancelable: true });
  p.canvas.dispatchEvent(loss);
  assert.equal(loss.defaultPrevented, true);
  assert.equal(p.body.dataset.scene, "static");
  const settled = [...p.work];
  for (const type of ["resize", "gamestrike:variant", "gamestrike:view", "gamestrike:pause", "gamestrike:layout"]) p.window.dispatchEvent(new Event(type));
  p.document.dispatchEvent(new Event("visibilitychange"));
  p.motionQuery.dispatchEvent(new Event("change"));
  assert.deepEqual(p.work, settled);
  assert.equal(p.body.dataset.scene, "static");
});

test("a lost foreground context also falls back instead of silently losing tubes and the startup emblem", () => {
  const p = scene();
  const loss = new Event("webglcontextlost", { cancelable: true });
  p.frontCanvas.dispatchEvent(loss);
  assert.equal(loss.defaultPrevented, true);
  assert.equal(p.body.dataset.scene, "static");
  assert.deepEqual(p.work, ["fallback"]);
});

test("an in-flight cover image does not upload or render through the abandoned WebGL scene", () => {
  const p = scene();
  p.context.setCover({ url: "/cover/test.png" });
  assert.equal(p.images.length, 1);
  p.canvas.dispatchEvent(new Event("webglcontextlost", { cancelable: true }));
  const settled = [...p.work];
  p.images[0].onload();
  p.context.draw();
  assert.deepEqual(p.work, settled);
  assert.equal(p.body.dataset.cradle, undefined);
});

test("a cover image still uploads and renders while its WebGL scene is active", () => {
  const p = scene();
  p.context.running = false;
  p.context.setCover({ url: "/cover/test.png" });
  p.images[0].onload();
  p.context.draw();
  assert.deepEqual(p.work, ["texture upload", "frame", "render"]);
  assert.equal(p.body.dataset.cradle, "image");
});
