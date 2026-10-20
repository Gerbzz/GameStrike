// The Collection style's 3D world: the inside of a large dome whose wall carries an irregular,
// glowing net (latitude and longitude around a pole that leans toward the light, bent by a smooth
// warp), lit by a soft light near the emblem and fading into fog with distance. The frame itself is
// 3D chrome (chrome.js): thick bevelled slabs, an emblem disc, and rivets, lit and reflecting a
// small procedural environment. Big glossy amber tubes wrap around it; the ones that pass in front
// of the frame are drawn on a second, transparent canvas above the page. The detail view seats the
// selected game's cover in a cradle. Bloom lights the bright parts only. Rendered with the bundled
// three.js; decoration only.
//
// The camera and the page's CSS 3D stage share one model: the CSS perspective is set from the
// camera's field of view, and one pixel of the page is one pixel of the frame's plane, so the DOM
// (text, rows, covers), the chrome, the tubes, and the cradle stay in step while the camera drifts
// and flies between views, and the net, much further away, shows real parallax.
//
// Events from app.js:
//   "gamestrike:view"    { view, instant }    library, detail, saves, settings, about, boot
//   "gamestrike:cradle"  { id, url, canvas }  the cover the cradle holds
//   "gamestrike:variant" { variant }          orange or green
//   "gamestrike:pause"   { paused }           stop rendering while a game runs
//   "gamestrike:layout"                       the page's layout changed: rebuild the chrome
//   "gamestrike:freeze"  { from, to, progress }  a still frame partway through a flight (captures)
//   "gamestrike:boot"    { action: "play", start } | { action: "skip" } | { action: "freeze", t }
//                        the startup (boot.js): play from a performance.now() time, jump to its
//                        end, or hold second t for a capture
// Without WebGL 2 it paints a still dome on a 2D canvas (and the page draws its SVG frame); with
// reduced motion it draws one frame.
import * as THREE from "./vendor/three/three.module.min.js";
import { EffectComposer } from "./vendor/three/postprocessing/EffectComposer.js";
import { RenderPass } from "./vendor/three/postprocessing/RenderPass.js";
import { UnrealBloomPass } from "./vendor/three/postprocessing/UnrealBloomPass.js";
import { OutputPass } from "./vendor/three/postprocessing/OutputPass.js";
import { createChrome, createEnvironment } from "./chrome.js";
import { BOOT, bootPose, createBoot } from "./boot.js";

const canvas = document.getElementById("scene");
const frontCanvas = document.getElementById("scene-front");
const body = document.body;
const core = window.GameStrikeCore;
const motionQuery = window.matchMedia?.("(prefers-reduced-motion: reduce)");
const prefersReduced = () => Boolean(motionQuery?.matches);
const leaving = document.documentElement.hasAttribute("data-leaving");

const FOV = 40; // vertical field of view, degrees; the CSS perspective is derived from it
const FRAME_DEPTH = 10; // world units from the camera to the frame's plane
const DOME = { radius: 30, center: new THREE.Vector3(1.0, 2.5, -14) };
// Camera stations. Flights between them are what move the net; the frame stays with the camera.
const VIEWS = {
  boot: { position: [0, 0.4, 5.5], yaw: 0, pitch: -2, net: 0.7 },
  library: { position: [0, 0, 0], yaw: 0, pitch: 0, net: 1 },
  detail: { position: [1.1, -0.15, -1.6], yaw: -7, pitch: 0.5, net: 0.42 },
  saves: { position: [1.6, -0.25, -2.0], yaw: -10, pitch: 0.8, net: 0.36 },
  settings: { position: [-0.7, 0.25, -0.9], yaw: 5, pitch: -1, net: 0.7 },
  about: { position: [-0.4, 0.2, -1.2], yaw: 3, pitch: -0.5, net: 0.7 },
};
// Colours are written in sRGB, as sampled from the references. WebGL works in linear light and the
// output pass converts back, so the 3D code uses LINEAR; the 2D fallback uses VARIANTS directly.
const VARIANTS = {
  orange: { net: [1.0, 0.46, 0.14], glow: [1.0, 0.6, 0.2], void: [0.05, 0.016, 0.006], tube: [1.0, 0.47, 0.1], light: [1.0, 0.75, 0.45] },
  green: { net: [0.45, 0.95, 0.25], glow: [0.65, 1.0, 0.3], void: [0.012, 0.045, 0.012], tube: [0.5, 1.0, 0.2], light: [0.85, 1.0, 0.65] },
};
const toLinear = (c) => (c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);
const LINEAR = Object.fromEntries(Object.entries(VARIANTS).map(([name, colors]) => [name, Object.fromEntries(Object.entries(colors).map(([key, rgb]) => [key, rgb.map(toLinear)]))]));
const QUALITY = [
  { name: "high", pixelRatio: Math.min(window.devicePixelRatio || 1, 1.5), bloom: true, bloomScale: 1 },
  { name: "medium", pixelRatio: 1, bloom: true, bloomScale: 0.75 },
  { name: "low", pixelRatio: 0.75, bloom: true, bloomScale: 0.5 },
  { name: "minimal", pixelRatio: 0.6, bloom: false, bloomScale: 0.5 },
];

function supportsWebGL2() {
  try {
    return Boolean(document.createElement("canvas").getContext("webgl2"));
  } catch (_) {
    return false;
  }
}
function setMode(mode, quality) {
  body.dataset.scene = mode;
  if (quality) body.dataset.quality = quality;
  window.GameStrikeScene = Object.freeze({ mode, quality: quality || "static" });
}
const variantName = () => (body.dataset.variant === "green" ? "green" : "orange");
// The CSS perspective that matches the camera, in pixels.
const cssPerspective = () => innerHeight / 2 / Math.tan(THREE.MathUtils.degToRad(FOV / 2));
// The world size of one page pixel on the frame's plane.
const pixelSize = () => (2 * Math.tan(THREE.MathUtils.degToRad(FOV / 2)) * FRAME_DEPTH) / Math.max(1, innerHeight);

// Unit vectors of the dome net's pole basis, shared with core.domeNet.
function poleBasis(pole) {
  const p = new THREE.Vector3(...pole).normalize();
  const helper = Math.abs(p.y) < 0.9 ? new THREE.Vector3(0, 1, 0) : new THREE.Vector3(1, 0, 0);
  const e1 = new THREE.Vector3().crossVectors(helper, p).normalize();
  const e2 = new THREE.Vector3().crossVectors(p, e1);
  return { p, e1, e2 };
}

// ---------- shaders ----------
const SHADED_VERTEX = /* glsl */ `
  varying vec2 vUv; varying vec3 vNormal; varying vec3 vView; varying float vDepth;
  void main() {
    vUv = uv;
    vec4 world = modelMatrix * vec4(position, 1.0);
    vec4 view = viewMatrix * world;
    vNormal = normalize(normalMatrix * normal);
    vView = normalize(-view.xyz);
    vDepth = -view.z;
    gl_Position = projectionMatrix * view;
  }`;

