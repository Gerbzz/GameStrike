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
    const BUTTONS = { a: 0, b: 1, x: 2, y: 3 };

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

  const KIND_LABELS = { single_html: "HTML files", zip: "ZIP archives" };

  // Sidebar filters, built only from what the library really records: everything, recently
  // played, favorites, how each game was imported, and the sources the player wrote down.
  function buildFilters(games) {
    const list = Array.isArray(games) ? games : [];
    const filters = [
      { key: "all", label: "All games", icon: "all", count: list.length },
      { key: "recent", label: "Recently played", icon: "recent", count: list.filter((game) => game.last_played).length },
      { key: "favorites", label: "Favorites", icon: "star", count: list.filter((game) => game.favorite).length },
    ];
    for (const [kind, label] of Object.entries(KIND_LABELS)) {
      const count = list.filter((game) => game.source_kind === kind).length;
      if (count) filters.push({ key: `kind:${kind}`, label, icon: kind === "zip" ? "zip" : "file", count, group: "Imported as" });
    }
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
    return filters;
  }

  function matchesFilter(game, key) {
    if (!key || key === "all") return true;
    if (key === "recent") return Boolean(game.last_played);
    if (key === "favorites") return Boolean(game.favorite);
    if (key.startsWith("kind:")) return game.source_kind === key.slice(5);
    if (key.startsWith("source:")) return (cleanSource(game.source) || "").toLocaleLowerCase() === key.slice(7);
    return false;
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

  const api = Object.freeze({
    filterAndSort, gridMove, shelfMove, tileLayout, createPadReader, cover, formatBytes, formatWhen, cleanTitle,
    cleanSource, buildFilters, matchesFilter, applyFilter, gridColumns, listMove, coverUrl,
  });
  if (typeof module === "object" && module.exports) module.exports = api;
  else root.GameStrikeCore = api;
})(typeof window === "object" ? window : globalThis);
