// The Collection style's frame: one thick, bevelled frame drawn to the exact size of the window,
// around the regions that the stylesheet lays out (emblem, header tab, sidebar, grid, and the bottom
// bar). Corners are hard 45-degree chamfers, never rounded.
//
// measure() turns the layout boxes into the frame's outlines (pixels, relative to the frame node).
// With WebGL, scene.js extrudes those outlines into the 3D chrome; without it, draw() paints them as
// SVG with gradients and glows (the fallback). Colours come from CSS custom properties, so the same
// frame works in PWDS Orange and Green. Pure drawing: it reads untransformed layout offsets, so the
// frame's own 3D drift never distorts it.
(function (root) {
  "use strict";
  const NS = "http://www.w3.org/2000/svg";

  const poly = (points) => `M${points.map(([x, y]) => `${x.toFixed(1)} ${y.toFixed(1)}`).join("L")}Z`;
  const path = (points) => `M${points.map(([x, y]) => `${x.toFixed(1)} ${y.toFixed(1)}`).join("L")}`;

  // A rectangle with 45-degree chamfers. `c` is one size or [topLeft, topRight, bottomRight, bottomLeft].
  function chamferPoints(x, y, w, h, c) {
    const [a, b, d, e] = Array.isArray(c) ? c : [c, c, c, c];
    return [[x + a, y], [x + w - b, y], [x + w, y + b], [x + w, y + h - d], [x + w - d, y + h], [x + e, y + h], [x, y + h - e], [x, y + a]];
  }
  // The same outline moved inward by `inset` (chamfers shrink by the 45-degree offset).
  function insetPoints(x, y, w, h, c, inset) {
    const k = inset * (Math.SQRT2 - 1);
    const cs = (Array.isArray(c) ? c : [c, c, c, c]).map((v) => Math.max(0, v - k));
    return chamferPoints(x + inset, y + inset, w - inset * 2, h - inset * 2, cs);
  }

  function el(name, attrs, parent) {
    const node = document.createElementNS(NS, name);
    for (const [key, value] of Object.entries(attrs)) node.setAttribute(key, String(value));
    if (parent) parent.appendChild(node);
    return node;
  }

  // A node's layout box relative to the frame, ignoring CSS transforms.
  function layoutBox(node, frame) {
    if (!node || node.hidden || !node.offsetWidth) return null;
    let x = 0;
    let y = 0;
    for (let n = node; n && n !== frame; n = n.offsetParent) { x += n.offsetLeft; y += n.offsetTop; }
    return { x, y, w: node.offsetWidth, h: node.offsetHeight };
  }

  // Every outline of the frame, in pixels relative to the frame node, plus where the frame node sits
  // in the window (offsetX, offsetY). Null until the layout has a size.
  function measure(frameNode) {
    if (!frameNode) return null;
    const W = frameNode.offsetWidth;
    const H = frameNode.offsetHeight;
    if (W < 10 || H < 10) return null;
    const u = Number(getComputedStyle(document.documentElement).getPropertyValue("--u")) || 1;
    const q = (selector) => layoutBox(frameNode.querySelector(selector), frameNode);
    const side = q(".sidebar");
    const lib = q(".library");
    const head = q(".head");
    const bottom = q(".bottom");
    const now = q(".now");
    const prompts = q(".prompts");
    const emblem = q(".emblem");
    const group = q(".side-bottom");
    if (!side || !lib || !head || !bottom) return null;
    const m = { W, H, u, offsetX: frameNode.offsetLeft, offsetY: frameNode.offsetTop };

    // The frame body: a thick chamfered ring around the interior.
    const c = 52 * u;
    const top = head.y + head.h * 0.3;
    m.c = c;
    m.top = top;
    m.outer = [[0, top + c * 1.6], [c * 0.62, top + c * 0.9], [c * 0.62, top], [W - c, top], [W, top + c], [W, H - c * 1.4], [W - c * 0.6, H - c * 0.8], [W - c * 0.6, H], [c, H], [0, H - c]];
    const ix0 = side.x - 18 * u;
    const iy0 = lib.y - 12 * u;
    const ix1 = lib.x + lib.w + 14 * u;
    const iy1 = lib.y + lib.h + 12 * u;
    const ic = [18 * u, 30 * u, 26 * u, 18 * u];
    m.interior = { x: ix0, y: iy0, w: ix1 - ix0, h: iy1 - iy0, c: ic };
    m.inner = chamferPoints(ix0, iy0, ix1 - ix0, iy1 - iy0, ic);
    // Glass inlays on the body: the top bar and the sides carry lit, translucent panels.
    m.bar = chamferPoints(head.x + head.w + 40 * u, top + 7 * u, W - c - 26 * u - (head.x + head.w + 40 * u), iy0 - top - 15 * u, [0, 10 * u, 0, 10 * u]);
    m.rightGlass = chamferPoints(ix1 + 9 * u, iy0 + c * 0.5, W - ix1 - 18 * u, (iy1 - iy0) * 0.62, [0, 8 * u, 8 * u, 0]);
    m.leftGlass = chamferPoints(6 * u, top + c * 2.1, ix0 - 12 * u, (iy1 - top) * 0.42, [8 * u, 0, 0, 8 * u]);

    // The sidebar: its own framed panel with bracket ears and rivets on the left edge.
    const s = { x: side.x - 10 * u, y: side.y - 14 * u, w: side.w + 18 * u, h: side.h + 18 * u };
    const sc = [16 * u, 24 * u, 16 * u, 16 * u];
    m.side = { box: s, poly: chamferPoints(s.x, s.y, s.w, s.h, sc), inner: insetPoints(s.x, s.y, s.w, s.h, sc, 5 * u) };
    m.ears = [0.16, 0.6].map((fy) => {
      const ey = s.y + s.h * fy;
      return { poly: [[s.x, ey], [s.x - 13 * u, ey + 9 * u], [s.x - 13 * u, ey + 64 * u], [s.x, ey + 73 * u]], rivets: [[s.x - 5 * u, ey + 22 * u, 3 * u], [s.x - 5 * u, ey + 51 * u, 3 * u]] };
    });
    if (group) {
      const g = { x: group.x - 8 * u, y: group.y - 12 * u, w: group.w + 14 * u, h: group.h + 18 * u };
      const gc = [10 * u, 30 * u, 12 * u, 12 * u];
      m.group = { poly: chamferPoints(g.x, g.y, g.w, g.h, gc), inner: insetPoints(g.x, g.y, g.w, g.h, gc, 4 * u) };
    }

    // The grid's well: a recessed panel with a lit lip.
    const wc = [12 * u, 20 * u, 20 * u, 12 * u];
    const wx = lib.x - 6 * u;
    const wy = lib.y - 6 * u;
    m.well = { box: { x: wx, y: wy, w: lib.w + 12 * u, h: lib.h + 12 * u }, c: wc, poly: chamferPoints(wx, wy, lib.w + 12 * u, lib.h + 12 * u, wc), inner: insetPoints(wx, wy, lib.w + 12 * u, lib.h + 12 * u, wc, 4 * u) };

    // Header tab: from the emblem to an angled, arrow-like end.
    {
      const x0 = emblem ? emblem.x + emblem.w * 0.6 : head.x - 30 * u;
      const x1 = head.x + head.w + 22 * u;
      const y0 = head.y - 6 * u;
      const y1 = head.y + head.h + 4 * u;
      const tip = (y1 - y0) * 0.5;
      m.tab = {
        poly: [[x0, y0 + 10 * u], [x0 + 12 * u, y0], [x1 - tip * 0.3, y0], [x1 + tip * 0.7, y0 + tip], [x1 - tip * 0.3, y1], [x0 + 12 * u, y1]],
        inner: [[x0 + 16 * u, y0 + 6 * u], [x1 - tip * 0.4, y0 + 6 * u], [x1 + tip * 0.45, y0 + tip], [x1 - tip * 0.4, y1 - 6 * u], [x0 + 16 * u, y1 - 6 * u]],
        rail: [[x1 + tip * 0.95, y0 + tip], [x1 + tip * 1.5, top + 4 * u], [W - c * 1.2, top + 4 * u]],
      };
    }

    // Bottom bar: the selected game's panel (arrow notch on the left) and the prompt panel.
    if (now && prompts) {
      const by0 = bottom.y + 2 * u;
      const by1 = bottom.y + bottom.h - 4 * u;
      const mid = (by0 + by1) / 2;
      const nx0 = now.x - 40 * u;
      // One long plate under the title, from its arrow notch to just before the prompt plate.
      const nx1 = Math.max(nx0 + 200 * u, prompts.x - 62 * u);
      m.titlePanel = {
        poly: [[nx0, mid], [nx0 + 28 * u, by0], [nx1 - 26 * u, by0], [nx1, by0 + 26 * u], [nx1, by1 - 18 * u], [nx1 - 18 * u, by1], [nx0 + 28 * u, by1]],
        inner: [[nx0 + 12 * u, mid], [nx0 + 33 * u, by0 + 6 * u], [nx1 - 28.5 * u, by0 + 6 * u], [nx1 - 6 * u, by0 + 28.5 * u], [nx1 - 6 * u, by1 - 20.5 * u], [nx1 - 20.5 * u, by1 - 6 * u], [nx0 + 33 * u, by1 - 6 * u]],
      };
      const px0 = prompts.x - 46 * u;
      const px1 = prompts.x + prompts.w + 10 * u;
      m.promptPanel = {
        poly: [[px0 + 24 * u, by0 + 10 * u], [px1 - 6 * u, by0 + 10 * u], [px1 + 30 * u, mid + 6 * u], [px1 - 6 * u, by1], [px0 + 24 * u, by1], [px0, mid + 6 * u]],
        inner: [[px0 + 28 * u, by0 + 16 * u], [px1 - 8 * u, by0 + 16 * u], [px1 + 22 * u, mid + 6 * u], [px1 - 8 * u, by1 - 6 * u], [px0 + 28 * u, by1 - 6 * u], [px0 + 8 * u, mid + 6 * u]],
      };
    }

    // The emblem socket: a thick bevelled ring with notches, set into the frame's corner.
    if (emblem) {
      const r = emblem.w / 2;
      const ex = emblem.x + r;
      const ey = emblem.y + emblem.h / 2;
      m.emblem = { x: ex, y: ey, r, notches: [-160, -20, 50, 130].map((angle) => { const a = (angle * Math.PI) / 180; return [ex + Math.cos(a) * r * 1.08, ey + Math.sin(a) * r * 1.08, angle]; }) };
    }

    // Rivets at the corners and along the right edge.
    const rr = 5 * u;
    m.rivets = [[W - c * 0.7, top + c * 0.55, rr], [W - 15 * u, iy0 + c * 0.3, rr], [W - 15 * u, iy1 - c * 0.5, rr], [14 * u, H - c * 0.75, rr * 0.9]];
    return m;
  }

  function defs(svg, u) {
    const d = el("defs", {}, svg);
    const grad = (id, x1, y1, x2, y2, stops) => {
      const g = el("linearGradient", { id, x1, y1, x2, y2 }, d);
      for (const [offset, color, opacity = 1] of stops) el("stop", { offset, style: `stop-color:${color};stop-opacity:${opacity}` }, g);
    };
    grad("fa-body", 0, 0, 0.35, 1, [[0, "var(--frame-body-hi)"], [0.12, "var(--frame-body)"], [0.5, "var(--frame-body-lo)"], [0.85, "var(--frame-body)"], [1, "var(--frame-body-hi)"]]);
    grad("fa-edge", 0, 0, 1, 1, [[0, "var(--edge-hi)"], [0.3, "var(--edge)"], [0.65, "var(--edge-lo)"], [1, "var(--edge)"]]);
    grad("fa-tab", 0, 0, 0, 1, [[0, "var(--tab-hi)"], [0.45, "var(--tab)"], [1, "var(--tab-lo)"]]);
    grad("fa-glass", 0, 0, 1, 0, [[0, "var(--edge)", 0.08], [0.45, "var(--edge)", 0.28], [0.55, "var(--edge-hi)", 0.42], [1, "var(--edge)", 0.12]]);
    grad("fa-glass-v", 0, 0, 0, 1, [[0, "var(--edge-hi)", 0.4], [0.3, "var(--edge)", 0.22], [1, "var(--edge)", 0.06]]);
    grad("fa-panel", 0, 0, 0, 1, [[0, "var(--panel-hi)"], [1, "var(--panel)"]]);
    grad("fa-gloss", 0, 0, 1, 0.6, [[0, "#ffffff", 0.8], [0.3, "#ffffff", 0.15], [0.6, "#ffffff", 0]]);
    const glow = el("filter", { id: "fa-glow", x: "-20%", y: "-20%", width: "140%", height: "140%" }, d);
    el("feGaussianBlur", { stdDeviation: (2.6 * u + 0.6).toFixed(2), in: "SourceGraphic", result: "blur" }, glow);
    const merge = el("feMerge", {}, glow);
    el("feMergeNode", { in: "blur" }, merge);
    el("feMergeNode", { in: "blur" }, merge);
    el("feMergeNode", { in: "SourceGraphic" }, merge);
    const rivet = el("radialGradient", { id: "fa-rivet", cx: 0.4, cy: 0.36, r: 0.65 }, d);
    for (const [offset, color] of [[0, "#3a3a3a"], [0.55, "#0c0c0c"], [0.8, "#000"], [1, "var(--edge)"]]) el("stop", { offset, style: `stop-color:${color}` }, rivet);
    const orb = el("radialGradient", { id: "fa-ring", cx: 0.5, cy: 0.5, r: 0.5 }, d);
    for (const [offset, color, opacity] of [[0.78, "var(--frame-body-lo)", 1], [0.86, "var(--frame-body-hi)", 1], [0.93, "var(--frame-body)", 1], [1, "var(--edge-lo)", 1]]) el("stop", { offset, style: `stop-color:${color};stop-opacity:${opacity}` }, orb);
  }

  // A double edge along a closed polygon: a bright glowing line and a dimmer inset companion.
  function edge(group, points, width, { inner = null, glow = true, opacity = 1 } = {}) {
    el("path", { d: poly(points), fill: "none", stroke: "url(#fa-edge)", "stroke-width": width, "stroke-linejoin": "miter", "stroke-opacity": opacity, ...(glow ? { filter: "url(#fa-glow)" } : {}) }, group);
    if (inner) el("path", { d: poly(inner), fill: "none", stroke: "var(--edge)", "stroke-opacity": 0.45 * opacity, "stroke-width": Math.max(1, width * 0.45) }, group);
  }

  function panel(group, points, inner, { fill = "url(#fa-panel)", width = 1.6, glow = true, className = "" } = {}) {
    el("path", { d: poly(points), fill, class: className }, group);
    edge(group, points, width, { inner, glow });
  }

  // A dark bolt with a lit ring.
  function rivet(group, x, y, r) {
    el("circle", { cx: x, cy: y, r: r * 1.5, fill: "var(--frame-body-lo)", stroke: "var(--edge)", "stroke-width": Math.max(1, r * 0.25), "stroke-opacity": 0.8 }, group);
    el("circle", { cx: x, cy: y, r, fill: "url(#fa-rivet)" }, group);
    el("circle", { cx: x - r * 0.25, cy: y - r * 0.3, r: r * 0.28, fill: "#ffffff", opacity: 0.35 }, group);
  }

  // The SVG frame, used when the 3D chrome is not drawn (no WebGL 2). Returns the measurements.
  function draw(svg, frameNode) {
    if (!svg || !frameNode) return null;
    const m = measure(frameNode);
    if (!m) return null;
    const { W, H, u, c, top } = m;
    svg.setAttribute("viewBox", `0 0 ${W} ${H}`);
    svg.setAttribute("width", String(W));
    svg.setAttribute("height", String(H));
    svg.replaceChildren();
    defs(svg, u);

    const back = el("g", { class: "fa-back" }, svg);
    el("path", { d: `${poly(m.outer)}${poly(m.inner)}`, "fill-rule": "evenodd", fill: "url(#fa-body)" }, back);
    el("path", { d: poly(m.bar), fill: "url(#fa-glass-v)", stroke: "var(--edge)", "stroke-opacity": 0.5, "stroke-width": 1 }, back);
    el("path", { d: poly(m.rightGlass), fill: "url(#fa-glass)", stroke: "var(--edge)", "stroke-opacity": 0.55, "stroke-width": 1 }, back);
    el("path", { d: poly(m.leftGlass), fill: "url(#fa-glass)", stroke: "var(--edge)", "stroke-opacity": 0.45, "stroke-width": 1 }, back);
    // Interior: dark, nearly opaque, so the dome shows only around the frame.
    el("path", { d: poly(m.inner), fill: "var(--interior)", class: "fa-interior" }, back);

    const lines = el("g", { class: "fa-lines" }, svg);
    const inset = (points, k) => points.map(([x, y]) => [x + (x < W / 2 ? k : -k), y + (y < H / 2 ? k : -k)]);
    const { x: ix0, y: iy0, w: iw, h: ih, c: ic } = m.interior;
    edge(lines, m.outer, 3.8 * u + 1, { inner: inset(m.outer, 7 * u) });
    edge(lines, m.inner, 3.2 * u + 0.8, { inner: insetPoints(ix0, iy0, iw, ih, ic, -6 * u) });
    el("path", { d: poly(inset(m.outer, 3.5 * u)), fill: "none", stroke: "url(#fa-gloss)", "stroke-width": 2.6 * u, opacity: 0.55 }, lines);
    el("path", { d: poly(inset(m.outer, 11 * u)), fill: "none", stroke: "#000", "stroke-opacity": 0.45, "stroke-width": 2 * u }, lines);
    el("path", { d: poly(insetPoints(ix0, iy0, iw, ih, ic, 4 * u)), fill: "none", stroke: "var(--edge)", "stroke-opacity": 0.3, "stroke-width": 1 }, lines);

    panel(lines, m.side.poly, m.side.inner, { fill: "var(--side-panel)", width: 2 * u + 0.5 });
    for (const ear of m.ears) {
      el("path", { d: poly(ear.poly), fill: "url(#fa-tab)" }, lines);
      edge(lines, ear.poly, 1.4, { glow: false });
      for (const [x, y, r] of ear.rivets) rivet(lines, x, y, r);
    }
    if (m.group) panel(lines, m.group.poly, m.group.inner, { fill: "var(--side-group)", width: 1.6 * u + 0.4 });
    panel(lines, m.well.poly, m.well.inner, { fill: "var(--well)", width: 1.6 * u + 0.4, className: "fa-well" });

    el("path", { d: poly(m.tab.poly), fill: "url(#fa-tab)" }, lines);
    edge(lines, m.tab.poly, 2.2 * u + 0.5, { inner: m.tab.inner });
    el("path", { d: path(m.tab.rail), fill: "none", stroke: "var(--edge)", "stroke-opacity": 0.55, "stroke-width": 1.4 }, lines);

    for (const part of [m.titlePanel, m.promptPanel]) {
      if (!part) continue;
      el("path", { d: poly(part.poly), fill: "url(#fa-tab)" }, lines);
      edge(lines, part.poly, 2 * u + 0.5, { inner: part.inner });
    }

    if (m.emblem) {
      const { x: ex, y: ey, r } = m.emblem;
      const ring = el("g", { class: "fa-emblem" }, svg);
      for (const [x, y, angle] of m.emblem.notches) {
        el("rect", { x: x - 10 * u, y: y - 7 * u, width: 20 * u, height: 14 * u, transform: `rotate(${angle + 90} ${x} ${y})`, fill: "url(#fa-tab)", stroke: "var(--edge)", "stroke-width": 1.2 }, ring);
      }
      el("circle", { cx: ex, cy: ey, r: r * 1.06, fill: "url(#fa-ring)" }, ring);
      el("circle", { cx: ex, cy: ey, r: r * 1.06, fill: "none", stroke: "url(#fa-edge)", "stroke-width": 3 * u + 0.6, filter: "url(#fa-glow)" }, ring);
      el("circle", { cx: ex, cy: ey, r: r * 0.98, fill: "none", stroke: "var(--edge)", "stroke-opacity": 0.5, "stroke-width": 1.2 }, ring);
      el("circle", { cx: ex, cy: ey, r: r * 0.86, fill: "none", stroke: "url(#fa-edge)", "stroke-width": 2.4 * u + 0.5, filter: "url(#fa-glow)" }, ring);
      el("path", { d: `M${ex - r * 0.98} ${ey + r * 0.05}A${r} ${r} 0 0 1 ${ex + r * 0.3} ${ey - r * 0.95}`, fill: "none", stroke: "url(#fa-gloss)", "stroke-width": 5 * u, opacity: 0.85 }, ring);
    }

    const bolts = el("g", { class: "fa-rivets" }, svg);
    for (const [x, y, r] of m.rivets) rivet(bolts, x, y, r);

    // Gloss streaks along the top-left edges and down the left side, like light on glass.
    el("path", { d: path([[c * 0.2, top + c * 1.9], [c * 0.62, top + c * 1.0], [c * 0.62, top + 3 * u], [W * 0.45, top + 3 * u]]), fill: "none", stroke: "url(#fa-gloss)", "stroke-width": 5 * u, opacity: 0.8 }, svg);
    el("path", { d: path([[5 * u, top + c * 2.2], [5 * u, H * 0.72]]), fill: "none", stroke: "#ffffff", "stroke-opacity": 0.22, "stroke-width": 3 * u, "stroke-linecap": "round" }, svg);
    el("path", { d: path([[W * 0.5, H - 4 * u], [W - c * 1.2, H - 4 * u]]), fill: "none", stroke: "#ffffff", "stroke-opacity": 0.12, "stroke-width": 2.5 * u }, svg);
    return m;
  }

  root.GameStrikeFrame = Object.freeze({ draw, measure, chamferPoints, insetPoints, poly });
})(window);