// The dome wall: the net is computed per pixel, so lines stay thin and anti-aliased at any size.
function domeMaterial() {
  const net = core?.NET || { rings: 34, meridians: 68, warp: 0.08, pole: [-0.62, 0.42, -0.66] };
  const basis = poleBasis(net.pole);
  return new THREE.ShaderMaterial({
    side: THREE.BackSide,
    depthWrite: false,
    uniforms: {
      center: { value: DOME.center.clone() },
      pole: { value: basis.p }, e1: { value: basis.e1 }, e2: { value: basis.e2 },
      rings: { value: net.rings }, meridians: { value: net.meridians }, warp: { value: net.warp },
      spin: { value: 0 },
      netColor: { value: new THREE.Color(...LINEAR.orange.net) },
      glowColor: { value: new THREE.Color(...LINEAR.orange.glow) },
      voidColor: { value: new THREE.Color(...LINEAR.orange.void) },
      lightDir: { value: new THREE.Vector3(-0.55, 0.45, -0.7).normalize() },
      intensity: { value: 1 },
    },
    vertexShader: /* glsl */ `
      varying vec3 vWorld;
      void main() { vec4 w = modelMatrix * vec4(position, 1.0); vWorld = w.xyz; gl_Position = projectionMatrix * viewMatrix * w; }`,
    fragmentShader: /* glsl */ `
      uniform vec3 center; uniform vec3 pole; uniform vec3 e1; uniform vec3 e2;
      uniform float rings; uniform float meridians; uniform float warp; uniform float spin;
      uniform vec3 netColor; uniform vec3 glowColor; uniform vec3 voidColor; uniform vec3 lightDir; uniform float intensity;
      varying vec3 vWorld;
      const float PI = 3.14159265;
      float lineMask(float coord, float px) {
        float w = fwidth(coord);
        float d = abs(fract(coord + 0.5) - 0.5) / max(w, 1e-5);
        float core = 1.0 - smoothstep(px * 0.5, px * 0.5 + 1.0, d);
        float halo = exp(-d / (px * 3.5)) * 0.32;
        // Lines that crowd together (near the poles) fade instead of turning into noise.
        return (core + halo) * (1.0 - smoothstep(0.18, 0.45, w));
      }
      void main() {
        vec3 d = normalize(vWorld - center);
        float theta = acos(clamp(dot(d, pole), -1.0, 1.0));
        float phi = atan(dot(d, e2), dot(d, e1)) + spin;
        // The same warp as core.domeNet: irregular, quad-ish, slightly wavy cells.
        float t = theta + warp * (sin(phi * 3.0 + theta * 5.0) * 0.6 + sin(phi * 7.0 - theta * 2.0) * 0.4);
        float f = phi + warp * 1.4 * (sin(theta * 6.0 + phi * 2.0) * 0.6 + sin(theta * 3.0 - phi * 5.0) * 0.4);
        float u = f * meridians / (2.0 * PI);
        float v = t * rings / PI;
        float dist = length(vWorld - cameraPosition);
        // Thicker and brighter near the camera, thin and faint far away.
        float px = mix(1.3, 0.75, smoothstep(14.0, 44.0, dist));
        float lines = max(lineMask(u, px), lineMask(v, px));
        float light = pow(max(dot(d, lightDir), 0.0), 3.0);
        float fog = exp(-max(dist - 18.0, 0.0) * 0.035);
        vec3 color = voidColor * (0.7 + 0.6 * light) * min(1.0, intensity * 3.0);
        color += netColor * lines * (0.13 + 0.75 * light) * fog * intensity;
        color += glowColor * light * 0.06 * intensity;
        gl_FragColor = vec4(color, 1.0);
      }`,
  });
}

// The light running through a tube's core.
function energyMaterial(phase) {
  return new THREE.ShaderMaterial({
    transparent: true,
    depthWrite: false,
    blending: THREE.AdditiveBlending,
    uniforms: { tint: { value: new THREE.Color(...LINEAR.orange.glow) }, time: { value: 0 }, phase: { value: phase } },
    vertexShader: SHADED_VERTEX,
    fragmentShader: /* glsl */ `
      uniform vec3 tint; uniform float time; uniform float phase; varying vec2 vUv;
      void main() {
        float flow = fract(vUv.x * 2.0 - time * 0.12 + phase);
        float pulse = smoothstep(0.0, 0.02, flow) * (1.0 - smoothstep(0.02, 0.12, flow));
        gl_FragColor = vec4(tint * (1.1 + pulse * 2.6), 0.85 + pulse);
      }`,
  });
}
// A soft halo around the front tubes, which the bloom pass does not reach.
function haloShellMaterial() {
  return new THREE.ShaderMaterial({
    transparent: true,
    depthWrite: false,
    blending: THREE.AdditiveBlending,
    side: THREE.BackSide,
    uniforms: { tint: { value: new THREE.Color(...LINEAR.orange.glow) } },
    vertexShader: SHADED_VERTEX,
    fragmentShader: /* glsl */ `
      uniform vec3 tint; varying vec3 vNormal; varying vec3 vView;
      void main() {
        float facing = abs(dot(normalize(vNormal), normalize(vView)));
        float glow = pow(facing, 2.5) * 0.22;
        gl_FragColor = vec4(tint * glow, glow);
      }`,
  });
}
// Translucent tinted glass with a bright edge, for the cradle's plates.
function glassMaterial(strength = 1) {
  return new THREE.ShaderMaterial({
    transparent: true,
    depthWrite: false,
    side: THREE.DoubleSide,
    uniforms: { tint: { value: new THREE.Color(...LINEAR.orange.tube) }, strength: { value: strength } },
    vertexShader: SHADED_VERTEX,
    fragmentShader: /* glsl */ `
      uniform vec3 tint; uniform float strength; varying vec3 vNormal; varying vec3 vView;
      void main() {
        vec3 n = normalize(vNormal); vec3 v = normalize(vView);
        float facing = abs(dot(n, v));
        float rim = pow(1.0 - facing, 1.6);
        float spec = pow(max(dot(reflect(-v, n), normalize(vec3(-0.4, 0.85, 0.45))), 0.0), 30.0);
        // Faces catch the light from the emblem side, so flat plates read as solid glass.
        float lit = max(dot(n, normalize(vec3(-0.5, 0.7, 0.6))), 0.0);
        vec3 color = tint * (0.2 + rim * 0.85 + lit * 0.45) * strength + vec3(1.3, 1.25, 1.1) * spec * 0.8;
        gl_FragColor = vec4(color, clamp(0.42 + rim * 0.45 + lit * 0.12 + spec * 0.4, 0.0, 1.0));
      }`,
  });
}

