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

  const api = Object.freeze({ filterAndSort, gridMove, createPadReader, cover, formatBytes, formatWhen, cleanTitle });
  if (typeof module === "object" && module.exports) module.exports = api;
  else root.GameStrikeCore = api;
})(typeof window === "object" ? window : globalThis);
