// The Collection style's 3D chrome: the frame body, the sidebar and bottom panels, the header tab,
// the emblem disc, glass inlays, and rivets, extruded from the outlines frame.js measures and lit in
// the dome. Each outline becomes a slab whose flat face is dark, glossy plastic and whose bevelled
// rims are lit amber, so every edge reads as thick and rounded; a small procedural environment map
// gives the gloss its reflections. The slabs sit exactly on the page's layout (one pixel of the
// layout is one pixel of the frame's plane), with the DOM's text and covers drawn over them.
// Original geometry; decoration only. Used by scene.js.
import * as THREE from "./vendor/three/three.module.min.js";

// Colours per variant: hex values are sRGB (three.js converts them to linear light); `pipe` is
// linear RGB, scaled above 1 for the light pipes so that they bloom.
const PALETTES = {
  orange: { pipe: [1.0, 0.22, 0.02], face: 0x1a0803, faceGlow: 0x0d0301, panel: 0x120502, panelGlow: 0x0a0200, edge: 0xff7a18, edgeGlow: 0xff5000, glass: 0x120502, glassGlow: 0xff5a10, ring: 0xff7a1c, ringGlow: 0xff4800, light: 0xffb36b, rim: 0xff7a1c, disc: 0x140602, discGlow: 0x9a2a00, interior: 0x090301 },
  green: { pipe: [0.2, 0.85, 0.04], face: 0x081203, faceGlow: 0x020801, panel: 0x050c02, panelGlow: 0x010600, edge: 0x7fe03a, edgeGlow: 0x3aaa00, glass: 0x050c02, glassGlow: 0x5aff10, ring: 0x8fe04a, ringGlow: 0x3aaa00, light: 0xc8ff9a, rim: 0x6fd12e, disc: 0x061404, discGlow: 0x2a7a00, interior: 0x030902 },
};

// ---------- the environment the gloss reflects ----------
// An equirectangular HDR sky painted here: a warm softbox overhead (the bright band along every
// top-facing bevel), a key light behind the camera at the upper left, a strip light, an amber glow
// where the dome's light is, and a dark floor (so lower bevels fall into shadow).
function paintEnvironment(tint) {
  const width = 256;
  const height = 128;
  const data = new Uint16Array(width * height * 4);
  const key = new THREE.Vector3(-0.45, 0.55, 0.7).normalize();
  const rim = new THREE.Vector3(0.9, 0.1, -0.2).normalize();
  const dir = new THREE.Vector3();
  const t = tint;
  for (let y = 0; y < height; y += 1) {
    const v = 1 - (y + 0.5) / height; // canvas top is up
    const lat = (v - 0.5) * Math.PI;
    for (let x = 0; x < width; x += 1) {
      const u = (x + 0.5) / width;
      const lon = (u - 0.5) * Math.PI * 2;
      // three.js equirect: u = atan(z, x) / 2pi + 0.5, v = asin(y) / pi + 0.5
      dir.set(Math.cos(lat) * Math.cos(lon), Math.sin(lat), Math.cos(lat) * Math.sin(lon));
      const up = Math.max(dir.y, 0);
      const down = Math.max(-dir.y, 0);
      let r = 0.004 + 0.02 * up;
      let g = 0.0025 + 0.011 * up;
      let b = 0.002 + 0.007 * up;
      r *= 1 - down * 0.9; g *= 1 - down * 0.9; b *= 1 - down * 0.9;
      const zenith = Math.pow(up, 10) * 3.2;
      const keyLobe = Math.pow(Math.max(dir.dot(key), 0), 400) * 14;
      const strip = Math.exp(-(((dir.y - 0.6) / 0.05) ** 2)) * Math.max(dir.z, 0) * 1.6 + Math.exp(-(((dir.y - 0.12) / 0.03) ** 2)) * Math.max(dir.z, 0) * Math.max(0, -dir.x) * 0.25;
      const amber = Math.pow(Math.max(-dir.z, 0), 2) * 0.35 * (0.5 + 0.5 * up);
      const rimLobe = Math.pow(Math.max(dir.dot(rim), 0), 12) * 1.6;
      r += (zenith + keyLobe + strip) * 1.0 + (amber + rimLobe) * t[0];
      g += (zenith + keyLobe + strip) * 0.82 + (amber + rimLobe) * t[1];
      b += (zenith + keyLobe + strip) * 0.62 + (amber + rimLobe) * t[2];
      const o = (y * width + x) * 4;
      data[o] = THREE.DataUtils.toHalfFloat(r);
      data[o + 1] = THREE.DataUtils.toHalfFloat(g);
      data[o + 2] = THREE.DataUtils.toHalfFloat(b);
      data[o + 3] = THREE.DataUtils.toHalfFloat(1);
    }
  }
  const texture = new THREE.DataTexture(data, width, height, THREE.RGBAFormat, THREE.HalfFloatType);
  texture.mapping = THREE.EquirectangularReflectionMapping;
  texture.flipY = true;
  texture.magFilter = THREE.LinearFilter;
  texture.minFilter = THREE.LinearFilter;
  texture.needsUpdate = true;
  return texture;
}