function haloTexture() {
  const size = 256;
  const paint = document.createElement("canvas");
  paint.width = paint.height = size;
  const g = paint.getContext("2d");
  const gradient = g.createRadialGradient(size / 2, size / 2, 0, size / 2, size / 2, size / 2);
  gradient.addColorStop(0, "rgba(255,255,255,0.9)");
  gradient.addColorStop(0.15, "rgba(255,255,255,0.35)");
  gradient.addColorStop(0.45, "rgba(255,255,255,0.06)");
  gradient.addColorStop(1, "rgba(255,255,255,0)");
  g.fillStyle = gradient;
  g.fillRect(0, 0, size, size);
  const texture = new THREE.CanvasTexture(paint);
  texture.colorSpace = THREE.SRGBColorSpace;
  return texture;
}

// Tubes: points in viewport fractions (x right, y down) with a depth in world units, a radius, and
// whether they pass in front of the frame. Big amber pipes cross the frame's top right, its left
// side, and its bottom left corner in front, as in the reference; others run behind it, showing
// around its edges. Collars mark the joints.
const TUBES = [
  { points: [[0.78, -0.16, 10.2], [0.845, -0.02, 9.85], [0.93, 0.035, 9.75], [1.0, 0.07, 9.8], [1.08, 0.16, 10.1]], radius: 0.25, front: true, collars: 4 },
  { points: [[0.875, -0.16, 10.3], [0.915, -0.015, 9.95], [0.985, 0.02, 9.9], [1.08, 0.035, 10.1]], radius: 0.15, front: true, collars: 3 },
  { points: [[-0.12, 0.31, 10.3], [-0.004, 0.39, 9.85], [0.008, 0.5, 9.75], [-0.004, 0.61, 9.85], [-0.12, 0.69, 10.3]], radius: 0.2, front: true, collars: 3 },
  { points: [[-0.15, 0.62, 10.4], [-0.02, 0.73, 9.9], [0.018, 0.86, 9.75], [0.06, 0.97, 9.75], [0.085, 1.14, 10.1]], radius: 0.29, front: true, collars: 6 },
  { points: [[-0.12, 0.86, 10.3], [-0.01, 0.93, 9.9], [0.025, 1.01, 9.9], [0.03, 1.14, 10.1]], radius: 0.17, front: true, collars: 3 },
  { points: [[0.52, -0.18, 11], [0.6, -0.012, 10.7], [0.73, 0.004, 10.6], [0.84, -0.03, 10.7], [0.9, -0.2, 11]], radius: 0.19, front: false, collars: 4 },
  { points: [[1.14, 0.33, 10.8], [1.0, 0.4, 10.4], [0.99, 0.52, 10.3], [1.0, 0.64, 10.4], [1.14, 0.72, 10.8]], radius: 0.21, front: false, collars: 3 },
  { points: [[0.93, 1.16, 10.8], [0.975, 0.97, 10.4], [1.12, 0.9, 10.8]], radius: 0.18, front: false, collars: 2 },
];

// The cradle: a card carrying the cover, held by a translucent mechanical arm with glassy plates,
// bevels, and cables trailing off. Original design.
function buildCradle() {
  const group = new THREE.Group();
  const holder = new THREE.Group();
  const cardW = 1.76;
  const cardH = 2.36;
  const chamfer = (w, h, c) => {
    const s = new THREE.Shape();
    s.moveTo(-w / 2 + c, -h / 2); s.lineTo(w / 2 - c, -h / 2); s.lineTo(w / 2, -h / 2 + c); s.lineTo(w / 2, h / 2 - c);
    s.lineTo(w / 2 - c, h / 2); s.lineTo(-w / 2 + c, h / 2); s.lineTo(-w / 2, h / 2 - c); s.lineTo(-w / 2, -h / 2 + c); s.closePath();
    return s;
  };
  const plates = [];
  const glass = (mesh) => { plates.push(mesh); return mesh; };
  const extrude = (shape, depth, bevel = 0.05) => new THREE.ExtrudeGeometry(shape, { depth, bevelEnabled: true, bevelThickness: bevel, bevelSize: bevel, bevelSegments: 2 });

  // The arm: one long bevelled beam crossing behind the cover from the lower left to the upper
  // right and out of view, with a second, slimmer strut.
  const beam = glass(new THREE.Mesh(extrude(chamfer(1.15, 11, 0.32), 0.55, 0.08), glassMaterial(0.85)));
  beam.position.set(0.6, 0.2, -1.5);
  beam.rotation.set(0, 0, -0.72);
  group.add(beam);
  const strut = glass(new THREE.Mesh(extrude(chamfer(0.5, 7, 0.16), 0.3), glassMaterial(0.7)));
  strut.position.set(-1.9, -1.2, -1.1);
  strut.rotation.set(0, 0.2, 0.55);
  group.add(strut);
  // A collar ring behind the cover, where the arm holds it.
  const collar = glass(new THREE.Mesh(new THREE.TorusGeometry(1.62, 0.17, 18, 96), glassMaterial(1.1)));
  collar.position.set(0.05, 0, -0.55);
  group.add(collar);
  const collarPlate = glass(new THREE.Mesh(new THREE.CylinderGeometry(1.5, 1.5, 0.12, 72), glassMaterial(0.45)));
  collarPlate.rotation.x = Math.PI / 2;
  collarPlate.position.set(0.05, 0, -0.75);
  group.add(collarPlate);

  // The cover, on a chamfered card with a lit edge, held by four claws.
  group.add(holder);
  const frameGeo = extrude(chamfer(cardW + 0.22, cardH + 0.22, 0.2), 0.08, 0.03);
  frameGeo.translate(0, 0, -0.06);
  const edge = new THREE.MeshBasicMaterial({ color: new THREE.Color(...LINEAR.orange.glow) });
  holder.add(new THREE.Mesh(frameGeo, edge));
  const coverMaterial = new THREE.MeshBasicMaterial({ color: new THREE.Color(0.82, 0.82, 0.82) });
  const cover = new THREE.Mesh(new THREE.PlaneGeometry(cardW, cardH), coverMaterial);
  cover.position.z = 0.06;
  holder.add(cover);
  const sheen = new THREE.Mesh(new THREE.PlaneGeometry(cardW, cardH), glassMaterial(0.22));
  sheen.position.z = 0.075;
  holder.add(sheen);
  for (const [x, y, rz] of [[-1, 1, -Math.PI / 2], [1, 1, Math.PI], [1, -1, Math.PI / 2], [-1, -1, 0]]) {
    const claw = new THREE.Shape();
    claw.moveTo(0, 0); claw.lineTo(0.62, 0); claw.lineTo(0.62, 0.16); claw.lineTo(0.16, 0.16); claw.lineTo(0.16, 0.62); claw.lineTo(0, 0.62); claw.closePath();
    const mesh = glass(new THREE.Mesh(extrude(claw, 0.22, 0.03), glassMaterial(1.3)));
    mesh.position.set(x * (cardW / 2 + 0.16), y * (cardH / 2 + 0.16), -0.08);
    mesh.rotation.z = rz;
    holder.add(mesh);
  }
  // A hub where the beam meets the collar.
  const hub = glass(new THREE.Mesh(new THREE.CylinderGeometry(0.46, 0.56, 0.5, 40), glassMaterial(1.2)));
  hub.rotation.x = Math.PI / 2;
  hub.position.set(1.55, -1.25, -1.1);
  group.add(hub);

  // Cables trailing off the arm and out of view, with energy running along them.
  const cables = [];
  [
    [[[1.55, -1.25, -0.8], [2.6, -1.0, -0.4], [3.4, -2.2, 0.0], [5.6, -2.6, 0.2]], 0.05],
    [[[-1.2, 1.4, -0.7], [-2.0, 2.2, -0.6], [-1.7, 3.3, -0.3], [-0.8, 4.6, 0]], 0.04],
    [[[1.3, 1.5, -0.9], [2.1, 2.5, -0.8], [3.2, 2.6, -0.5], [4.8, 3.8, -0.3]], 0.035],
    [[[-1.6, -1.5, -0.9], [-2.6, -2.0, -0.7], [-3.3, -1.3, -0.4], [-5.0, -2.2, -0.2]], 0.04],
  ].forEach(([points, radius], index) => {
    const curve = new THREE.CatmullRomCurve3(points.map((p) => new THREE.Vector3(...p)));
    const cable = new THREE.Mesh(new THREE.TubeGeometry(curve, 90, radius, 8, false), energyMaterial(index * 0.31));
    group.add(cable);
    cables.push(cable);
  });
  return { group, holder, cover, coverMaterial, edge, plates, cables, appear: 0 };
}

