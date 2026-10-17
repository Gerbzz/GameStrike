const assert = require("node:assert/strict");
const test = require("node:test");
const core = require("./core.js");

const pad = (pressed = [], axes = [0, 0]) => ({
  buttons: Array.from({ length: 17 }, (_, index) => ({ pressed: pressed.includes(index) })),
  axes,
});

test("search filters by title and sorts by recent play, then added date", () => {
  const games = [
    { id: "a", title: "Asteroid Run", last_played: 10, added_at: 1 },
    { id: "b", title: "Puzzle Box", last_played: 50, added_at: 2 },
    { id: "c", title: "Zen Garden", added_at: 9 },
    { id: "d", title: "puzzle quest", added_at: 3 },
  ];
  assert.deepEqual(core.filterAndSort(games, "", "recent").map((g) => g.id), ["b", "a", "c", "d"]);
  assert.deepEqual(core.filterAndSort(games, "  PUZZLE ", "title-asc").map((g) => g.id), ["b", "d"]);
  assert.deepEqual(core.filterAndSort(games, "", "title-desc").map((g) => g.id), ["c", "d", "b", "a"]);
  assert.deepEqual(core.filterAndSort(games, "", "added").map((g) => g.id), ["c", "d", "b", "a"]);
  assert.deepEqual(core.filterAndSort(games, "nothing", "recent"), []);
  assert.equal(games[0].id, "a", "sorting never mutates the library");
});

test("titles sort naturally so Level 2 comes before Level 10", () => {
  const games = [{ id: "1", title: "Level 10" }, { id: "2", title: "Level 2" }];
  assert.deepEqual(core.filterAndSort(games, "", "title-asc").map((g) => g.title), ["Level 2", "Level 10"]);
});

test("grid movement stays inside the grid and handles a short last row", () => {
  // 7 cards in 3 columns:  0 1 2 / 3 4 5 / 6
  assert.equal(core.gridMove(0, 7, 3, "left"), 0);
  assert.equal(core.gridMove(2, 7, 3, "right"), 3);
  assert.equal(core.gridMove(6, 7, 3, "right"), 6);
  assert.equal(core.gridMove(1, 7, 3, "down"), 4);
  assert.equal(core.gridMove(4, 7, 3, "down"), 6, "lands on the last card of a short row");
  assert.equal(core.gridMove(6, 7, 3, "down"), 6);
  assert.equal(core.gridMove(4, 7, 3, "up"), 1);
  assert.equal(core.gridMove(1, 7, 3, "up"), 1);
  assert.equal(core.gridMove(-1, 7, 3, "right"), 1, "an unknown selection starts at the first card");
  assert.equal(core.gridMove(3, 7, 3, "home"), 0);
  assert.equal(core.gridMove(3, 7, 3, "end"), 6);
  assert.equal(core.gridMove(0, 0, 3, "down"), -1);
});

test("a held controller button fires once and needs a fresh press", () => {
  const read = core.createPadReader();
  assert.deepEqual(read(pad([0]), 0), ["a"]);
  assert.deepEqual(read(pad([0]), 16), []);
  assert.deepEqual(read(pad([0]), 2000), [], "holding A never repeats, so it cannot confirm a later dialog");
  assert.deepEqual(read(pad([]), 2016), []);
  assert.deepEqual(read(pad([0]), 2032), ["a"]);
});

test("a button pressed before the pad disconnects does not fire again on reconnect while held", () => {
  const read = core.createPadReader();
  read(pad([1]), 0);
  assert.deepEqual(read(null, 16), []);
  assert.deepEqual(read(pad([1]), 32), ["b"], "a reconnect is a new press");
  assert.deepEqual(read(pad([1]), 48), []);
});