export function createEnvironment(renderer, tint) {
  const pmrem = new THREE.PMREMGenerator(renderer);
  const source = paintEnvironment(tint);
  const target = pmrem.fromEquirectangular(source);
  source.dispose();
  pmrem.dispose();
  return target.texture;
}

// ---------- materials ----------
function makeMaterials(envMap) {
  const physical = (options) => new THREE.MeshPhysicalMaterial({ envMap, ...options });
  return {
    face: physical({ color: 0x1a0803, emissive: 0x0d0301, emissiveIntensity: 1, specularIntensity: 0.2, metalness: 0.0, roughness: 0.5, clearcoat: 1, clearcoatRoughness: 0.05, envMapIntensity: 0.9 }),
    panel: physical({ color: 0x120502, emissive: 0x0a0200, emissiveIntensity: 1, specularIntensity: 0.15, metalness: 0.0, roughness: 0.6, clearcoat: 1, clearcoatRoughness: 0.08, envMapIntensity: 0.7 }),
    edge: physical({ color: 0xff7a18, emissive: 0xff5000, emissiveIntensity: 0.8, metalness: 0.2, roughness: 0.28, clearcoat: 1, clearcoatRoughness: 0.04, envMapIntensity: 1.2 }),
    glass: physical({ color: 0x120502, emissive: 0xff5a10, emissiveIntensity: 0.03, specularIntensity: 0.3, metalness: 0, roughness: 0.3, clearcoat: 1, clearcoatRoughness: 0.03, envMapIntensity: 0.8, transparent: true, opacity: 0.85, depthWrite: false }),
    head: physical({ color: 0x151110, metalness: 0.85, roughness: 0.22, clearcoat: 1, clearcoatRoughness: 0.05, envMapIntensity: 1.4, side: THREE.DoubleSide }),
    disc: physical({ color: 0x140602, emissive: 0x9a2a00, emissiveIntensity: 0.9, metalness: 0.0, roughness: 0.35, clearcoat: 1, clearcoatRoughness: 0.03, envMapIntensity: 1.2, side: THREE.DoubleSide }),
    ring: physical({ color: 0xff7a1c, emissive: 0xff4800, emissiveIntensity: 0.45, metalness: 0.3, roughness: 0.24, clearcoat: 1, clearcoatRoughness: 0.04, envMapIntensity: 1.4, side: THREE.DoubleSide }),
    interior: new THREE.MeshBasicMaterial({ color: 0x090301, transparent: true, opacity: 0.9, depthWrite: false, side: THREE.DoubleSide }),
    well: new THREE.MeshBasicMaterial({ color: 0x050100, transparent: true, opacity: 0.55, depthWrite: false, side: THREE.DoubleSide }),
    // Light pipes: colours above 1 so the bloom pass makes them glow.
    pipe: new THREE.MeshBasicMaterial({ color: new THREE.Color(1.0, 0.22, 0.02).multiplyScalar(1.9), side: THREE.DoubleSide }),
    pipeDim: new THREE.MeshBasicMaterial({ color: new THREE.Color(1.0, 0.22, 0.02).multiplyScalar(0.75), side: THREE.DoubleSide }),
  };
}

// A radial glow painted once, for the emblem disc's inner light.
function glowTexture() {
  const size = 256;
  const paint = document.createElement("canvas");
  paint.width = paint.height = size;
  const g = paint.getContext("2d");
  const gradient = g.createRadialGradient(size * 0.5, size * 0.56, size * 0.05, size * 0.5, size * 0.5, size * 0.5);
  gradient.addColorStop(0, "#ffffff");
  gradient.addColorStop(0.45, "#7a7a7a");
  gradient.addColorStop(0.8, "#1a1a1a");
  gradient.addColorStop(1, "#000000");
  g.fillStyle = gradient;
  g.fillRect(0, 0, size, size);
  const texture = new THREE.CanvasTexture(paint);
  texture.colorSpace = THREE.SRGBColorSpace;
  return texture;
}