function buildWorld() {
  const scene = new THREE.Scene();
  const dome = new THREE.Mesh(new THREE.SphereGeometry(DOME.radius, 96, 64), domeMaterial());
  dome.position.copy(DOME.center);
  scene.add(dome);
  // The rig follows the camera: the chrome, the tubes, the light behind the emblem, and the cradle.
  const rig = new THREE.Group();
  scene.add(rig);
  const light = new THREE.Sprite(new THREE.SpriteMaterial({ map: haloTexture(), blending: THREE.AdditiveBlending, depthWrite: false, transparent: true, color: new THREE.Color(...LINEAR.orange.light), opacity: 0.55 }));
  rig.add(light);
  const tubes = new THREE.Group();
  rig.add(tubes);
  const cradle = buildCradle();
  rig.add(cradle.group);
  return { scene, dome, rig, light, tubes, cradle };
}

// Glossy amber glass for the tubes, and dark metal for their collars: lit from the upper left like
// the chrome, with a bright rim and the reflection of a painted studio (a strip light and a
// softbox), so the tubes read as thick, polished pipes.
const STUDIO = /* glsl */ `
  vec3 studio(vec3 r) {
    float strip = exp(-pow((r.y - 0.42) / 0.07, 2.0)) * smoothstep(-0.2, 0.6, r.z);
    float box = pow(max(dot(r, normalize(vec3(-0.45, 0.6, 0.65))), 0.0), 40.0) * 3.0;
    float sky = smoothstep(-0.1, 0.9, r.y) * 0.25;
    return vec3(strip * 1.6 + box + sky);
  }`;
function tubeGlassMaterial() {
  return new THREE.ShaderMaterial({
    transparent: true,
    depthWrite: false,
    uniforms: { tint: { value: new THREE.Color(...LINEAR.orange.tube) } },
    vertexShader: SHADED_VERTEX,
    fragmentShader: /* glsl */ `
      uniform vec3 tint; varying vec3 vNormal; varying vec3 vView;
      ${STUDIO}
      void main() {
        vec3 n = normalize(vNormal); vec3 v = normalize(vView);
        float facing = abs(dot(n, v));
        float rim = pow(1.0 - facing, 2.0);
        float lit = max(dot(n, normalize(vec3(-0.35, 0.9, 0.35))), 0.0);
        // Clear, slightly smoked glass: the glowing core shows through the middle, the walls
        // light up toward the silhouette, and the studio leaves sharp highlights.
        vec3 body = tint * (0.05 + 0.3 * lit);
        vec3 color = mix(body, tint * 1.7, smoothstep(0.15, 0.95, rim));
        vec3 r = reflect(-v, n);
        vec3 shine = studio(r) * (0.4 + 0.6 * rim);
        color += mix(tint, vec3(1.0, 0.96, 0.88), 0.7) * shine;
        float alpha = clamp(0.28 + rim * 0.62 + length(shine) * 0.45, 0.0, 1.0);
        gl_FragColor = vec4(color, alpha);
      }`,
  });
}
function tubeCollarMaterial() {
  return new THREE.ShaderMaterial({
    uniforms: { tint: { value: new THREE.Color(...LINEAR.orange.tube) } },
    vertexShader: SHADED_VERTEX,
    fragmentShader: /* glsl */ `
      uniform vec3 tint; varying vec3 vNormal; varying vec3 vView;
      ${STUDIO}
      void main() {
        vec3 n = normalize(vNormal); vec3 v = normalize(vView);
        float lit = max(dot(n, normalize(vec3(-0.35, 0.9, 0.35))), 0.0);
        vec3 r = reflect(-v, n);
        float rim = pow(1.0 - abs(dot(n, v)), 2.0);
        vec3 color = tint * (0.12 + 0.6 * lit + 1.1 * rim) + mix(tint, vec3(1.0), 0.6) * studio(r);
        gl_FragColor = vec4(color, 1.0);
      }`,
  });
}
function tubeMaterials() {
  return { glass: tubeGlassMaterial(), collar: tubeCollarMaterial() };
}