test("held directions repeat after a delay, D-pad and stick map the same", () => {
  const read = core.createPadReader({ initialDelay: 300, repeatDelay: 100 });
  assert.deepEqual(read(pad([15]), 0), ["right"]);
  assert.deepEqual(read(pad([15]), 200), []);
  assert.deepEqual(read(pad([15]), 300), ["right"]);
  assert.deepEqual(read(pad([15]), 350), []);
  assert.deepEqual(read(pad([15]), 400), ["right"]);
  assert.deepEqual(read(pad([], [0, 0.9]), 410), ["down"], "a new direction fires at once");
  assert.deepEqual(read(pad([], [0.3, 0.3]), 420), [], "small stick drift is ignored");
  assert.deepEqual(read(pad([12]), 430), ["up"]);
});

test("A and a direction in one frame report A first, so Go back is activated before focus moves", () => {
  const read = core.createPadReader();
  assert.deepEqual(read(pad([0, 15]), 0), ["a", "right"]);
});

test("generated covers are stable per game, decorative, and use readable initials", () => {
  const one = core.cover("6c1f6f7e-0000-4000-8000-000000000001", "Orbit Garden");
  assert.deepEqual(one, core.cover("6c1f6f7e-0000-4000-8000-000000000001", "Orbit Garden"));
  assert.equal(one.initials, "OG");
  assert.ok(one.hue >= 0 && one.hue < 360 && one.pattern >= 0 && one.pattern < 4);
  assert.notDeepEqual(one, core.cover("6c1f6f7e-0000-4000-8000-000000000002", "Orbit Garden"));
  assert.equal(core.cover("x", "tetris").initials, "TE");
  assert.equal(core.cover("x", "  ").initials, "?");
  assert.equal(core.cover("x", "<script> alert").initials, "SA");
});

test("sizes, times, and titles are formatted for people", () => {
  assert.equal(core.formatBytes(0), "None yet");
  assert.equal(core.formatBytes(512), "512 B");
  assert.equal(core.formatBytes(1536), "1.5 KB");
  assert.equal(core.formatBytes(25 * 1024 * 1024), "25 MB");
  const now = 1_800_000_000_000;
  assert.equal(core.formatWhen(null, now), "Never");
  assert.equal(core.formatWhen(now / 1000 - 20, now), "Just now");
  assert.equal(core.formatWhen(now / 1000 - 600, now), "10 min ago");
  assert.equal(core.formatWhen(now / 1000 - 86400 * 1.5, now), "Yesterday");
  assert.equal(core.formatWhen(now / 1000 - 86400 * 4, now), "4 days ago");
  assert.equal(core.cleanTitle("  Star\u0007field "), "Starfield");
  assert.equal(core.cleanTitle("   "), null);
  assert.equal(core.cleanTitle("x".repeat(81)), null);
  assert.equal(core.cleanTitle("x".repeat(80)), "x".repeat(80));
});

test("shelves move like a console dashboard and skip empty rows", () => {
  const rows = [3, 0, 6];
  assert.deepEqual(core.shelfMove(rows, { row: 0, col: 2 }, "right"), { row: 0, col: 2 }, "stops at the end of a shelf");
  assert.deepEqual(core.shelfMove(rows, { row: 0, col: 2 }, "down"), { row: 2, col: 2 }, "skips the empty shelf");
  assert.deepEqual(core.shelfMove(rows, { row: 2, col: 5 }, "up"), { row: 0, col: 2 }, "clamps to the shorter shelf");
  assert.deepEqual(core.shelfMove(rows, { row: 0, col: 1 }, "up"), { row: 0, col: 1 });
  assert.deepEqual(core.shelfMove(rows, { row: 2, col: 3 }, "home"), { row: 2, col: 0 });
  assert.deepEqual(core.shelfMove(rows, { row: 2, col: 0 }, "end"), { row: 2, col: 5 });
  assert.deepEqual(core.shelfMove([0, 4], { row: 0, col: 9 }, "left"), { row: 1, col: 2 }, "an invalid position recovers");
  assert.equal(core.shelfMove([0, 0], { row: 0, col: 0 }, "down"), null);
});
