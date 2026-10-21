// Pure launcher logic shared by app.js and the Node tests. No DOM access here.
(function (root) {
  "use strict";

  const SORTS = {
    recent: (a, b) => (b.last_played || 0) - (a.last_played || 0) || (b.added_at || 0) - (a.added_at || 0),
    "title-asc": () => 0,
    "title-desc": () => 0,
    added: (a, b) => (b.added_at || 0) - (a.added_at || 0),
  };

  function byTitle(a, b) {
    return a.title.localeCompare(b.title, undefined, { sensitivity: "base", numeric: true }) || a.id.localeCompare(b.id);
  }

  function filterAndSort(games, query, sort) {
    const needle = String(query || "").trim().toLocaleLowerCase();
    const list = games.filter((game) => !needle || game.title.toLocaleLowerCase().includes(needle));
    const primary = SORTS[sort] || SORTS.recent;
    list.sort((a, b) => {
      if (sort === "title-desc") return -byTitle(a, b);
      return primary(a, b) || byTitle(a, b);
    });
    return list;
  }

  // Moves through a wrapped grid of `count` items laid out in `columns` columns.
  function gridMove(index, count, columns, direction) {
    if (count <= 0) return -1;
    const cols = Math.max(1, columns | 0);
    const current = Math.min(Math.max(index, 0), count - 1);
    switch (direction) {
      case "left": return current > 0 ? current - 1 : current;
      case "right": return current < count - 1 ? current + 1 : current;
      case "up": return current - cols >= 0 ? current - cols : current;
      case "down": {
        if (current + cols < count) return current + cols;
        // Moving down from a row above a short last row lands on its final card.
        const lastRowStart = Math.floor((count - 1) / cols) * cols;
        return current < lastRowStart ? count - 1 : current;
      }
      case "home": return 0;
      case "end": return count - 1;
      default: return current;
    }
  }

  // Console-style shelves: left/right within a row, up/down between rows keeping the column
  // where possible. `rows` is an array of row lengths; `pos` is { row, col }.
  function shelfMove(rows, pos, direction) {
    const nonEmpty = rows.map((length, row) => ({ length, row })).filter((entry) => entry.length > 0);
    if (!nonEmpty.length) return null;
    let row = Math.min(Math.max(pos?.row ?? nonEmpty[0].row, 0), rows.length - 1);
    if (!rows[row]) row = nonEmpty[0].row;
    let col = Math.min(Math.max(pos?.col ?? 0, 0), rows[row] - 1);
    if (direction === "left") col = Math.max(0, col - 1);
    else if (direction === "right") col = Math.min(rows[row] - 1, col + 1);
    else if (direction === "home") col = 0;
    else if (direction === "end") col = rows[row] - 1;
    else if (direction === "up" || direction === "down") {
      const step = direction === "up" ? -1 : 1;
      let next = row + step;
      while (next >= 0 && next < rows.length && !rows[next]) next += step;
      if (next >= 0 && next < rows.length) { row = next; col = Math.min(col, rows[row] - 1); }
    }
    return { row, col };
  }

  // Perspective row measured from the classic console dashboard: the focused tile is full size on
  // the left and later tiles sit edge to edge, shrinking faster in width than height so they read as
  // receding into the distance. Earlier tiles slide out to the left. Units are tile widths/heights.
  const WIDTHS = [1, 0.74, 0.44, 0.29, 0.2, 0.15];
  const HEIGHTS = [1, 0.74, 0.58, 0.48, 0.42, 0.38];
  const RISE = [0, -0.024, -0.035, -0.047, -0.05, -0.052];
  function tileLayout(offset) {
    if (offset < 0) return { x: -0.4 * Math.min(-offset, 2), y: 0, sx: 0.9, sy: 0.9, opacity: 0, z: 0, visible: false };
    if (offset >= WIDTHS.length) return { x: 2.9, y: -0.055, sx: 0.1, sy: 0.34, opacity: 0, z: 0, visible: false };
    let x = 0;
    for (let k = 0; k < offset; k += 1) x += WIDTHS[k];
    return { x: Number(x.toFixed(4)), y: RISE[offset], sx: WIDTHS[offset], sy: HEIGHTS[offset], opacity: 1, z: 100 - offset, visible: true };
  }

  // Standard-mapping gamepad edges. Buttons fire once per press; directions repeat while held.
  function createPadReader({ initialDelay = 380, repeatDelay = 140, threshold = 0.55 } = {}) {
    let previous = {};
    let heldDirection = "";
    let nextRepeatAt = 0;
    // A, B, X, Y, and the menu (Start) button, which opens a game's details.
    const BUTTONS = { a: 0, b: 1, x: 2, y: 3, menu: 9 };

    return function read(pad, now) {
      if (!pad) {
        previous = {};
        heldDirection = "";
        return [];
      }
      const pressed = (index) => Boolean(pad.buttons?.[index]?.pressed);
      const axes = pad.axes || [];
      const actions = [];
      const current = {};
      for (const [name, index] of Object.entries(BUTTONS)) {
        current[name] = pressed(index);
        if (current[name] && !previous[name]) actions.push(name);
      }
      const direction =
        pressed(12) || axes[1] < -threshold ? "up"
        : pressed(13) || axes[1] > threshold ? "down"
        : pressed(14) || axes[0] < -threshold ? "left"
        : pressed(15) || axes[0] > threshold ? "right"
        : "";
      if (direction && direction !== heldDirection) {
        actions.push(direction);
        nextRepeatAt = now + initialDelay;
      } else if (direction && now >= nextRepeatAt) {
        actions.push(direction);
        nextRepeatAt = now + repeatDelay;
      }
      heldDirection = direction;
      previous = current;
      return actions;
    };
  }

  // Deterministic decorative cover: two hues and initials derived from the game ID and title.
  // It is clearly generated artwork, never presented as the game's own cover.
  function cover(id, title) {
    let hash = 2166136261;
    for (const char of String(id)) hash = Math.imul(hash ^ char.charCodeAt(0), 16777619) >>> 0;
    const hue = hash % 360;
    const shift = 28 + ((hash >>> 9) % 70);
    const angle = 100 + ((hash >>> 17) % 120);
    const words = String(title).replace(/[^\p{L}\p{N}\s]/gu, " ").trim().split(/\s+/).filter(Boolean);
    const initials = (words.length > 1 ? words[0][0] + words[1][0] : (words[0] || "?").slice(0, 2)).toLocaleUpperCase();
    return { hue, hue2: (hue + shift) % 360, angle, pattern: hash % 4, initials };
  }

  function formatBytes(bytes) {
    if (!bytes) return "None yet";
    const units = ["B", "KB", "MB", "GB"];
    let value = bytes;
    let unit = 0;
    while (value >= 1024 && unit < units.length - 1) { value /= 1024; unit += 1; }
    return `${value >= 10 || unit === 0 ? Math.round(value) : value.toFixed(1)} ${units[unit]}`;
  }

  function formatWhen(seconds, nowMs = Date.now()) {
    if (!seconds) return "Never";
    const diff = Math.max(0, Math.round(nowMs / 1000 - seconds));
    if (diff < 60) return "Just now";
    if (diff < 3600) return `${Math.floor(diff / 60)} min ago`;
    if (diff < 86400) return `${Math.floor(diff / 3600)} h ago`;
    const days = Math.floor(diff / 86400);
    if (days === 1) return "Yesterday";
    if (days < 30) return `${days} days ago`;
    return new Date(seconds * 1000).toLocaleDateString(undefined, { year: "numeric", month: "short", day: "numeric" });
  }

  function cleanTitle(raw) {
    const title = String(raw || "").replace(/[\u0000-\u001f\u007f]/g, "").trim();
    return title.length >= 1 && [...title].length <= 80 ? title : null;
  }

  // Same rule as the Rust library: no control characters, 1 to 32 characters after trimming.
  function cleanSource(raw) {
    const source = String(raw || "").replace(/[\u0000-\u001f\u007f]/g, "").trim();
    return source.length >= 1 && [...source].length <= 32 ? source : null;
  }

  // Stores in sidebar order, with the short names the sidebar shows. GameStrike draws its own
  // icons for them; no store logos are used.
  const STORES = Object.freeze([
    ["steam", "Steam", "Steam"],
    ["epic", "Epic", "Epic Games"],
    ["ea", "EA", "EA app"],
    ["xbox", "MS Store", "Microsoft Store"],
    ["gog", "GOG", "GOG Galaxy"],
    ["ubisoft", "Ubisoft", "Ubisoft Connect"],
    ["battlenet", "Battle.net", "Battle.net"],
    ["custom", "Custom", "Custom programs"],
  ]);
  const storeName = (key) => STORES.find(([k]) => k === key)?.[2] || "HTML game";
  const storeLabel = (key) => STORES.find(([k]) => k === key)?.[1] || "HTML games";

  // One list for the launcher: HTML games from the library scan, and store games and custom
  // programs from the store catalog, with the same fields.
  function mergeLibrary(htmlGames, storeGames) {
    const html = (Array.isArray(htmlGames) ? htmlGames : []).map((game) => ({ ...game, kind: "html", store: "html", installed: game.files_ok !== false, hidden: false }));
    const stores = (Array.isArray(storeGames) ? storeGames : []).map((game) => ({
      ...game,
      kind: "store",
      source_kind: "store",
      source: null,
      files_ok: Boolean(game.installed),
      save_bytes: 0,
    }));
    return [...html, ...stores];
  }

  // Sidebar filters, built only from what the library really records: everything, recently
  // played, favorites, each store that has games, HTML games, the sources the player wrote
  // down for HTML games, and hidden games. Hidden games appear only under Hidden.
  function buildFilters(games) {
    const all = Array.isArray(games) ? games : [];
    const list = all.filter((game) => !game.hidden);
    const filters = [
      { key: "all", label: "All games", icon: "all", count: list.length },
      { key: "recent", label: "Recently played", icon: "recent", count: list.filter((game) => game.last_played).length },
      { key: "favorites", label: "Favorites", icon: "star", count: list.filter((game) => game.favorite).length },
    ];
    for (const [key, label] of STORES) {
      const count = list.filter((game) => game.kind === "store" && game.store === key).length;
      if (count) filters.push({ key: `store:${key}`, label, icon: `store-${key}`, count, group: "Stores" });
    }
    const html = list.filter((game) => game.kind !== "store").length;
    if (html) filters.push({ key: "html", label: "HTML games", icon: "html", count: html, group: "Stores" });
    const sources = new Map();
    for (const game of list) {
      const source = cleanSource(game.source);
      if (!source) continue;
      const key = source.toLocaleLowerCase();
      const entry = sources.get(key) || { label: source, count: 0 };
      entry.count += 1;
      sources.set(key, entry);
    }
    [...sources.entries()]
      .sort((a, b) => a[1].label.localeCompare(b[1].label, undefined, { sensitivity: "base", numeric: true }))
      .forEach(([key, entry]) => filters.push({ key: `source:${key}`, label: entry.label, icon: "tag", count: entry.count, group: "Sources" }));
    const hidden = all.length - list.length;
    if (hidden) filters.push({ key: "hidden", label: "Hidden", icon: "hidden", count: hidden });
    return filters;
  }

  function matchesFilter(game, key) {
    if (key === "hidden") return Boolean(game.hidden);
    if (game.hidden) return false;
    if (!key || key === "all") return true;
    if (key === "recent") return Boolean(game.last_played);
    if (key === "favorites") return Boolean(game.favorite);
    if (key === "html") return game.kind !== "store";
    if (key.startsWith("store:")) return game.kind === "store" && game.store === key.slice(6);
    if (key.startsWith("kind:")) return game.source_kind === key.slice(5);
    if (key.startsWith("source:")) return (cleanSource(game.source) || "").toLocaleLowerCase() === key.slice(7);
    return false;
  }

  // What the bottom bar says about a game: whether it can start, and where it comes from.
  function gameStatus(game, running) {
    if (!game) return "";
    const where = game.kind === "store" ? storeName(game.store) : "HTML game";
    if (game.kind === "store") return `${game.installed ? "Ready to play" : "Not installed"} · ${where}`;
    if (game.files_ok === false) return `Files missing · ${where}`;
    if (game.id === running) return "Playing now";
    if (running) return "Another game is open";
    return `Ready to play · ${where}`;
  }

  // The games shown in the grid: filter, then search, then sort. Recently played always shows
  // the latest first.
  function applyFilter(games, key, query, sort) {
    const filtered = (Array.isArray(games) ? games : []).filter((game) => matchesFilter(game, key));
    return filterAndSort(filtered, query, key === "recent" ? "recent" : sort);
  }

  // Columns that fit `width` with tiles at least `minTile` wide and `gap` apart.
  function gridColumns(width, minTile, gap) {
    if (!(width > 0) || !(minTile > 0)) return 1;
    return Math.max(1, Math.floor((width + gap) / (minTile + gap)));
  }

  // Sidebar rows: up and down move, and stop at the ends.
  function listMove(index, count, direction) {
    if (count <= 0) return -1;
    const current = Math.min(Math.max(index, 0), count - 1);
    if (direction === "up") return Math.max(0, current - 1);
    if (direction === "down") return Math.min(count - 1, current + 1);
    if (direction === "home") return 0;
    if (direction === "end") return count - 1;
    return current;
  }

  // The stored cover's URL, or null when the launcher draws a cover from the title.
  // Only a validated UUID reaches the URL; the version makes a changed cover load again.
  function coverUrl(game) {
    if (!game || !game.cover || !/^[0-9a-f-]{36}$/i.test(String(game.id))) return null;
    return `/cover/${game.id}.png?v=${Number(game.cover_version) || 0}`;
  }

  // Curated box-art palettes: a deep two-stop background, a vivid accent, and the title ink.
  // Every pair keeps strong contrast, so no generated cover comes out muddy.
  const PALETTES = Object.freeze([
    { name: "lime", bg1: "#0d2a3f", bg2: "#03080f", accent: "#8dff4a", ink: "#f0ffe6" },
    { name: "cyan", bg1: "#06345c", bg2: "#020914", accent: "#3ee8ff", ink: "#e9fcff" },
    { name: "magenta", bg1: "#55104f", bg2: "#140312", accent: "#ff4fd8", ink: "#fff0fc" },
    { name: "sunset", bg1: "#6a1f08", bg2: "#180503", accent: "#ff8a2b", ink: "#fff4ea" },
    { name: "gold", bg1: "#2f2a17", bg2: "#0b0a06", accent: "#ffcc33", ink: "#fff9e3" },
    { name: "violet", bg1: "#311370", bg2: "#09041d", accent: "#b38cff", ink: "#f4eeff" },
    { name: "crimson", bg1: "#5c0b18", bg2: "#120205", accent: "#ff3d5a", ink: "#ffecef" },
    { name: "teal", bg1: "#074640", bg2: "#010f0e", accent: "#29f0c8", ink: "#e8fffb" },
    { name: "ice", bg1: "#213650", bg2: "#070d14", accent: "#9fd6ff", ink: "#f0f8ff" },
    { name: "acid", bg1: "#1d470e", bg2: "#050d02", accent: "#d4ff3a", ink: "#f9ffe4" },
  ]);
  // Geometric motifs drawn in CSS: rings, sunburst, horizon, chevrons, halftone, slabs.
  const MOTIFS = 6;

  // A designed cover for a game with no art: palette and motif from the game ID, the full title
  // as the headline (sized by length), and a small kicker such as the source.
  function coverDesign(id, title, kicker) {
    let hash = 2166136261;
    for (const char of String(id)) hash = Math.imul(hash ^ char.charCodeAt(0), 16777619) >>> 0;
    const text = String(title || "").replace(/[\u0000-\u001f\u007f]/g, "").trim() || "Untitled";
    const length = [...text].length;
    const size = length <= 10 ? "xl" : length <= 18 ? "l" : length <= 30 ? "m" : "s";
    return {
      palette: PALETTES[hash % PALETTES.length],
      motif: (hash >>> 8) % MOTIFS,
      title: text,
      size,
      kicker: String(kicker || "").trim().slice(0, 24),
    };
  }


  // ---------- Collection style ----------

  // Dashboard styles. The Collection styles are drawn by index.html; the earlier ones by classic.html.
  const STYLES = Object.freeze([
    { key: "collection-orange", label: "Collection: PWDS Orange", page: "index", variant: "orange" },
    { key: "collection-green", label: "Collection: Green", page: "index", variant: "green" },
    { key: "blades", label: "Blades", page: "classic" },
    { key: "original", label: "Original", page: "classic" },
    { key: "pwds", label: "PWDS Orange", page: "classic" },
  ]);
  // The saved style, or the default when nothing (or something unknown) is saved.
  function styleFor(saved) {
    return STYLES.find((style) => style.key === saved) || STYLES[0];
  }
  function nextStyle(saved) {
    const at = STYLES.indexOf(styleFor(saved));
    return STYLES[(at + 1) % STYLES.length];
  }

  // Scale for the frame, measured from the reference concept at 1672 x 941: layout scale u, a
  // type scale t that stays readable in small windows, and the number of cover columns. Columns
  // are chosen so that three rows of covers fill the grid's height, as in the reference (seven
  // columns by three rows at 1672 x 941); taller windows get fewer, larger columns and scroll.
  function collectionMetrics(width, height) {
    const w = Math.max(1, Number(width) || 0);
    const h = Math.max(1, Number(height) || 0);
    const raw = Math.min(w / 1672, h / 941);
    const u = Math.max(0.5, Math.min(1.6, raw));
    const t = Math.max(0.72, u);
    // The grid's well: the frame's margins (20 + 22, 6 + 12), the sidebar column (288 incl. gap),
    // the right column (54), the header (108), and the bottom bar (100), all scaled by u.
    const gridWidth = w - u * (20 + 22 + 288 + 54);
    const gridHeight = h - u * (6 + 12 + 108 + 100);
    const gap = 9.5 * u;
    const tileHeight = Math.max(80, (gridHeight - gap * 2 - 28 * u) / 3);
    const tileWidth = (tileHeight * 176) / 236;
    const columns = Math.max(2, Math.min(12, Math.floor((gridWidth + gap) / (tileWidth + gap))));
    return { u: Number(u.toFixed(4)), t: Number(t.toFixed(4)), columns };
  }

  // ---------- generated box art ----------
  // Scenes the cover painter can draw. A word in the title picks a fitting one; otherwise the game
  // ID does. Every cover is drawn by GameStrike from these rules: no third-party art.
  const SCENES = Object.freeze(["peaks", "space", "neon", "forest", "reef", "emblem", "dunes", "city", "castle", "harbor", "frost", "forge"]);
  const SCENE_WORDS = Object.freeze([
    [/\b(moon|star|stars|starfall|comet|orbit|planet|galaxy|space|rocket|cosmic|astro|nova)\b/i, "space"],
    [/\b(neon|volt|circuit|drift|turbo|cyber|pixel|synth|laser|grid|racer|runner)\b/i, "neon"],
    [/\b(forest|wood|woods|moss|garden|grove|leaf|lantern|firefly|tree)\b/i, "forest"],
    [/\b(reef|deep|ocean|sea|tide|coral|abyss|fish|dive)\b/i, "reef"],
    [/\b(desert|dune|dunes|canyon|sand|mesa|sun|oasis)\b/i, "dunes"],
    [/\b(city|town|street|bakery|inn|tower|signal|metro)\b/i, "city"],
    [/\b(kingdom|castle|knight|king|queen|keep|keepers|tactics|crown|quest)\b/i, "castle"],
    [/\b(harbor|harbour|lighthouse|port|boat|ship|voyage|sail|pilot|pilots|courier)\b/i, "harbor"],
    [/\b(frost|ice|snow|winter|glacier|relay|cold|polar)\b/i, "frost"],
    [/\b(ember|forge|fire|flame|lava|volcano|inferno|burn)\b/i, "forge"],
    [/\b(mountain|peak|peaks|summit|valley|cliff|hollow)\b/i, "peaks"],
    [/\b(monk|temple|maze|rune|relic|crystal|glass|gem)\b/i, "emblem"],
  ]);
  // Each palette: sky top and bottom, far and near layers, a light, the title ink, and its edge.
  const ART_PALETTES = Object.freeze([
    { name: "ember dusk", sky: ["#1b0a2e", "#ff6a3d"], far: "#7a2c4e", near: "#160814", light: "#ffd36b", ink: "#fff2d6", edge: "#2a0610" },
    { name: "teal night", sky: ["#021a2b", "#1f7a8c"], far: "#14506a", near: "#04121c", light: "#9df6ff", ink: "#eaffff", edge: "#01121f" },
    { name: "violet storm", sky: ["#12052b", "#6a2bd8"], far: "#3a1a7a", near: "#0b0420", light: "#e0b3ff", ink: "#f6ecff", edge: "#14052a" },
    { name: "jungle", sky: ["#04220f", "#4fbf5a"], far: "#1e6b34", near: "#03140a", light: "#e9ff8a", ink: "#f2ffe2", edge: "#03170a" },
    { name: "gold hour", sky: ["#2b1302", "#ffb43a"], far: "#a3551a", near: "#1c0b03", light: "#fff1a8", ink: "#fff8e6", edge: "#2a1304" },
    { name: "arctic", sky: ["#0d2340", "#bfe6ff"], far: "#6c9cc7", near: "#0b1d33", light: "#ffffff", ink: "#f4fbff", edge: "#0a1a30" },
    { name: "crimson", sky: ["#1a0206", "#e3223e"], far: "#7a0f22", near: "#120104", light: "#ffd0b0", ink: "#fff0f0", edge: "#220308" },
    { name: "candy", sky: ["#2b0a3d", "#ff5fb8"], far: "#a3307a", near: "#1f0628", light: "#fff07a", ink: "#fff4fb", edge: "#24062e" },
    { name: "ocean", sky: ["#020b2b", "#1f5fff"], far: "#173f9e", near: "#020820", light: "#7af0ff", ink: "#eef6ff", edge: "#030a24" },
    { name: "lime acid", sky: ["#0d1a02", "#b8ff2a"], far: "#4f8a12", near: "#0a1402", light: "#ffffff", ink: "#f8ffe6", edge: "#0b1402" },
  ]);
  const TITLE_STYLES = Object.freeze(["block", "wide", "slant", "stencil"]);
  // Palettes that suit each scene, by name.
  const SCENE_PALETTES = Object.freeze({
    space: ["violet storm", "teal night", "crimson", "ocean", "candy"],
    neon: ["candy", "violet storm", "ocean", "lime acid", "crimson"],
    forest: ["jungle", "teal night", "gold hour", "lime acid"],
    reef: ["ocean", "teal night", "jungle"],
    dunes: ["gold hour", "ember dusk", "crimson"],
    city: ["ember dusk", "violet storm", "candy", "teal night"],
    castle: ["gold hour", "crimson", "violet storm", "ember dusk"],
    harbor: ["ocean", "teal night", "ember dusk", "arctic"],
    frost: ["arctic", "teal night", "ocean"],
    forge: ["ember dusk", "crimson", "gold hour"],
    peaks: ["ember dusk", "violet storm", "arctic", "gold hour"],
  });

  function paletteFor(scene, hash) {
    const names = SCENE_PALETTES[scene];
    if (!names) return ART_PALETTES[(hash >>> 5) % ART_PALETTES.length];
    const name = names[(hash >>> 5) % names.length];
    return ART_PALETTES.find((palette) => palette.name === name);
  }

  // A cover's design, stable per game: scene, palette, title style and placement, and a seed for
  // the painter's own random layout.
  function coverArt(id, title, kicker) {
    let hash = 2166136261;
    for (const char of String(id)) hash = Math.imul(hash ^ char.charCodeAt(0), 16777619) >>> 0;
    const text = String(title || "").replace(/[\u0000-\u001f\u007f]/g, "").trim() || "Untitled";
    const match = SCENE_WORDS.find(([pattern]) => pattern.test(text));
    const scene = match ? match[1] : SCENES[hash % SCENES.length];
    const longest = Math.max(...text.split(/\s+/).map((word) => [...word].length));
    return {
      seed: hash || 1,
      scene,
      palette: paletteFor(scene, hash),
      titleStyle: TITLE_STYLES[(hash >>> 11) % TITLE_STYLES.length],
      titleAt: (hash >>> 15) % 3 === 0 ? "bottom" : "top",
      title: text,
      // Long words need a smaller title so they fit across the cover.
      titleScale: longest > 12 ? 0.62 : longest > 9 ? 0.76 : longest > 6 ? 0.9 : 1,
      kicker: String(kicker || "").trim().slice(0, 24),
    };
  }

  // A small seeded random generator (mulberry32) for the cover painter and the static dome.
  function seededRandom(seed) {
    let a = seed >>> 0 || 1;
    return function next() {
      a = (a + 0x6d2b79f5) >>> 0;
      let t = a;
      t = Math.imul(t ^ (t >>> 15), t | 1);
      t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
      return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
    };
  }

  // ---------- the dome net ----------
  // The net on the inside of the dome: latitude and longitude rings around a pole that points at
  // the light, bent by a smooth warp so the cells are irregular, quad-ish, and slightly wavy, as in
  // the reference. Returns polylines of unit vectors; scene.js draws the same pattern in a shader,
  // and the static fallback projects these lines on a 2D canvas.
  const NET = Object.freeze({ rings: 34, meridians: 68, warp: 0.08, pole: Object.freeze([-0.62, 0.42, -0.66]) });
  function domeNet({ rings = NET.rings, meridians = NET.meridians, steps = 72, warp = NET.warp, pole = NET.pole } = {}) {
    const n = Math.hypot(...pole) || 1;
    const p = pole.map((v) => v / n);
    const helper = Math.abs(p[1]) < 0.9 ? [0, 1, 0] : [1, 0, 0];
    const cross = (a, b) => [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
    const norm = (v) => { const l = Math.hypot(...v) || 1; return v.map((x) => x / l); };
    const e1 = norm(cross(helper, p));
    const e2 = cross(p, e1);
    const bend = (theta, phi) => [
      theta + warp * (Math.sin(phi * 3 + theta * 5) * 0.6 + Math.sin(phi * 7 - theta * 2) * 0.4),
      phi + warp * 1.4 * (Math.sin(theta * 6 + phi * 2) * 0.6 + Math.sin(theta * 3 - phi * 5) * 0.4),
    ];
    const point = (theta, phi) => {
      const [t, f] = bend(theta, phi);
      const s = Math.sin(t);
      return [0, 1, 2].map((k) => p[k] * Math.cos(t) + s * (e1[k] * Math.cos(f) + e2[k] * Math.sin(f)));
    };
    const lines = [];
    for (let i = 1; i < rings; i += 1) {
      const theta = (Math.PI * i) / rings;
      const line = [];
      for (let k = 0; k <= steps; k += 1) line.push(point(theta, (2 * Math.PI * k) / steps));
      lines.push(line);
    }
    for (let j = 0; j < meridians; j += 1) {
      const phi = (2 * Math.PI * j) / meridians;
      const line = [];
      for (let k = 1; k < steps; k += 1) line.push(point((Math.PI * k) / steps, phi));
      lines.push(line);
    }
    return lines;
  }

  const api = Object.freeze({
    filterAndSort, gridMove, shelfMove, tileLayout, createPadReader, cover, formatBytes, formatWhen, cleanTitle,
    cleanSource, buildFilters, matchesFilter, applyFilter, gridColumns, listMove, coverUrl, coverDesign, PALETTES,
    STYLES, styleFor, nextStyle, collectionMetrics, SCENES, coverArt, seededRandom, NET, domeNet,
    STORES, storeName, storeLabel, mergeLibrary, gameStatus,
  });
  if (typeof module === "object" && module.exports) module.exports = api;
  else root.GameStrikeCore = api;
})(typeof window === "object" ? window : globalThis);