function startWebGL() {
  let qualityLevel = 0;
  const renderer = new THREE.WebGLRenderer({ canvas, antialias: true, alpha: false, powerPreference: "high-performance", preserveDrawingBuffer: true });
  renderer.toneMapping = THREE.NoToneMapping;
  const world = buildWorld();
  const camera = new THREE.PerspectiveCamera(FOV, 1, 0.1, 200);
  world.scene.add(camera);
  const composer = new EffectComposer(renderer);
  composer.addPass(new RenderPass(world.scene, camera));
  // Only bright, lit parts pass the threshold: net lines near the light, tube and bevel highlights, the cradle's edge.
  const bloom = new UnrealBloomPass(new THREE.Vector2(1, 1), 0.55, 0.4, 0.84);
  composer.addPass(bloom);
  composer.addPass(new OutputPass());

  // The chrome and its environment.
  let envMap = createEnvironment(renderer, LINEAR[variantName()].tube);
  const chrome = createChrome({ envMap, palette: variantName() });
  chrome.group.position.z = -FRAME_DEPTH;
  world.rig.add(chrome.group);
  const backTubeMaterials = tubeMaterials();

  // The front layer: its own renderer, scene, and rig, kept in step with the main rig.
  let front = null;
  if (frontCanvas) {
    try {
      const frontRenderer = new THREE.WebGLRenderer({ canvas: frontCanvas, antialias: true, alpha: true, premultipliedAlpha: true, preserveDrawingBuffer: true });
      frontRenderer.setClearColor(0x000000, 0);
      frontRenderer.toneMapping = THREE.NoToneMapping;
      const frontScene = new THREE.Scene();
      const rig = new THREE.Group();
      frontScene.add(rig);
      const tubes = new THREE.Group();
      rig.add(tubes);
      front = { renderer: frontRenderer, scene: frontScene, rig, tubes, materials: tubeMaterials() };
    } catch (_) {
      front = null;
    }
  }
  body.dataset.chrome = "3d";

  // The startup: its own objects in the world and in the rig; null start means not playing.
  // The emblem and the flash fly above the page (the front layer, when there is one), so the covers
  // that flip in during the assembly never hide them; the particles and the core stay in the dome.
  const bootEnv = front ? createEnvironment(front.renderer, LINEAR[variantName()].tube) : envMap;
  const boot = createBoot({
    tint: LINEAR[variantName()].glow,
    envMap: bootEnv,
    green: variantName() === "green",
    onLogo: () => { body.dataset.bootLogo = "ready"; if (!running && (frozen || bootHold !== null)) renderFrame(0); },
  });
  boot.setTint(LINEAR[variantName()].glow, variantName() === "green");
  world.scene.add(boot.world);
  (front ? front.rig : world.rig).add(boot.rig);
  let bootStart = null;
  let bootHold = null;
  let bootTime = BOOT.end;

  const station = (name) => VIEWS[name] || VIEWS.library;
  const pos = new THREE.Vector3(...station("boot").position);
  const goalPos = pos.clone();
  let yaw = station("boot").yaw;
  let pitch = station("boot").pitch;
  let goalYaw = yaw;
  let goalPitch = pitch;
  let net = station("boot").net;
  let goalNet = net;
  let view = "boot";
  let paused = false;
  let running = false;
  let frozen = false;
  let last = performance.now();
  let clock = 0;
  let frames = 0;
  let frameTime = 0;
  let interior = 1;
  const stage = document.getElementById("frame");
  const capturing = () => body.classList.contains("reduce-motion");
  const still = () => prefersReduced() || capturing() || frozen;
  const booting = () => bootStart !== null && bootTime < BOOT.end;

  function applyVariant() {
    const name = variantName();
    const v = LINEAR[name];
    const u = world.dome.material.uniforms;
    u.netColor.value.setRGB(...v.net);
    u.glowColor.value.setRGB(...v.glow);
    u.voidColor.value.setRGB(...v.void);
    world.light.material.color.setRGB(...v.light);
    for (const mesh of world.cradle.plates) mesh.material.uniforms.tint.value.setRGB(...v.tube);
    for (const mesh of world.cradle.cables) mesh.material.uniforms.tint.value.setRGB(...v.glow);
    world.cradle.edge.color.setRGB(...v.glow).multiplyScalar(0.9);
    renderer.setClearColor(new THREE.Color(...v.void));
    chrome.setPalette(name);
    for (const set of [backTubeMaterials, front?.materials].filter(Boolean)) {
      set.glass.uniforms.tint.value.setRGB(...v.tube);
      set.collar.uniforms.tint.value.setRGB(...v.tube);
    }
    for (const group of [world.tubes, front?.tubes].filter(Boolean)) {
      group.traverse((mesh) => { if (mesh.material?.uniforms?.tint) mesh.material.uniforms.tint.value.setRGB(...v.glow); });
    }
  }
  // The environment carries the variant's colour; it is painted again when the variant changes.
  function applyEnvironment() {
    const tint = LINEAR[variantName()].tube;
    const next = createEnvironment(renderer, tint);
    chrome.setEnvironment(next);
    envMap.dispose();
    envMap = next;
  }

  // A point given in viewport fractions at a depth in front of the camera (camera space).
  function viewportPoint(x, y, depth) {
    const half = Math.tan(THREE.MathUtils.degToRad(FOV / 2)) * depth;
    return new THREE.Vector3((x * 2 - 1) * half * camera.aspect, (1 - y * 2) * half, -depth);
  }

  function disposeGroup(group) {
    group.traverse((node) => { if (node.isMesh) { node.geometry.dispose(); if (node.material?.isShaderMaterial) node.material.dispose(); } });
    group.clear();
  }

  function buildTubes() {
    disposeGroup(world.tubes);
    if (front) disposeGroup(front.tubes);
    const v = LINEAR[variantName()];
    TUBES.forEach(({ points, radius, front: inFront, collars }, index) => {
      const target = inFront && front ? front.tubes : world.tubes;
      const materials = inFront && front ? front.materials : backTubeMaterials;
      const curve = new THREE.CatmullRomCurve3(points.map(([x, y, d]) => viewportPoint(x, y, d)));
      const energy = new THREE.Mesh(new THREE.TubeGeometry(curve, 120, radius * 0.3, 10, false), energyMaterial(index * 0.23));
      energy.userData.extend = 8;
      energy.material.uniforms.tint.value.setRGB(...v.glow);
      energy.renderOrder = 1;
      const glass = new THREE.Mesh(new THREE.TubeGeometry(curve, 160, radius, 32, false), materials.glass);
      glass.userData.extend = 32;
      glass.renderOrder = 2;
      target.add(energy, glass);
      if (inFront && front) {
        const halo = new THREE.Mesh(new THREE.TubeGeometry(curve, 80, radius * 1.7, 16, false), haloShellMaterial());
        halo.userData.extend = 16;
        halo.material.uniforms.tint.value.setRGB(...v.glow);
        halo.renderOrder = 0;
        target.add(halo);
      }
      // Collars at the joints, square to the tube.
      const ring = new THREE.TorusGeometry(radius * 1.04, radius * 0.16, 12, 40);
      for (let i = 1; i <= collars; i += 1) {
        const t = i / (collars + 1);
        const collar = new THREE.Mesh(ring.clone(), materials.collar);
        collar.position.copy(curve.getPointAt(t));
        collar.quaternion.setFromUnitVectors(new THREE.Vector3(0, 0, 1), curve.getTangentAt(t));
        collar.renderOrder = 3;
        collar.userData.at = t;
        target.add(collar);
      }
      ring.dispose();
    });
    // The light sits behind the emblem, top left.
    world.light.position.copy(viewportPoint(0.09, 0.09, FRAME_DEPTH + 2));
    world.light.scale.setScalar(9);
    // The cradle sits in the right part of the detail view, smaller in narrower windows.
    world.cradle.group.position.copy(viewportPoint(0.54, 0.39, 8.5));
    world.cradle.base = THREE.MathUtils.clamp(camera.aspect / 1.78, 0.72, 1.05);
  }

  function buildChrome() {
    chrome.build(window.GameStrikeFrame?.measure(stage), { width: innerWidth, height: innerHeight }, pixelSize());
  }

  function applyQuality() {
    const quality = QUALITY[qualityLevel];
    renderer.setPixelRatio(quality.pixelRatio);
    composer.setPixelRatio(quality.pixelRatio);
    bloom.enabled = quality.bloom;
    bloom.resolution.set(innerWidth * quality.bloomScale, innerHeight * quality.bloomScale);
    chrome.setLean(qualityLevel >= 2);
    if (front) {
      front.renderer.setPixelRatio(quality.pixelRatio);
      front.renderer.setSize(innerWidth, innerHeight, false);
    }
    setMode("webgl", quality.name);
  }

  function resize() {
    renderer.setSize(innerWidth, innerHeight, false);
    composer.setSize(innerWidth, innerHeight);
    camera.aspect = innerWidth / Math.max(1, innerHeight);
    camera.updateProjectionMatrix();
    document.documentElement.style.setProperty("--persp", `${cssPerspective().toFixed(1)}px`);
    buildTubes();
    buildChrome();
    if (bootStart !== null || bootHold !== null) poseBoot(bootHold ?? (performance.now() - bootStart) / 1000);
    applyQuality();
    if (!running) renderFrame(0);
  }

  // The frame drifts a little with the camera, and the tubes with it.
  function drift(t) {
    if (still()) return { yaw: 0, pitch: 0, x: 0, y: 0 };
    return { yaw: Math.sin(t * 0.21) * 0.55, pitch: Math.sin(t * 0.17 + 1.3) * 0.35, x: Math.sin(t * 0.13) * 0.06, y: Math.sin(t * 0.11 + 0.7) * 0.04 };
  }

  function placeCamera(p, cameraYaw, cameraPitch, wobble) {
    camera.position.copy(p).add(new THREE.Vector3(wobble.x, wobble.y, 0));
    camera.rotation.set(THREE.MathUtils.degToRad(cameraPitch), THREE.MathUtils.degToRad(cameraYaw), 0, "YXZ");
    camera.updateMatrixWorld();
    // The rig rides with the camera, tilted about the frame's centre by the same wobble as the CSS frame.
    world.rig.position.copy(camera.position);
    world.rig.quaternion.copy(camera.quaternion);
    world.rig.translateZ(-FRAME_DEPTH);
    world.rig.rotateY(THREE.MathUtils.degToRad(wobble.yaw));
    world.rig.rotateX(THREE.MathUtils.degToRad(wobble.pitch));
    world.rig.translateZ(FRAME_DEPTH);
    if (front) {
      front.rig.position.copy(world.rig.position);
      front.rig.quaternion.copy(world.rig.quaternion);
    }
    if (stage && !body.classList.contains("flat")) {
      stage.style.transform = wobble.yaw || wobble.pitch ? `rotateY(${(-wobble.yaw).toFixed(3)}deg) rotateX(${wobble.pitch.toFixed(3)}deg)` : "";
    }
  }

  function poseCradle(a, sway) {
    const c = world.cradle;
    c.group.visible = a > 0.01;
    // Flies in from deep in the dome, turning to face the camera.
    c.group.scale.setScalar((c.base || 1) * (0.55 + 0.45 * a));
    c.group.position.z = -8.5 - (1 - a) * 9;
    c.group.rotation.set(-0.06 + (1 - a) * 0.2, -0.32 + (1 - a) * 0.9 + sway, 0.02);
  }

  function updateCradle(dt, target) {
    const c = world.cradle;
    c.appear = dt > 0 && !still() ? c.appear + (target - c.appear) * (1 - Math.exp(-dt * 4)) : target;
    poseCradle(c.appear, still() ? 0 : Math.sin(clock * 0.6) * 0.12);
    c.holder.rotation.y = still() ? 0.04 : Math.sin(clock * 0.9) * 0.05;
  }

  // Tubes grow from their far ends (0 to 1), collars appearing as the tube passes them.
  function extendTubes(p) {
    for (const group of [world.tubes, front?.tubes]) {
      group?.children.forEach((mesh) => {
        if (mesh.userData.extend) {
          const index = mesh.geometry.index;
          const per = mesh.userData.extend * 6;
          mesh.geometry.setDrawRange(0, p >= 1 ? Infinity : Math.floor(((index ? index.count : 0) / per) * p) * per);
          mesh.visible = p > 0;
        } else if (mesh.userData.at !== undefined) {
          mesh.visible = p >= mesh.userData.at;
        }
      });
    }
  }

  // The startup at second t: the camera's path, the net, the frame assembling, the tubes, the
  // emblem's flight from the core to its seat. Past the end, everything is at rest.
  function poseBoot(t) {
    bootTime = t;
    const k = pixelSize();
    if (t >= BOOT.end) {
      boot.world.visible = false;
      boot.rig.visible = false;
      chrome.setAssembly(1, k);
      extendTubes(1);
      world.light.visible = true;
      return false;
    }
    const pose = bootPose(t);
    pos.copy(pose.position);
    yaw = 0;
    pitch = pose.pitch;
    net = pose.net;
    placeCamera(pos, yaw, pitch, { yaw: 0, pitch: 0, x: 0, y: 0 });
    world.dome.material.uniforms.intensity.value = net;
    chrome.setAssembly(pose.assembly, k, { hideEmblem: t < 5.0 });
    extendTubes(Math.min(1, Math.max(0, (t - 4.45) / 0.9)));
    world.light.visible = t > 4.6;
    world.cradle.group.visible = false;
    boot.rig.visible = true;
    const seat = chrome.seat();
    boot.update(t, {
      start: { position: viewportPoint(0.5, 0.4, 7), radius: 0.85 },
      seat: seat ? { position: seat.position.clone().add(chrome.group.position), radius: seat.radius } : { position: viewportPoint(0.09, 0.09, FRAME_DEPTH), radius: 0.5 },
    });
    return true;
  }

  // The frame's interior turns translucent in the detail view, so the cradle and the dimmer dome show through.
  function interiorTarget() {
    if (view !== "detail" && view !== "saves") return 1;
    return body.classList.contains("mid-move") ? 0.62 : 0.42;
  }

  function render() {
    composer.render();
    if (front) front.renderer.render(front.scene, camera);
  }

  function renderFrame(dt) {
    clock += dt;
    if (bootHold !== null || bootStart !== null) {
      const t = bootHold !== null ? bootHold : (performance.now() - bootStart) / 1000;
      if (poseBoot(t)) {
        const time = (group) => group?.traverse((mesh) => { if (mesh.material?.uniforms?.time) mesh.material.uniforms.time.value = clock; });
        time(world.tubes);
        time(front?.tubes);
        render();
        if (bootHold !== null) body.dataset.bootFrame = String(bootHold);
        return;
      }
      bootStart = null;
      if (bootHold !== null) body.dataset.bootFrame = String(bootHold);
      bootHold = null;
    }
    const ease = dt > 0 && !still() ? 1 - Math.exp(-dt * 2.4) : 1;
    pos.lerp(goalPos, ease);
    yaw += (goalYaw - yaw) * ease;
    pitch += (goalPitch - pitch) * ease;
    net += (goalNet - net) * ease;
    placeCamera(pos, yaw, pitch, drift(clock));
    const u = world.dome.material.uniforms;
    u.intensity.value = net;
    u.spin.value = still() ? 0 : clock * 0.006;
    const time = (group) => group?.traverse((mesh) => { if (mesh.material?.uniforms?.time) mesh.material.uniforms.time.value = clock; });
    time(world.tubes);
    time(front?.tubes);
    for (const mesh of world.cradle.cables) mesh.material.uniforms.time.value = clock;
    interior += (interiorTarget() - interior) * (dt > 0 && !still() ? 1 - Math.exp(-dt * 3) : 1);
    chrome.setInterior(interior);
    updateCradle(dt, view === "detail" || view === "saves" ? 1 : 0);
    render();
  }

  function loop(now) {
    if (!running) return;
    const dt = Math.min(0.1, (now - last) / 1000);
    last = now;
    renderFrame(dt);
    frameTime += dt * 1000;
    frames += 1;
    if (frames >= 90) {
      const average = frameTime / frames;
      if (average > 22 && qualityLevel < QUALITY.length - 1) { qualityLevel += 1; applyQuality(); }
      frames = 0;
      frameTime = 0;
    }
    requestAnimationFrame(loop);
  }

  function setRunning() {
    body.classList.toggle("flat", prefersReduced());
    const shouldRun = !paused && !document.hidden && (!still() || booting());
    if (shouldRun && !running) {
      running = true;
      last = performance.now();
      requestAnimationFrame(loop);
    } else if (!shouldRun && running) {
      running = false;
    }
    if (!running && !frozen) renderFrame(0);
  }

  function fly(name, instant) {
    if (frozen) return;
    view = name in VIEWS ? name : "library";
    const s = station(view);
    goalPos.set(...s.position);
    goalYaw = s.yaw;
    goalPitch = s.pitch;
    goalNet = s.net;
    if (instant || still()) {
      pos.copy(goalPos);
      yaw = goalYaw;
      pitch = goalPitch;
      net = goalNet;
    }
    setRunning();
  }

  // The cover the cradle holds. The generated box art goes in at once, so the card is never blank;
  // the stored image then replaces it, read through a plain same-origin <img> and copied to a canvas
  // (no CORS request, which WebKit refuses for the launcher's custom scheme). If the copy cannot be
  // read back, the box art stays. body.dataset.cradle records which one is shown, for captures.
  let coverRequest = 0;
  function setCover(detail) {
    const material = world.cradle.coverMaterial;
    const request = ++coverRequest;
    const apply = (source, kind) => {
      if (request !== coverRequest) return;
      const texture = new THREE.CanvasTexture(source);
      texture.colorSpace = THREE.SRGBColorSpace;
      texture.anisotropy = 4;
      material.map?.dispose();
      material.map = texture;
      material.needsUpdate = true;
      body.dataset.cradle = kind;
      if (!running && !frozen) renderFrame(0);
    };
    if (detail?.canvas) apply(detail.canvas, detail.url ? "loading" : "generated");
    if (!detail?.url) { body.dataset.cradle = detail?.canvas ? "generated" : "empty"; return; }
    const settle = (reason) => { if (request === coverRequest) body.dataset.cradle = `generated (${reason})`; };
    const image = new Image();
    image.decoding = "async";
    image.onload = () => {
      try {
        const copy = document.createElement("canvas");
        copy.width = Math.min(1024, image.naturalWidth || 300);
        copy.height = Math.min(1536, image.naturalHeight || 450);
        const g = copy.getContext("2d");
        g.drawImage(image, 0, 0, copy.width, copy.height);
        copy.toDataURL(); // throws if the copy is not readable, which WebGL would refuse too
        apply(copy, "image");
      } catch (error) {
        settle(error && error.name ? error.name : "unreadable");
      }
    };
    image.onerror = () => settle("load error");
    image.src = detail.url;
  }

  window.addEventListener("resize", resize);
  window.addEventListener("gamestrike:view", (event) => fly(event.detail?.view, Boolean(event.detail?.instant)));
  window.addEventListener("gamestrike:pause", (event) => { paused = Boolean(event.detail?.paused); setRunning(); });
  window.addEventListener("gamestrike:variant", () => { applyEnvironment(); applyVariant(); boot.setTint(LINEAR[variantName()].glow, variantName() === "green"); if (!running) renderFrame(0); });
  window.addEventListener("gamestrike:cradle", (event) => setCover(event.detail));
  window.addEventListener("gamestrike:layout", () => { buildChrome(); poseBoot(bootStart !== null ? (performance.now() - bootStart) / 1000 : bootHold ?? BOOT.end); if (!running && !frozen) renderFrame(0); });
  function onBoot(detail) {
    const action = detail?.action;
    if (action === "play" && !prefersReduced()) {
      bootStart = typeof detail.start === "number" ? detail.start : performance.now();
      bootHold = null;
      poseBoot(0);
      setRunning();
    } else if (action === "freeze") {
      const t = Math.max(0, Number(detail.t) || 0);
      bootStart = null;
      running = false;
      if (t >= BOOT.end) {
        // The end of the startup is the library at rest.
        bootHold = null;
        poseBoot(BOOT.end);
        frozen = false;
        fly("library", true);
        frozen = true;
        renderFrame(0);
        body.dataset.bootFrame = String(detail.t);
        return;
      }
      bootHold = t;
      frozen = true;
      renderFrame(0);
    } else {
      bootStart = null;
      bootHold = null;
      poseBoot(BOOT.end);
      fly(view === "boot" ? "library" : view, true);
      if (!running && !frozen) renderFrame(0);
    }
  }
  window.addEventListener("gamestrike:boot", (event) => onBoot(event.detail));
  window.addEventListener("gamestrike:freeze", (event) => {
    // A still frame partway through a flight, for the capture of a camera move.
    const { from = "library", to = "detail", progress = 0.5 } = event.detail || {};
    frozen = true;
    running = false;
    const a = station(from);
    const b = station(to);
    const k = progress * progress * (3 - 2 * progress);
    view = to;
    pos.set(...a.position).lerp(new THREE.Vector3(...b.position), k);
    goalPos.copy(pos);
    yaw = goalYaw = a.yaw + (b.yaw - a.yaw) * k;
    pitch = goalPitch = a.pitch + (b.pitch - a.pitch) * k;
    net = goalNet = a.net + (b.net - a.net) * k;
    world.cradle.appear = k;
    placeCamera(pos, yaw, pitch, { yaw: 0, pitch: 0, x: 0, y: 0 });
    world.dome.material.uniforms.intensity.value = net;
    poseCradle(k, 0);
    interior = 1 + (0.62 - 1) * k;
    chrome.setInterior(interior);
    render();
  });
  document.addEventListener("visibilitychange", setRunning);
  motionQuery?.addEventListener?.("change", () => { fly(view, true); setRunning(); });
  canvas.addEventListener("webglcontextlost", (event) => { event.preventDefault(); running = false; startStatic("context lost"); }, { once: true });

  applyVariant();
  resize();
  fly(body.dataset.view || "boot", true);
  // Anything the page asked for before the scene was running.
  const pending = window.GameStrikeScenePending || {};
  if (pending.cradle) setCover(pending.cradle);
  if (pending.boot) onBoot(pending.boot);
  else poseBoot(BOOT.end);
  setRunning();
  if (pending.freeze) window.dispatchEvent(new CustomEvent("gamestrike:freeze", { detail: pending.freeze }));
}

