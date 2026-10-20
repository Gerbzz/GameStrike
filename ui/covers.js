// Generated box art for games without a cover image: a small illustrated scene picked from the
// title (a harbour for "Pixel Harbor", a forge for "Ember Forge"), a palette that suits it, and the
// full title set as a cover logo. Everything is drawn here on a canvas from core.coverArt's rules;
// no third-party art is used, and the detail view says the cover was drawn from the title.
(function (root) {
  "use strict";
  const core = root.GameStrikeCore;
  const TITLE_FONTS = {
    block: (px) => `700 ${px}px "GS Sans", sans-serif`,
    wide: (px) => `400 ${px * 0.78}px "GS Square", sans-serif`,
    slant: (px) => `700 ${px}px "GS Sans", sans-serif`,
    stencil: (px) => `400 ${px * 0.8}px "GS Mono", monospace`,
  };

  function mix(a, b, t) {
    const pa = parseInt(a.slice(1), 16);
    const pb = parseInt(b.slice(1), 16);
    const ch = (p, s) => (p >> s) & 255;
    const c = (s) => Math.round(ch(pa, s) + (ch(pb, s) - ch(pa, s)) * t);
    return `rgb(${c(16)} ${c(8)} ${c(0)})`;
  }

  // A ridge line across the width: smooth random hills.
  function ridge(rand, w, base, amp, bumps) {
    const points = [];
    const phases = Array.from({ length: 3 }, () => rand() * Math.PI * 2);
    for (let i = 0; i <= 48; i += 1) {
      const x = (i / 48) * w;
      const t = i / 48;
      const y = base - amp * (0.55 * Math.sin(t * bumps + phases[0]) + 0.3 * Math.sin(t * bumps * 2.3 + phases[1]) + 0.15 * Math.sin(t * bumps * 5.1 + phases[2]));
      points.push([x, y]);
    }
    return points;
  }
  function fillRidge(g, points, w, h, color) {
    g.beginPath();
    g.moveTo(0, h);
    for (const [x, y] of points) g.lineTo(x, y);
    g.lineTo(w, h);
    g.closePath();
    g.fillStyle = color;
    g.fill();
  }
  function glowDisc(g, x, y, r, color, glow = 2.6) {
    const halo = g.createRadialGradient(x, y, r * 0.2, x, y, r * glow);
    halo.addColorStop(0, color);
    halo.addColorStop(0.35, color.replace(")", " / 35%)").replace("rgb(", "rgb("));
    halo.addColorStop(1, "rgb(0 0 0 / 0%)");
    g.fillStyle = halo;
    g.beginPath();
    g.arc(x, y, r * glow, 0, Math.PI * 2);
    g.fill();
    g.fillStyle = color;
    g.beginPath();
    g.arc(x, y, r, 0, Math.PI * 2);
    g.fill();
  }
  function stars(g, rand, w, h, count, maxY) {
    for (let i = 0; i < count; i += 1) {
      const r = rand() < 0.08 ? 1.6 : 0.4 + rand() * 0.9;
      g.fillStyle = `rgb(255 255 255 / ${(0.3 + rand() * 0.7).toFixed(2)})`;
      g.beginPath();
      g.arc(rand() * w, rand() * h * maxY, r * (w / 200), 0, Math.PI * 2);
      g.fill();
    }
  }
  const hexRgb = (hex) => { const p = parseInt(hex.slice(1), 16); return `rgb(${(p >> 16) & 255} ${(p >> 8) & 255} ${p & 255})`; };

  const SCENES = {
    peaks(g, w, h, p, rand) {
      stars(g, rand, w, h, 40, 0.4);
      glowDisc(g, w * (0.3 + rand() * 0.4), h * 0.42, w * 0.13, hexRgb(p.light));
      for (let i = 0; i < 4; i += 1) fillRidge(g, ridge(rand, w, h * (0.55 + i * 0.1), h * (0.16 - i * 0.025), 4 + i * 2), w, h, mix(p.far, p.near, i / 3));
    },
    space(g, w, h, p, rand) {
      for (let i = 0; i < 5; i += 1) {
        const x = rand() * w; const y = rand() * h; const r = w * (0.3 + rand() * 0.4);
        const neb = g.createRadialGradient(x, y, 0, x, y, r);
        neb.addColorStop(0, i % 2 ? hexRgb(p.far).replace(")", " / 55%)") : hexRgb(p.sky[1]).replace(")", " / 40%)"));
        neb.addColorStop(1, "rgb(0 0 0 / 0%)");
        g.fillStyle = neb;
        g.fillRect(0, 0, w, h);
      }
      stars(g, rand, w, h, 140, 1);
      const px = w * (0.25 + rand() * 0.5); const py = h * (0.5 + rand() * 0.2); const pr = w * (0.24 + rand() * 0.1);
      const planet = g.createRadialGradient(px - pr * 0.4, py - pr * 0.4, pr * 0.1, px, py, pr);
      planet.addColorStop(0, hexRgb(p.light));
      planet.addColorStop(0.45, hexRgb(p.far));
      planet.addColorStop(1, hexRgb(p.near));
      g.save();
      g.translate(px, py);
      g.rotate(-0.35);
      g.strokeStyle = hexRgb(p.light).replace(")", " / 55%)");
      g.lineWidth = w * 0.018;
      g.beginPath(); g.ellipse(0, 0, pr * 1.7, pr * 0.42, 0, Math.PI, Math.PI * 2); g.stroke();
      g.restore();
      g.fillStyle = planet; g.beginPath(); g.arc(px, py, pr, 0, Math.PI * 2); g.fill();
      g.save(); g.translate(px, py); g.rotate(-0.35);
      g.strokeStyle = hexRgb(p.light).replace(")", " / 70%)");
      g.beginPath(); g.ellipse(0, 0, pr * 1.7, pr * 0.42, 0, 0, Math.PI); g.stroke();
      g.restore();
      glowDisc(g, w * (0.15 + rand() * 0.7), h * (0.15 + rand() * 0.2), w * 0.035, "rgb(255 255 255)", 3.5);
    },
    neon(g, w, h, p, rand) {
      const sy = h * 0.5;
      const sun = g.createLinearGradient(0, sy - w * 0.3, 0, sy);
      sun.addColorStop(0, hexRgb(p.light)); sun.addColorStop(1, hexRgb(p.sky[1]));
      g.save();
      g.beginPath(); g.arc(w / 2, sy, w * 0.3, Math.PI, 0); g.closePath(); g.clip();
      g.fillStyle = sun; g.fillRect(0, 0, w, h);
      g.fillStyle = hexRgb(p.sky[0]);
      for (let i = 0; i < 6; i += 1) g.fillRect(0, sy - w * 0.02 - i * w * 0.045, w, w * 0.006 + i * 0.002 * w);
      g.restore();
      // A skyline, then a perspective grid floor.
      for (let x = 0; x < w;) {
        const bw = w * (0.05 + rand() * 0.08); const bh = h * (0.04 + rand() * 0.14);
        g.fillStyle = hexRgb(p.near); g.fillRect(x, sy - bh, bw, bh);
        x += bw + w * 0.005;
      }
      g.fillStyle = hexRgb(p.near); g.fillRect(0, sy, w, h - sy);
      g.strokeStyle = hexRgb(p.light).replace(")", " / 75%)");
      g.lineWidth = Math.max(1, w / 260);
      for (let i = 0; i < 12; i += 1) { const y = sy + (h - sy) * Math.pow(i / 11, 2.2); g.beginPath(); g.moveTo(0, y); g.lineTo(w, y); g.stroke(); }
      for (let i = -8; i <= 8; i += 1) { g.beginPath(); g.moveTo(w / 2 + i * w * 0.03, sy); g.lineTo(w / 2 + i * w * 0.42, h); g.stroke(); }
    },
    forest(g, w, h, p, rand) {
      stars(g, rand, w, h, 50, 0.45);
      glowDisc(g, w * (0.2 + rand() * 0.6), h * 0.3, w * 0.1, hexRgb(p.light), 3);
      for (let layer = 0; layer < 4; layer += 1) {
        const base = h * (0.6 + layer * 0.12);
        g.fillStyle = mix(p.far, p.near, layer / 3);
        for (let x = -w * 0.1; x < w * 1.1; x += w * (0.07 + rand() * 0.06)) {
          const th = h * (0.18 + rand() * 0.16) * (1 - layer * 0.12) + layer * h * 0.04;
          const tw = th * 0.38;
          g.beginPath(); g.moveTo(x, base - th); g.lineTo(x + tw / 2, base); g.lineTo(x - tw / 2, base); g.closePath(); g.fill();
        }
        g.fillRect(0, base - 1, w, h - base + 1);
        // Mist between layers.
        const mist = g.createLinearGradient(0, base - h * 0.1, 0, base);
        mist.addColorStop(0, "rgb(255 255 255 / 0%)"); mist.addColorStop(1, hexRgb(p.light).replace(")", " / 10%)"));
        g.fillStyle = mist; g.fillRect(0, base - h * 0.1, w, h * 0.1);
      }
      for (let i = 0; i < 18; i += 1) glowDisc(g, rand() * w, h * (0.55 + rand() * 0.4), w * 0.006, hexRgb(p.light), 4);
    },
    reef(g, w, h, p, rand) {
      g.globalCompositeOperation = "lighter";
      for (let i = 0; i < 6; i += 1) {
        const x = w * (0.1 + rand() * 0.8);
        const ray = g.createLinearGradient(0, 0, 0, h * 0.8);
        ray.addColorStop(0, hexRgb(p.light).replace(")", " / 22%)")); ray.addColorStop(1, "rgb(0 0 0 / 0%)");
        g.fillStyle = ray;
        g.beginPath(); g.moveTo(x - w * 0.05, 0); g.lineTo(x + w * 0.05, 0); g.lineTo(x + w * 0.25, h * 0.8); g.lineTo(x - w * 0.05, h * 0.8); g.closePath(); g.fill();
      }
      g.globalCompositeOperation = "source-over";
      for (let i = 0; i < 26; i += 1) { g.strokeStyle = "rgb(255 255 255 / 45%)"; g.lineWidth = Math.max(1, w / 300); g.beginPath(); g.arc(rand() * w, rand() * h * 0.9, w * (0.006 + rand() * 0.014), 0, Math.PI * 2); g.stroke(); }
      for (let i = 0; i < 5; i += 1) {
        const fx = rand() * w; const fy = h * (0.3 + rand() * 0.35); const fs = w * (0.04 + rand() * 0.05);
        g.fillStyle = mix(p.far, p.near, 0.4);
        g.beginPath(); g.ellipse(fx, fy, fs, fs * 0.45, 0, 0, Math.PI * 2); g.fill();
        g.beginPath(); g.moveTo(fx + fs * 0.8, fy); g.lineTo(fx + fs * 1.5, fy - fs * 0.45); g.lineTo(fx + fs * 1.5, fy + fs * 0.45); g.closePath(); g.fill();
      }
      fillRidge(g, ridge(rand, w, h * 0.86, h * 0.06, 7), w, h, hexRgb(p.near));
      g.strokeStyle = hexRgb(p.far); g.lineWidth = w * 0.025; g.lineCap = "round";
      for (let i = 0; i < 9; i += 1) {
        const x = rand() * w; const top = h * (0.55 + rand() * 0.2);
        g.beginPath(); g.moveTo(x, h); g.bezierCurveTo(x + w * 0.08, h * 0.8, x - w * 0.08, top + h * 0.1, x + w * 0.03, top); g.stroke();
      }
    },
    emblem(g, w, h, p, rand) {
      const cx = w / 2; const cy = h * 0.52;
      g.save(); g.translate(cx, cy);
      for (let i = 0; i < 24; i += 1) {
        g.rotate((Math.PI * 2) / 24);
        g.fillStyle = i % 2 ? hexRgb(p.far).replace(")", " / 45%)") : "rgb(0 0 0 / 0%)";
        g.beginPath(); g.moveTo(0, 0); g.lineTo(w * 0.09, -h); g.lineTo(-w * 0.09, -h); g.closePath(); g.fill();
      }
      g.restore();
      const r = w * 0.28; const sides = 5 + Math.floor(rand() * 3);
      const metal = g.createLinearGradient(cx - r, cy - r, cx + r, cy + r);
      metal.addColorStop(0, "#ffffff"); metal.addColorStop(0.3, hexRgb(p.light)); metal.addColorStop(0.55, hexRgb(p.far)); metal.addColorStop(1, hexRgb(p.near));
      for (const [scale, fill] of [[1.12, hexRgb(p.near)], [1, metal], [0.62, hexRgb(p.edge)]]) {
        g.beginPath();
        for (let k = 0; k <= sides; k += 1) { const a = -Math.PI / 2 + (k * Math.PI * 2) / sides; g.lineTo(cx + Math.cos(a) * r * scale, cy + Math.sin(a) * r * scale); }
        g.fillStyle = fill; g.fill();
      }
      glowDisc(g, cx, cy, w * 0.07, hexRgb(p.light), 3);
    },
    dunes(g, w, h, p, rand) {
      glowDisc(g, w * (0.3 + rand() * 0.4), h * 0.38, w * 0.18, hexRgb(p.light), 2.2);
      const base = h * 0.66;
      g.fillStyle = mix(p.far, p.near, 0.5);
      g.beginPath(); g.moveTo(w * 0.62, base); g.lineTo(w * 0.78, base - h * 0.16); g.lineTo(w * 0.94, base); g.closePath(); g.fill();
      for (let i = 0; i < 4; i += 1) fillRidge(g, ridge(rand, w, h * (0.64 + i * 0.09), h * 0.05, 2 + i), w, h, mix(p.far, p.near, i / 3));
    },
    city(g, w, h, p, rand) {
      stars(g, rand, w, h, 30, 0.3);
      g.globalCompositeOperation = "lighter";
      for (let i = 0; i < 3; i += 1) {
        const x = w * (0.2 + rand() * 0.6);
        const beam = g.createLinearGradient(x, h, x, 0);
        beam.addColorStop(0, hexRgb(p.light).replace(")", " / 30%)")); beam.addColorStop(1, "rgb(0 0 0 / 0%)");
        g.fillStyle = beam; g.beginPath(); g.moveTo(x - w * 0.01, h * 0.7); g.lineTo(x + w * 0.01, h * 0.7); g.lineTo(x + (rand() - 0.5) * w + w * 0.08, 0); g.lineTo(x + (rand() - 0.5) * w - w * 0.08, 0); g.closePath(); g.fill();
      }
      g.globalCompositeOperation = "source-over";
      for (const [depth, top] of [[0, 0.42], [1, 0.52]]) {
        for (let x = -w * 0.02; x < w;) {
          const bw = w * (0.08 + rand() * 0.1); const bh = h * (0.18 + rand() * 0.26) * (depth ? 0.8 : 1);
          const y = h * top + h * 0.3 - bh;
          g.fillStyle = depth ? hexRgb(p.near) : mix(p.far, p.near, 0.55);
          g.fillRect(x, y, bw, h - y);
          g.fillStyle = hexRgb(p.light).replace(")", depth ? " / 85%)" : " / 45%)");
          for (let wy = y + w * 0.02; wy < h - w * 0.02; wy += w * 0.035) for (let wx = x + w * 0.015; wx < x + bw - w * 0.02; wx += w * 0.03) if (rand() < 0.4) g.fillRect(wx, wy, w * 0.012, w * 0.016);
          x += bw + w * 0.008;
        }
      }
    },
    castle(g, w, h, p, rand) {
      stars(g, rand, w, h, 40, 0.4);
      glowDisc(g, w * 0.72, h * 0.28, w * 0.09, hexRgb(p.light), 3);
      fillRidge(g, ridge(rand, w, h * 0.74, h * 0.06, 3), w, h, mix(p.far, p.near, 0.5));
      const base = h * 0.72; const cx = w * (0.35 + rand() * 0.3);
      g.fillStyle = hexRgb(p.near);
      const tower = (x, tw, th) => {
        g.fillRect(x - tw / 2, base - th, tw, th);
        g.beginPath(); g.moveTo(x - tw * 0.65, base - th); g.lineTo(x, base - th - tw * 1.2); g.lineTo(x + tw * 0.65, base - th); g.closePath(); g.fill();
      };
      g.fillRect(cx - w * 0.22, base - h * 0.12, w * 0.44, h * 0.3);
      tower(cx - w * 0.22, w * 0.08, h * 0.2); tower(cx + w * 0.22, w * 0.08, h * 0.2); tower(cx, w * 0.11, h * 0.3);
      g.fillStyle = hexRgb(p.light);
      for (let i = 0; i < 7; i += 1) g.fillRect(cx - w * 0.18 + i * w * 0.06, base - h * 0.06, w * 0.014, w * 0.024);
      fillRidge(g, ridge(rand, w, h * 0.9, h * 0.03, 4), w, h, hexRgb(p.near));
    },
    harbor(g, w, h, p, rand) {
      stars(g, rand, w, h, 30, 0.35);
      glowDisc(g, w * (0.2 + rand() * 0.6), h * 0.32, w * 0.1, hexRgb(p.light), 2.6);
      const sea = h * 0.6;
      fillRidge(g, ridge(rand, w, sea, h * 0.04, 3), w, h, mix(p.far, p.near, 0.4));
      g.fillStyle = hexRgb(p.near); g.fillRect(0, sea, w, h - sea);
      g.strokeStyle = hexRgb(p.light).replace(")", " / 35%)"); g.lineWidth = Math.max(1, w / 240);
      for (let i = 0; i < 14; i += 1) { const y = sea + (h - sea) * (i / 14); g.beginPath(); g.moveTo(rand() * w * 0.5, y); g.lineTo(w * 0.5 + rand() * w * 0.5, y); g.stroke(); }
      // A lighthouse with its beam.
      const lx = w * 0.78;
      g.globalCompositeOperation = "lighter";
      const beam = g.createLinearGradient(lx, sea - h * 0.3, 0, sea - h * 0.38);
      beam.addColorStop(0, hexRgb(p.light).replace(")", " / 55%)")); beam.addColorStop(1, "rgb(0 0 0 / 0%)");
      g.fillStyle = beam; g.beginPath(); g.moveTo(lx, sea - h * 0.3); g.lineTo(0, sea - h * 0.45); g.lineTo(0, sea - h * 0.27); g.closePath(); g.fill();
      g.globalCompositeOperation = "source-over";
      g.fillStyle = mix(p.near, "#000000", 0.2);
      g.beginPath(); g.moveTo(lx - w * 0.05, sea); g.lineTo(lx - w * 0.03, sea - h * 0.28); g.lineTo(lx + w * 0.03, sea - h * 0.28); g.lineTo(lx + w * 0.05, sea); g.closePath(); g.fill();
      glowDisc(g, lx, sea - h * 0.3, w * 0.025, hexRgb(p.light), 3);
      // A small sailboat.
      const bx = w * (0.2 + rand() * 0.3);
      g.fillStyle = hexRgb(p.near);
      g.beginPath(); g.moveTo(bx - w * 0.08, sea + h * 0.04); g.lineTo(bx + w * 0.08, sea + h * 0.04); g.lineTo(bx + w * 0.06, sea + h * 0.07); g.lineTo(bx - w * 0.06, sea + h * 0.07); g.closePath(); g.fill();
      g.fillStyle = hexRgb(p.ink).replace(")", " / 80%)");
      g.beginPath(); g.moveTo(bx, sea + h * 0.035); g.lineTo(bx, sea - h * 0.1); g.lineTo(bx + w * 0.07, sea + h * 0.03); g.closePath(); g.fill();
    },
    frost(g, w, h, p, rand) {
      glowDisc(g, w * 0.5, h * 0.35, w * 0.08, "rgb(255 255 255)", 3.5);
      for (let i = 0; i < 4; i += 1) {
        const pts = [];
        for (let k = 0; k <= 10; k += 1) pts.push([(k / 10) * w, h * (0.55 + i * 0.1) - (k % 2 ? h * (0.14 - i * 0.025) * (0.5 + rand()) : 0)]);
        fillRidge(g, pts, w, h, mix("#ffffff", p.near, 0.25 + i * 0.25));
      }
      g.fillStyle = "rgb(255 255 255 / 85%)";
      for (let i = 0; i < 80; i += 1) { g.beginPath(); g.arc(rand() * w, rand() * h, w * (0.003 + rand() * 0.006), 0, Math.PI * 2); g.fill(); }
    },
    forge(g, w, h, p, rand) {
      g.globalCompositeOperation = "lighter";
      const fire = g.createRadialGradient(w / 2, h * 0.8, 0, w / 2, h * 0.8, h * 0.6);
      fire.addColorStop(0, "rgb(255 220 120 / 90%)"); fire.addColorStop(0.3, hexRgb(p.sky[1]).replace(")", " / 60%)")); fire.addColorStop(1, "rgb(0 0 0 / 0%)");
      g.fillStyle = fire; g.fillRect(0, 0, w, h);
      for (let i = 0; i < 60; i += 1) glowDisc(g, rand() * w, h * (0.2 + rand() * 0.7), w * (0.003 + rand() * 0.006), "rgb(255 190 90)", 3);
      g.globalCompositeOperation = "source-over";
      // An anvil silhouette on a rocky floor.
      fillRidge(g, ridge(rand, w, h * 0.86, h * 0.04, 6), w, h, hexRgb(p.near));
      const ax = w / 2; const ay = h * 0.8;
      g.fillStyle = hexRgb(p.near);
      g.beginPath(); g.moveTo(ax - w * 0.26, ay - h * 0.08); g.lineTo(ax + w * 0.2, ay - h * 0.08); g.lineTo(ax + w * 0.3, ay - h * 0.05); g.lineTo(ax + w * 0.08, ay - h * 0.03); g.lineTo(ax + w * 0.1, ay + h * 0.04); g.lineTo(ax - w * 0.12, ay + h * 0.04); g.lineTo(ax - w * 0.1, ay - h * 0.03); g.lineTo(ax - w * 0.2, ay - h * 0.04); g.closePath(); g.fill();
    },
  };

  function wrapTitle(g, text, maxWidth) {
    const words = text.split(/\s+/);
    const lines = [];
    let line = "";
    for (const word of words) {
      const next = line ? `${line} ${word}` : word;
      if (g.measureText(next).width <= maxWidth || !line) line = next;
      else { lines.push(line); line = word; }
    }
    if (line) lines.push(line);
    return lines;
  }

  // The cover logo: the full title, big, with a dark edge, a light fill, and a glow.
  function drawTitle(g, w, h, art) {
    const p = art.palette;
    const text = art.title.toLocaleUpperCase();
    let px = w * 0.2 * art.titleScale;
    let lines;
    for (let tries = 0; tries < 8; tries += 1) {
      g.font = TITLE_FONTS[art.titleStyle](px);
      lines = wrapTitle(g, text, w * 0.86);
      const widest = Math.max(...lines.map((line) => g.measureText(line).width));
      if (lines.length <= 3 && widest <= w * 0.9) break;
      px *= 0.86;
    }
    if (lines.length > 4) lines = [...lines.slice(0, 3), `${lines[3]}…`];
    const lineHeight = px * (art.titleStyle === "wide" ? 0.82 : 0.95);
    const block = lineHeight * lines.length;
    const top = art.titleAt === "top" ? h * 0.07 : h * 0.93 - block - h * 0.04;
    g.save();
    g.textAlign = "center";
    g.textBaseline = "top";
    if (art.titleStyle === "slant") { g.translate(w / 2, top + block / 2); g.transform(1, 0, -0.18, 1, 0, 0); g.translate(-w / 2, -(top + block / 2)); }
    lines.forEach((line, index) => {
      const y = top + index * lineHeight;
      const fill = g.createLinearGradient(0, y, 0, y + px);
      fill.addColorStop(0, "#ffffff");
      fill.addColorStop(0.5, hexRgb(p.ink));
      fill.addColorStop(1, hexRgb(p.light));
      g.lineJoin = "miter";
      g.lineWidth = Math.max(2, px * 0.16);
      g.strokeStyle = hexRgb(p.edge);
      g.shadowColor = "rgb(0 0 0 / 70%)";
      g.shadowBlur = px * 0.3;
      g.shadowOffsetY = px * 0.06;
      g.strokeText(line, w / 2, y);
      g.shadowColor = hexRgb(p.light).replace(")", " / 60%)");
      g.shadowBlur = px * 0.35;
      g.shadowOffsetY = 0;
      g.fillStyle = fill;
      g.fillText(line, w / 2, y);
    });
    g.restore();
    // A small kicker (the source or how it was imported) on a rule, opposite the title.
    if (art.kicker) {
      g.save();
      g.font = `600 ${Math.max(8, w * 0.055)}px "GS Sans", sans-serif`;
      g.textAlign = "center";
      g.textBaseline = "middle";
      const ky = art.titleAt === "top" ? h * 0.93 : h * 0.07;
      const label = art.kicker.toLocaleUpperCase().split("").join(String.fromCharCode(8202));
      const kw = g.measureText(label).width + w * 0.1;
      g.fillStyle = "rgb(0 0 0 / 55%)";
      g.fillRect(w / 2 - kw / 2, ky - w * 0.045, kw, w * 0.09);
      g.fillStyle = hexRgb(p.light);
      g.fillRect(w / 2 - kw / 2, ky - w * 0.045, kw, Math.max(1, w * 0.006));
      g.fillStyle = hexRgb(p.ink);
      g.fillText(label, w / 2, ky);
      g.restore();
    }
  }

  // Paints a cover onto `canvas` at its current pixel size.
  function paint(canvas, game, kicker) {
    const g = canvas.getContext("2d");
    if (!g || !core) return;
    const w = canvas.width;
    const h = canvas.height;
    const art = core.coverArt(game.id, game.title, kicker);
    const p = art.palette;
    const rand = core.seededRandom(art.seed);
    g.save();
    const sky = g.createLinearGradient(0, 0, 0, h);
    sky.addColorStop(0, hexRgb(p.sky[0]));
    sky.addColorStop(0.62, hexRgb(p.sky[1]));
    sky.addColorStop(1, hexRgb(p.near));
    g.fillStyle = sky;
    g.fillRect(0, 0, w, h);
    try { (SCENES[art.scene] || SCENES.peaks)(g, w, h, p, rand); } catch (_) { /* a scene never blocks the cover */ }
    g.restore();
    // Darken behind the title so it always reads.
    const shade = g.createLinearGradient(0, art.titleAt === "top" ? 0 : h, 0, art.titleAt === "top" ? h * 0.45 : h * 0.55);
    shade.addColorStop(0, "rgb(0 0 0 / 55%)");
    shade.addColorStop(1, "rgb(0 0 0 / 0%)");
    g.fillStyle = shade;
    g.fillRect(0, 0, w, h);
    drawTitle(g, w, h, art);
    // Vignette and a fine grain, like printed box art.
    const vignette = g.createRadialGradient(w / 2, h * 0.45, w * 0.3, w / 2, h * 0.5, h * 0.75);
    vignette.addColorStop(0, "rgb(0 0 0 / 0%)");
    vignette.addColorStop(1, "rgb(0 0 0 / 45%)");
    g.fillStyle = vignette;
    g.fillRect(0, 0, w, h);
  }

  const cache = new Map();
  // A canvas with the game's cover at (width x height) device pixels. Cached per size.
  function canvasFor(game, kicker, width, height) {
    const w = Math.max(40, Math.round(width));
    const h = Math.max(54, Math.round(height));
    const key = `${game.id}|${game.title}|${kicker}|${w}x${h}`;
    let source = cache.get(key);
    if (!source) {
      source = document.createElement("canvas");
      source.width = w;
      source.height = h;
      paint(source, game, kicker);
      cache.set(key, source);
      if (cache.size > 400) cache.delete(cache.keys().next().value);
    }
    const copy = document.createElement("canvas");
    copy.width = w;
    copy.height = h;
    copy.getContext("2d")?.drawImage(source, 0, 0);
    return copy;
  }
  function clearCache() { cache.clear(); }

  root.GameStrikeCovers = Object.freeze({ paint, canvasFor, clearCache });
})(window);