// ---------- geometry ----------
// Every outline is in viewport pixels; `toLocal` maps a pixel to the frame plane (y up, centred).
// Zero-size chamfers repeat a corner; repeated points break the bevel and the triangulation.
function distinct(points) {
  const out = [];
  for (const p of points) {
    const prev = out[out.length - 1];
    if (!prev || Math.hypot(p[0] - prev[0], p[1] - prev[1]) > 0.05) out.push(p);
  }
  while (out.length > 2 && Math.hypot(out[0][0] - out[out.length - 1][0], out[0][1] - out[out.length - 1][1]) <= 0.05) out.pop();
  return out;
}
function shapeFrom(points, toLocal, holes = []) {
  const shape = new THREE.Shape(distinct(points).map((p) => toLocal(p)));
  for (const hole of holes) shape.holes.push(new THREE.Path(distinct(hole).map((p) => toLocal(p))));
  return shape;
}

// A slab: the face on the frame plane (z = 0), the body and its rounded bevel behind it. Material 0
// is the face, material 1 the rims (ExtrudeGeometry's groups).
function slab(shape, k, { depth = 10, bevel = 6, size = bevel * 0.9, segments = 4 } = {}) {
  const geometry = new THREE.ExtrudeGeometry(shape, { depth: depth * k, bevelEnabled: true, bevelThickness: bevel * k, bevelSize: size * k, bevelSegments: segments, curveSegments: 1 });
  geometry.translate(0, 0, -(depth + bevel) * k);
  return geometry;
}

// A polygon moved outward (d > 0) or inward by d pixels, with mitred corners.
function offsetPolygon(points, d) {
  const pts = distinct(points);
  const n = pts.length;
  let area = 0;
  for (let i = 0; i < n; i += 1) { const [x1, y1] = pts[i]; const [x2, y2] = pts[(i + 1) % n]; area += x1 * y2 - x2 * y1; }
  // Pixel y points down: a positive area is clockwise on screen, whose outward normal is (dy, -dx).
  const sign = area > 0 ? 1 : -1;
  const normal = (a, b) => { const dx = b[0] - a[0]; const dy = b[1] - a[1]; const len = Math.hypot(dx, dy) || 1; return [(sign * dy) / len, (-sign * dx) / len]; };
  return pts.map((p, i) => {
    const n1 = normal(pts[(i - 1 + n) % n], p);
    const n2 = normal(p, pts[(i + 1) % n]);
    const bx = n1[0] + n2[0];
    const by = n1[1] + n2[1];
    const len = Math.hypot(bx, by) || 1;
    const cos = Math.max(0.35, (n1[0] * bx + n1[1] * by) / len);
    return [p[0] + (bx / len) * (d / cos), p[1] + (by / len) * (d / cos)];
  });
}
// A thin glowing line along an outline, as a flat ring (the reference's lit double edges).
function pipe(points, width, toLocal) {
  return new THREE.ShapeGeometry(shapeFrom(offsetPolygon(points, width / 2), toLocal, [offsetPolygon(points, -width / 2)]));
}

// A screw head: an amber washer and a dark domed head with a slot.
function rivetGeometry(r) {
  const washer = new THREE.LatheGeometry([new THREE.Vector2(r * 0.9, -r * 0.6), new THREE.Vector2(r * 1.65, -r * 0.6), new THREE.Vector2(r * 1.7, -r * 0.2), new THREE.Vector2(r * 1.5, r * 0.05), new THREE.Vector2(r * 0.95, r * 0.05)], 28);
  const head = new THREE.LatheGeometry(Array.from({ length: 9 }, (_, i) => { const a = (i / 8) * (Math.PI / 2); return new THREE.Vector2(Math.cos(a) * r * 1.02 + 0.0001, Math.sin(a) * r * 0.62 + r * 0.02); }).reverse(), 28);
  washer.rotateX(Math.PI / 2);
  head.rotateX(Math.PI / 2);
  return { washer, head };
}