// ---------- the still dome, for machines without WebGL 2 ----------
// Projects core.domeNet's lines with the same camera and paints them with a soft glow, the tubes as
// glossy strokes (the ones in front of the frame on the front canvas), and the light near the
// emblem. The page draws its SVG frame over it.
function startStatic(reason) {
  const fresh = (id, className) => {
    const node = document.createElement("canvas");
    node.id = id;
    node.className = className;
    node.setAttribute("aria-hidden", "true");
    document.getElementById(id)?.replaceWith(node);
    return node;
  };
  const fallback = fresh("scene", "scene");
  const frontLayer = document.getElementById("scene-front") ? fresh("scene-front", "scene-front") : null;
  const g = fallback.getContext("2d");
  const gf = frontLayer?.getContext("2d") || null;
  body.classList.add("flat");
  delete body.dataset.chrome;
  const frame = document.getElementById("frame");
  if (frame) frame.style.transform = "";
  if (!g || !core) { setMode("none"); return; }
  const lines = core.domeNet({ steps: 96 });
  function paint() {
    const ratio = Math.min(window.devicePixelRatio || 1, 1.5);
    const width = innerWidth;
    const height = innerHeight;
    for (const [node, ctx] of [[fallback, g], [frontLayer, gf]]) {
      if (!node || !ctx) continue;
      node.width = Math.round(width * ratio);
      node.height = Math.round(height * ratio);
      ctx.setTransform(ratio, 0, 0, ratio, 0, 0);
    }
    const v = VARIANTS[variantName()];
    const rgb = (c, k = 1, a = 1) => `rgba(${Math.min(255, Math.round(c[0] * 255 * k))},${Math.min(255, Math.round(c[1] * 255 * k))},${Math.min(255, Math.round(c[2] * 255 * k))},${a})`;
    g.fillStyle = rgb(v.void, 1.6);
    g.fillRect(0, 0, width, height);
    const focal = height / 2 / Math.tan((FOV * Math.PI) / 360);
    const project = ([x, y, z]) => {
      const wx = DOME.center.x + x * DOME.radius;
      const wy = DOME.center.y + y * DOME.radius;
      const wz = DOME.center.z + z * DOME.radius;
      if (wz > -0.5) return null;
      return [width / 2 + (wx / -wz) * focal, height / 2 - (wy / -wz) * focal, Math.hypot(wx, wy, wz), [x, y, z]];
    };
    const light = new THREE.Vector3(-0.55, 0.45, -0.7).normalize();
    g.lineCap = "round";
    for (const [lineWidth, strength] of [[3.2, 0.12], [1.1, 0.75]]) {
      for (const line of lines) {
        let prev = null;
        for (const point of line) {
          const p = project(point);
          if (p && prev) {
            const fog = Math.exp(-Math.max(p[2] - 9, 0) * 0.075);
            const lit = Math.pow(Math.max(p[3][0] * light.x + p[3][1] * light.y + p[3][2] * light.z, 0), 3);
            g.strokeStyle = rgb(v.net, 1, Math.min(1, (0.26 + 1.25 * lit) * fog * strength));
            g.lineWidth = lineWidth * (p[2] < 14 ? 1.4 : 1);
            g.beginPath();
            g.moveTo(prev[0], prev[1]);
            g.lineTo(p[0], p[1]);
            g.stroke();
          }
          prev = p;
        }
      }
    }
    const halo = g.createRadialGradient(width * 0.09, height * 0.09, 0, width * 0.09, height * 0.09, height * 0.55);
    halo.addColorStop(0, rgb(v.light, 1, 0.35));
    halo.addColorStop(1, rgb(v.light, 1, 0));
    g.fillStyle = halo;
    g.fillRect(0, 0, width, height);
    for (const { points, radius, front } of TUBES) {
      const ctx = front && gf ? gf : g;
      const pts = points.map(([x, y]) => [x * width, y * height]);
      const px = (radius / FRAME_DEPTH) * focal;
      for (const [w, color] of [[px * 2, rgb(v.tube, 0.35, 0.75)], [px * 1.5, rgb(v.tube, 0.85, 0.7)], [px * 0.6, rgb(v.glow, 1.1, 0.6)], [px * 0.2, "rgba(255,240,220,0.7)"]]) {
        ctx.beginPath();
        ctx.moveTo(...pts[0]);
        for (let i = 1; i < pts.length - 1; i += 1) ctx.quadraticCurveTo(pts[i][0], pts[i][1], (pts[i][0] + pts[i + 1][0]) / 2, (pts[i][1] + pts[i + 1][1]) / 2);
        ctx.lineTo(...pts[pts.length - 1]);
        ctx.strokeStyle = color;
        ctx.lineWidth = w;
        ctx.lineCap = "round";
        ctx.stroke();
      }
    }
  }
  paint();
  window.addEventListener("resize", paint);
  window.addEventListener("gamestrike:variant", paint);
  setMode("static", "static");
  body.dataset.sceneReason = reason;
}

if (leaving || !canvas) {
  setMode("none");
} else if (!supportsWebGL2()) {
  startStatic("webgl2 unavailable");
} else {
  try {
    startWebGL();
  } catch (error) {
    startStatic(`webgl failed: ${error && error.message ? error.message : error}`);
  }
}