// The emblem: a glossy domed disc inside a thick bevelled ring.
function emblemGeometry(r) {
  const ringProfile = [
    [0.82, -0.08], [0.84, 0.05], [0.88, 0.13], [0.95, 0.16], [1.02, 0.13], [1.08, 0.05], [1.1, -0.04], [1.1, -0.2], [0.82, -0.2],
  ].map(([x, y]) => new THREE.Vector2(x * r, y * r));
  const ring = new THREE.LatheGeometry(ringProfile, 96);
  ring.rotateX(Math.PI / 2);
  const discProfile = Array.from({ length: 10 }, (_, i) => { const a = (i / 9) * (Math.PI / 2); return new THREE.Vector2(Math.sin(a) * r * 0.84 + 0.0001, Math.cos(a) * r * 0.06 - r * 0.04); });
  const disc = new THREE.LatheGeometry(discProfile, 96);
  disc.rotateX(Math.PI / 2);
  return { ring, disc };
}

const easeOut = (t) => 1 - (1 - t) ** 3;
const easeBack = (t) => { const c = 1.6; return 1 + (c + 1) * (t - 1) ** 3 + c * (t - 1) ** 2; };
const clamp01 = (t) => Math.min(1, Math.max(0, t));

export function createChrome({ envMap, palette = "orange" }) {
  const group = new THREE.Group();
  group.name = "chrome";
  const materials = makeMaterials(envMap);
  materials.disc.emissiveMap = glowTexture();
  // Lights that ride with the frame: a warm key from the upper left, an amber rim from the right,
  // and the emblem's glow.
  const lights = new THREE.Group();
  const ambient = new THREE.AmbientLight(0xffffff, 0.05);
  const key = new THREE.DirectionalLight(0xfff0dc, 1.1);
  key.position.set(-4, 7, 6);
  const rim = new THREE.DirectionalLight(0xff8a3a, 0.6);
  rim.position.set(8, -2, 3);
  const orb = new THREE.PointLight(0xffa050, 1.5, 12, 1.6);
  lights.add(ambient, key, key.target, rim, rim.target, orb);
  group.add(lights);
  const parts = new THREE.Group();
  group.add(parts);
  let pieces = [];
  let interiorOpacity = 1;

  function setPalette(name) {
    const p = PALETTES[name] || PALETTES.orange;
    materials.face.color.setHex(p.face);
    materials.face.emissive.setHex(p.faceGlow);
    materials.panel.color.setHex(p.panel);
    materials.panel.emissive.setHex(p.panelGlow);
    materials.edge.color.setHex(p.edge);
    materials.edge.emissive.setHex(p.edgeGlow);
    materials.glass.color.setHex(p.glass);
    materials.glass.emissive.setHex(p.glassGlow);
    materials.ring.color.setHex(p.ring);
    materials.ring.emissive.setHex(p.ringGlow);
    materials.disc.color.setHex(p.disc);
    materials.disc.emissive.setHex(p.discGlow);
    materials.interior.color.setHex(p.interior);
    materials.pipe.color.setRGB(...p.pipe).multiplyScalar(1.9);
    materials.pipeDim.color.setRGB(...p.pipe).multiplyScalar(0.75);
    rim.color.setHex(p.rim);
    orb.color.setHex(p.light);
  }

  function setEnvironment(texture) {
    for (const material of Object.values(materials)) if ("envMap" in material && material.isMeshPhysicalMaterial) { material.envMap = texture; material.needsUpdate = true; }
  }

  function clear() {
    for (const piece of pieces) {
      piece.object.traverse((node) => { if (node.isMesh) node.geometry.dispose(); });
    }
    parts.clear();
    pieces = [];
  }

  // `from` is where a piece starts when the frame assembles in the startup (an offset in pixels and
  // a turn), `at` when it starts moving (0 to 1 of the assembly).
  function add(object, { from = [0, 0, 0], turn = [0, 0, 0], at = 0, pop = false, tag = "" } = {}) {
    parts.add(object);
    pieces.push({ object, home: object.position.clone(), from: new THREE.Vector3(...from), turn: new THREE.Euler(...turn), at: at * 0.65, pop, tag });
    return object;
  }
  let emblemSeat = null;

  // Rebuilds the chrome from frame.js's measurements. `view` is the window size in pixels and
  // `k` the world size of one pixel on the frame's plane.
  function build(m, view, k) {
    clear();
    if (!m) return;
    const ox = m.offsetX;
    const oy = m.offsetY;
    const toLocal = ([x, y]) => new THREE.Vector2((x + ox - view.width / 2) * k, (view.height / 2 - (y + oy)) * k);
    const u = m.u;
    const mesh = (geometry, material) => new THREE.Mesh(geometry, material);
    const W = view.width * k;

    // The frame body, with the interior cut out. Its rims are the thick amber bevels.
    add(mesh(slab(shapeFrom(m.outer, toLocal, [m.inner]), k, { depth: 30 * u, bevel: 13 * u, size: 11 * u, segments: 5 }), [materials.face, materials.edge]), { from: [0, 0, -60], turn: [0.05, 0, 0], at: 0 });
    // The interior behind the panels: dark, so the dome shows only around the frame.
    const interior = mesh(new THREE.ShapeGeometry(shapeFrom(m.inner, toLocal)), materials.interior);
    interior.position.z = -46 * u * k;
    interior.renderOrder = -1;
    add(interior, { at: 0 });
    // Glass inlays set into the body.
    for (const [points, from] of [[m.bar, [0, 60, 0]], [m.rightGlass, [80, 0, 0]], [m.leftGlass, [-80, 0, 0]]]) {
      const glass = mesh(slab(shapeFrom(points, toLocal), k, { depth: 4 * u, bevel: 3 * u, size: 2.5 * u, segments: 3 }), [materials.glass, materials.edge]);
      glass.position.z = 1.5 * u * k;
      add(glass, { from, at: 0.25 });
    }
    // The sidebar panel, its ears, the panel of Add game / Settings / Power, and the grid's well.
    add(mesh(slab(shapeFrom(m.side.poly, toLocal), k, { depth: 10 * u, bevel: 6 * u, size: 5 * u }), [materials.panel, materials.edge]), { from: [-420, 0, 0], at: 0.35 });
    for (const ear of m.ears) {
      add(mesh(slab(shapeFrom(ear.poly, toLocal), k, { depth: 8 * u, bevel: 4 * u, size: 3 * u, segments: 3 }), [materials.face, materials.edge]), { from: [-120, 0, 0], at: 0.45 });
    }
    if (m.group) add(mesh(slab(shapeFrom(m.group.poly, toLocal), k, { depth: 8 * u, bevel: 5 * u, size: 4 * u }), [materials.panel, materials.edge]), { from: [-420, 0, 0], at: 0.45 });
    {
      const { x, y, w, h } = m.well.box;
      const lip = 7 * u;
      const outer = window.GameStrikeFrame.chamferPoints(x - lip, y - lip, w + lip * 2, h + lip * 2, m.well.c.map((c) => c + lip * 0.4));
      add(mesh(slab(shapeFrom(outer, toLocal, [m.well.poly]), k, { depth: 8 * u, bevel: 4 * u, size: 3 * u, segments: 3 }), [materials.face, materials.edge]), { from: [0, 0, -300], at: 0.3 });
      const well = mesh(new THREE.ShapeGeometry(shapeFrom(m.well.poly, toLocal)), materials.well);
      well.position.z = -30 * u * k;
      well.renderOrder = -1;
      add(well, { at: 0.3 });
    }
    // The header tab and the two bottom panels.
    add(mesh(slab(shapeFrom(m.tab.poly, toLocal), k, { depth: 12 * u, bevel: 7 * u, size: 5.5 * u }), [materials.panel, materials.edge]), { from: [-600, 40, 0], at: 0.55 });
    if (m.titlePanel) add(mesh(slab(shapeFrom(m.titlePanel.poly, toLocal), k, { depth: 12 * u, bevel: 7 * u, size: 5.5 * u }), [materials.panel, materials.edge]), { from: [0, -200, 0], at: 0.6 });
    if (m.promptPanel) add(mesh(slab(shapeFrom(m.promptPanel.poly, toLocal), k, { depth: 12 * u, bevel: 7 * u, size: 5.5 * u }), [materials.panel, materials.edge]), { from: [300, -200, 0], at: 0.65 });

    // The emblem disc in its ring, with notches.
    if (m.emblem) {
      const r = m.emblem.r * k;
      const centre = toLocal([m.emblem.x, m.emblem.y]);
      const emblem = new THREE.Group();
      emblem.position.set(centre.x, centre.y, 4 * u * k);
      const { ring, disc } = emblemGeometry(r * 1.0);
      emblem.add(mesh(ring, materials.ring), mesh(disc, materials.disc));
      for (const [x, y, angle] of m.emblem.notches) {
        const notch = mesh(slab(new THREE.Shape([new THREE.Vector2(-10 * u * k, -7 * u * k), new THREE.Vector2(10 * u * k, -7 * u * k), new THREE.Vector2(10 * u * k, 7 * u * k), new THREE.Vector2(-10 * u * k, 7 * u * k)]), k, { depth: 6 * u, bevel: 3 * u, size: 2 * u, segments: 2 }), [materials.face, materials.edge]);
        const p = toLocal([x, y]);
        notch.position.set(p.x - centre.x, p.y - centre.y, -2 * u * k);
        notch.rotation.z = -((angle + 90) * Math.PI) / 180;
        emblem.add(notch);
      }
      add(emblem, { tag: "emblem" });
      emblemSeat = { position: emblem.position.clone(), radius: r };
      orb.position.set(centre.x + r * 0.2, centre.y - r * 0.4, r * 1.6);
      orb.distance = r * 9;
    }

    // Light pipes: the lit double edges of every panel.
    const lines = new THREE.Group();
    lines.position.z = 0.6 * u * k;
    const line = (points, width, material) => { if (points && points.length > 2) lines.add(mesh(pipe(points, width * u, toLocal), material)); };
    line(offsetPolygon(m.outer, -8), 1.6, materials.pipeDim);
    line(offsetPolygon(m.inner, 6), 2.2, materials.pipe);
    line(m.side.inner, 1.7, materials.pipe);
    if (m.group) line(m.group.inner, 1.5, materials.pipeDim);
    line(offsetPolygon(m.well.poly, 3.5), 1.6, materials.pipe);
    line(m.tab.inner, 1.7, materials.pipe);
    if (m.titlePanel) line(offsetPolygon(m.titlePanel.poly, -6), 1.6, materials.pipe);
    if (m.promptPanel) line(offsetPolygon(m.promptPanel.poly, -6), 1.6, materials.pipe);
    for (const glass of [m.bar, m.rightGlass, m.leftGlass]) line(offsetPolygon(glass, -3), 1.1, materials.pipeDim);
    add(lines, { at: 0.7 });

    // Rivets.
    const bolts = [...m.rivets, ...m.ears.flatMap((ear) => ear.rivets)];
    bolts.forEach(([x, y, r], index) => {
      const p = toLocal([x, y]);
      const { washer, head } = rivetGeometry(r * k * 1.15);
      const bolt = new THREE.Group();
      bolt.position.set(p.x, p.y, 2 * u * k);
      const slot = mesh(new THREE.BoxGeometry(r * k * 1.5, r * k * 0.28, r * k * 0.5), materials.face);
      slot.position.z = r * k * 0.62;
      slot.rotation.z = 0.6 + index;
      bolt.add(mesh(washer, materials.edge), mesh(head, materials.head), slot);
      add(bolt, { at: 0.75 + (index % 4) * 0.04, pop: true });
    });

    key.target.position.set(0, 0, -4);
    rim.target.position.set(-W * 0.2, 0, -4);
    setInterior(interiorOpacity);
  }

  function setInterior(opacity) {
    interiorOpacity = opacity;
    materials.interior.opacity = 0.95 * opacity;
    materials.well.opacity = 0.75 * Math.min(1, opacity * 1.4);
  }

  // The startup's assembly, 0 (scattered, hidden) to 1 (in place). Pieces slide in from their
  // offsets in order; rivets pop in last.
  // While the startup flies its own emblem in, the frame's emblem stays hidden (hideEmblem).
  function setAssembly(t, k, { hideEmblem = false } = {}) {
    for (const piece of pieces) {
      if (piece.tag === "emblem") { piece.object.visible = !hideEmblem; continue; }
      const local = t >= 1 ? 1 : clamp01((t - piece.at) / 0.35);
      const p = piece.pop ? easeBack(local) : easeOut(local);
      const left = 1 - p;
      piece.object.visible = local > 0;
      piece.object.position.set(piece.home.x + piece.from.x * k * left, piece.home.y + piece.from.y * k * left, piece.home.z + piece.from.z * k * left);
      piece.object.rotation.set(piece.turn.x * left, piece.turn.y * left, piece.turn.z * left);
      const s = piece.pop ? Math.max(0.001, p) : 1;
      piece.object.scale.setScalar(s);
    }
  }

  setPalette(palette);
  // The emblem's centre (in the chrome group) and radius, for the startup's flight.
  const seat = () => emblemSeat;
  return { group, materials, build, setPalette, setEnvironment, setInterior, setAssembly, seat };
}
