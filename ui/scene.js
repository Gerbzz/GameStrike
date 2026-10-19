// GameStrike's 3D backdrop: a deep green void with a pulsing core, translucent energy conduits,
// drifting wisps, and bloom, rendered with the bundled three.js. It is decoration only. It never
// sees game content, and it talks to the launcher page only through two DOM events:
//   "gamestrike:view"  { view, instant }  where the camera should fly (library, detail, saves, settings, about, boot)
//   "gamestrike:pause" { paused }         stop rendering while a game runs
// If WebGL 2 is unavailable it paints a static 2D version of the same scene instead, and the
// launcher stays fully usable either way. Reduced motion renders single still frames.
import * as THREE from "./vendor/three/three.module.min.js";
import { EffectComposer } from "./vendor/three/postprocessing/EffectComposer.js";
import { RenderPass } from "./vendor/three/postprocessing/RenderPass.js";
import { UnrealBloomPass } from "./vendor/three/postprocessing/UnrealBloomPass.js";
import { OutputPass } from "./vendor/three/postprocessing/OutputPass.js";

const canvas = document.getElementById("scene");
const body = document.body;
const motionQuery = window.matchMedia?.("(prefers-reduced-motion: reduce)");
const reducedMotion = () => Boolean(motionQuery?.matches) || body.classList.contains("reduce-motion");

const ORB = new THREE.Vector3(2.7, 0.25, -1.6);
// Camera stations. Moving between views is a fly-through between these.
const VIEWS = {
  boot: { position: [3.1, 0.45, 4.4], target: [2.7, 0.25, -1.6] },
  library: { position: [-0.4, 0.6, 10.5], target: [-0.4, 0.1, -1.6] },
  detail: { position: [6.4, 1.5, 5.2], target: [1.6, 0.2, -1.8] },
  saves: { position: [1.0, -0.6, 6.4], target: [2.3, 0.1, -1.6] },
  settings: { position: [-6.2, 1.8, 4.6], target: [-0.6, 0.3, -2.4] },
  about: { position: [4.2, 0.6, 3.6], target: [2.7, 0.25, -1.6] },
};
// Quality steps; the renderer drops a step whenever frames take too long.
const QUALITY = [
  { name: "high", pixelRatio: Math.min(window.devicePixelRatio || 1, 1.5), particles: 1600, bloom: true, bloomScale: 1 },
  { name: "medium", pixelRatio: 1, particles: 900, bloom: true, bloomScale: 0.75 },
  { name: "low", pixelRatio: 0.75, particles: 450, bloom: true, bloomScale: 0.5 },
  { name: "minimal", pixelRatio: 0.6, particles: 200, bloom: false, bloomScale: 0.5 },
];

function supportsWebGL2() {
  try {
    const probe = document.createElement("canvas");
    return Boolean(probe.getContext("webgl2"));
  } catch (_) {
    return false;
  }
}

function setMode(mode, quality) {
  body.dataset.scene = mode;
  if (quality) body.dataset.quality = quality;
  window.GameStrikeScene = Object.freeze({ mode, quality: quality || "static" });
}

// ---------- shaders ----------
const NOISE = /* glsl */ `
  float hash(vec3 p) { p = fract(p * 0.3183099 + 0.1); p *= 17.0; return fract(p.x * p.y * p.z * (p.x + p.y + p.z)); }
  float noise(vec3 x) {
    vec3 i = floor(x); vec3 f = fract(x); f = f * f * (3.0 - 2.0 * f);
    return mix(mix(mix(hash(i), hash(i + vec3(1,0,0)), f.x), mix(hash(i + vec3(0,1,0)), hash(i + vec3(1,1,0)), f.x), f.y),
               mix(mix(hash(i + vec3(0,0,1)), hash(i + vec3(1,0,1)), f.x), mix(hash(i + vec3(0,1,1)), hash(i + vec3(1,1,1)), f.x), f.y), f.z);
  }
  float fbm(vec3 p) { float v = 0.0; float a = 0.5; for (int i = 0; i < 4; i++) { v += a * noise(p); p *= 2.03; a *= 0.5; } return v; }
`;

function backgroundMaterial() {
  return new THREE.ShaderMaterial({
    side: THREE.BackSide,
    depthWrite: false,
    uniforms: { time: { value: 0 } },
    vertexShader: /* glsl */ `
      varying vec3 vDir;
      void main() { vDir = normalize(position); gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0); }`,
    fragmentShader: /* glsl */ `
      uniform float time; varying vec3 vDir; ${NOISE}
      void main() {
        float horizon = smoothstep(-0.6, 0.9, vDir.y);
        vec3 deep = mix(vec3(0.001, 0.003, 0.0015), vec3(0.003, 0.014, 0.004), 1.0 - horizon);
        float cloud = fbm(vDir * 2.6 + vec3(0.0, time * 0.01, time * 0.006));
        float wisps = smoothstep(0.52, 0.86, cloud);
        vec3 nebula = vec3(0.012, 0.06, 0.01) * wisps * (0.6 + 0.4 * (1.0 - horizon));
        float glow = pow(max(dot(vDir, normalize(vec3(0.3, 0.05, -1.0))), 0.0), 18.0);
        gl_FragColor = vec4(deep + nebula + vec3(0.01, 0.05, 0.008) * glow, 1.0);
      }`,
  });
}

function coreMaterial() {
  return new THREE.ShaderMaterial({
    uniforms: { time: { value: 0 } },
    vertexShader: /* glsl */ `
      varying vec3 vNormal; varying vec3 vView; varying vec3 vPos;
      void main() {
        vec4 world = modelMatrix * vec4(position, 1.0);
        vPos = position; vNormal = normalize(normalMatrix * normal);
        vView = normalize(-(viewMatrix * world).xyz);
        gl_Position = projectionMatrix * viewMatrix * world;
      }`,
    fragmentShader: /* glsl */ `
      uniform float time; varying vec3 vNormal; varying vec3 vView; varying vec3 vPos; ${NOISE}
      void main() {
        float facing = clamp(dot(normalize(vNormal), normalize(vView)), 0.0, 1.0);
        float swirl = fbm(vPos * 2.2 + vec3(time * 0.25, -time * 0.18, time * 0.12));
        float veins = smoothstep(0.55, 0.75, swirl);
        float pulse = 0.85 + 0.15 * sin(time * 1.7);
        vec3 inner = mix(vec3(0.12, 0.85, 0.06), vec3(1.1, 2.4, 0.7), veins) * pulse;
        vec3 rim = vec3(0.4, 2.4, 0.3) * pow(1.0 - facing, 2.5);
        gl_FragColor = vec4(inner * (0.55 + 0.45 * facing) + rim, 1.0);
      }`,
  });
}

function glassShellMaterial() {
  return new THREE.ShaderMaterial({
    transparent: true,
    depthWrite: false,
    blending: THREE.AdditiveBlending,
    uniforms: { time: { value: 0 } },
    vertexShader: /* glsl */ `
      varying vec3 vNormal; varying vec3 vView;
      void main() {
        vec4 world = modelMatrix * vec4(position, 1.0);
        vNormal = normalize(normalMatrix * normal);
        vView = normalize(-(viewMatrix * world).xyz);
        gl_Position = projectionMatrix * viewMatrix * world;
      }`,
    fragmentShader: /* glsl */ `
      uniform float time; varying vec3 vNormal; varying vec3 vView;
      void main() {
        float facing = clamp(dot(normalize(vNormal), normalize(vView)), 0.0, 1.0);
        float fresnel = pow(1.0 - facing, 3.0);
        float spec = pow(max(dot(reflect(-normalize(vView), normalize(vNormal)), normalize(vec3(-0.4, 0.8, 0.5))), 0.0), 40.0);
        vec3 color = vec3(0.25, 1.4, 0.2) * fresnel + vec3(1.4) * spec;
        gl_FragColor = vec4(color, fresnel * 0.9 + spec);
      }`,
  });
}

function conduitMaterial(phase, speed, brightness) {
  return new THREE.ShaderMaterial({
    transparent: true,
    depthWrite: false,
    side: THREE.DoubleSide,
    blending: THREE.AdditiveBlending,
    uniforms: { time: { value: 0 }, phase: { value: phase }, speed: { value: speed }, brightness: { value: brightness } },
    vertexShader: /* glsl */ `
      varying vec2 vUv; varying vec3 vNormal; varying vec3 vView;
      void main() {
        vUv = uv;
        vec4 world = modelMatrix * vec4(position, 1.0);
        vNormal = normalize(normalMatrix * normal);
        vView = normalize(-(viewMatrix * world).xyz);
        gl_Position = projectionMatrix * viewMatrix * world;
      }`,
    fragmentShader: /* glsl */ `
      uniform float time; uniform float phase; uniform float speed; uniform float brightness;
      varying vec2 vUv; varying vec3 vNormal; varying vec3 vView;
      void main() {
        float facing = abs(dot(normalize(vNormal), normalize(vView)));
        float rim = pow(1.0 - facing, 2.2);
        float flow = fract(vUv.x * 3.0 - time * speed + phase);
        float pulse = smoothstep(0.0, 0.02, flow) * (1.0 - smoothstep(0.02, 0.1, flow));
        float fade = smoothstep(0.0, 0.08, vUv.x) * (1.0 - smoothstep(0.9, 1.0, vUv.x));
        float spec = pow(max(dot(reflect(-normalize(vView), normalize(vNormal)), normalize(vec3(-0.3, 0.9, 0.4))), 0.0), 24.0);
        vec3 glass = vec3(0.06, 0.38, 0.05) * (0.2 + rim) + vec3(0.8, 1.0, 0.8) * spec * 0.5;
        vec3 energy = vec3(0.7, 2.4, 0.45) * pulse;
        float alpha = (0.06 + rim * 0.4 + pulse * 0.9 + spec * 0.4) * fade;
        gl_FragColor = vec4((glass + energy) * brightness * fade, alpha);
      }`,
  });
}

function particleMaterial() {
  return new THREE.ShaderMaterial({
    transparent: true,
    depthWrite: false,
    blending: THREE.AdditiveBlending,
    uniforms: { time: { value: 0 }, scale: { value: 1 } },
    vertexShader: /* glsl */ `
      uniform float time; uniform float scale; attribute float seed; varying float vGlow;
      void main() {
        vec3 p = position;
        float a = time * (0.03 + seed * 0.05) + seed * 6.2831;
        p.xz = mat2(cos(a), -sin(a), sin(a), cos(a)) * (p.xz - vec2(${ORB.x.toFixed(2)}, ${ORB.z.toFixed(2)})) + vec2(${ORB.x.toFixed(2)}, ${ORB.z.toFixed(2)});
        p.y += sin(time * 0.4 + seed * 40.0) * 0.25;
        vec4 view = modelViewMatrix * vec4(p, 1.0);
        vGlow = 0.35 + 0.65 * fract(seed * 13.7 + time * 0.05);
        gl_PointSize = scale * (6.0 + seed * 14.0) / -view.z;
        gl_Position = projectionMatrix * view;
      }`,
    fragmentShader: /* glsl */ `
      varying float vGlow;
      void main() {
        float d = length(gl_PointCoord - 0.5);
        float soft = smoothstep(0.5, 0.0, d);
        gl_FragColor = vec4(vec3(0.35, 1.25, 0.28) * vGlow * soft, soft * vGlow);
      }`,
  });
}

function haloTexture() {
  const size = 256;
  const paint = document.createElement("canvas");
  paint.width = paint.height = size;
  const g = paint.getContext("2d");
  const gradient = g.createRadialGradient(size / 2, size / 2, 0, size / 2, size / 2, size / 2);
  gradient.addColorStop(0, "rgba(200,255,170,0.5)");
  gradient.addColorStop(0.2, "rgba(120,255,70,0.22)");
  gradient.addColorStop(0.55, "rgba(50,190,30,0.05)");
  gradient.addColorStop(1, "rgba(0,0,0,0)");
  g.fillStyle = gradient;
  g.fillRect(0, 0, size, size);
  const texture = new THREE.CanvasTexture(paint);
  texture.colorSpace = THREE.SRGBColorSpace;
  return texture;
}

// Energy conduits sweep around the core and off into the dark.
const CONDUITS = [
  [[-9, -2.2, -6], [-3, -1.4, -2], [1.2, -0.6, 0.2], [ORB.x + 1.6, 0.1, -0.4], [7, 1.4, -4]],
  [[-8, 3.0, -9], [-2, 1.8, -4], [ORB.x - 0.4, 1.3, -2.6], [ORB.x + 2.2, 0.6, -1.2], [9, -1.2, 1]],
  [[-10, 0.4, -3], [-4, 0.2, -1.2], [0.6, 0.9, -0.6], [ORB.x, 1.6, -0.4], [5, 4.0, -7]],
  [[-6, -3.6, 2], [-1.5, -2.2, -0.4], [ORB.x - 0.8, -1.2, -1.0], [ORB.x + 1.4, -0.9, -2.8], [8, -2.0, -8]],
  [[-12, 2.0, -14], [-5, 0.8, -8], [ORB.x - 2.0, 0.2, -4.0], [ORB.x + 0.6, -0.3, -3.2], [6, -0.4, -12]],
];

function buildScene(quality) {
  const scene = new THREE.Scene();
  const background = new THREE.Mesh(new THREE.SphereGeometry(80, 48, 24), backgroundMaterial());
  scene.add(background);

  const core = new THREE.Group();
  core.position.copy(ORB);
  const inner = new THREE.Mesh(new THREE.SphereGeometry(0.95, 64, 32), coreMaterial());
  const shell = new THREE.Mesh(new THREE.SphereGeometry(1.32, 64, 32), glassShellMaterial());
  const halo = new THREE.Sprite(new THREE.SpriteMaterial({ map: haloTexture(), blending: THREE.AdditiveBlending, depthWrite: false, transparent: true }));
  halo.scale.setScalar(5.2);
  core.add(halo, inner, shell);
  const rings = [1.85, 2.35, 3.0].map((radius, index) => {
    const ring = new THREE.Mesh(
      new THREE.TorusGeometry(radius, 0.012 + index * 0.004, 8, 180),
      new THREE.MeshBasicMaterial({ color: new THREE.Color(0.45, 1.8, 0.35), transparent: true, opacity: 0.55 - index * 0.12, blending: THREE.AdditiveBlending, depthWrite: false }),
    );
    ring.rotation.set(1.1 + index * 0.35, index * 0.8, index * 0.4);
    core.add(ring);
    return ring;
  });
  scene.add(core);

  const conduits = [];
  CONDUITS.forEach((points, index) => {
    const curve = new THREE.CatmullRomCurve3(points.map((p) => new THREE.Vector3(...p)));
    const outer = new THREE.Mesh(new THREE.TubeGeometry(curve, 320, 0.16 - index * 0.015, 18, false), conduitMaterial(index * 0.23, 0.12 + index * 0.025, 1));
    const filament = new THREE.Mesh(new THREE.TubeGeometry(curve, 220, 0.022, 6, false), conduitMaterial(index * 0.23 + 0.05, 0.12 + index * 0.025, 1.6));
    scene.add(outer, filament);
    conduits.push(outer, filament);
  });

  const geometry = new THREE.BufferGeometry();
  const count = QUALITY[0].particles;
  const positions = new Float32Array(count * 3);
  const seeds = new Float32Array(count);
  let state = 1234567;
  const random = () => ((state = (state * 1103515245 + 12345) & 0x7fffffff) / 0x7fffffff);
  for (let i = 0; i < count; i += 1) {
    const radius = 1.8 + Math.pow(random(), 0.7) * 11;
    const angle = random() * Math.PI * 2;
    positions[i * 3] = ORB.x + Math.cos(angle) * radius;
    positions[i * 3 + 1] = (random() - 0.5) * 7 * (0.3 + radius / 12);
    positions[i * 3 + 2] = ORB.z + Math.sin(angle) * radius * 0.8;
    seeds[i] = random();
  }
  geometry.setAttribute("position", new THREE.BufferAttribute(positions, 3));
  geometry.setAttribute("seed", new THREE.BufferAttribute(seeds, 1));
  geometry.setDrawRange(0, quality.particles);
  const particles = new THREE.Points(geometry, particleMaterial());
  scene.add(particles);

  const timed = [background.material, inner.material, shell.material, particles.material, ...conduits.map((mesh) => mesh.material)];
  return { scene, core, inner, rings, particles, timed };
}

function startWebGL() {
  let qualityLevel = 0;
  const renderer = new THREE.WebGLRenderer({
    canvas,
    antialias: false,
    alpha: false,
    powerPreference: "high-performance",
    // Kept so that the launcher's --capture-preview screenshots include the rendered scene.
    preserveDrawingBuffer: true,
  });
  renderer.toneMapping = THREE.ACESFilmicToneMapping;
  renderer.toneMappingExposure = 0.85;
  const world = buildScene(QUALITY[0]);
  const camera = new THREE.PerspectiveCamera(48, 1, 0.1, 200);
  const composer = new EffectComposer(renderer);
  composer.addPass(new RenderPass(world.scene, camera));
  const bloom = new UnrealBloomPass(new THREE.Vector2(1, 1), 0.75, 0.38, 0.62);
  composer.addPass(bloom);
  composer.addPass(new OutputPass());

  const station = (name) => VIEWS[name] || VIEWS.library;
  const position = new THREE.Vector3(...station("boot").position);
  const target = new THREE.Vector3(...station("boot").target);
  const goalPosition = position.clone();
  const goalTarget = target.clone();
  const pointer = new THREE.Vector2();
  let view = "boot";
  let paused = false;
  let running = false;
  let last = performance.now();
  let clock = 0;
  let frames = 0;
  let frameTime = 0;

  function applyQuality() {
    const quality = QUALITY[qualityLevel];
    renderer.setPixelRatio(quality.pixelRatio);
    composer.setPixelRatio(quality.pixelRatio);
    bloom.enabled = quality.bloom;
    bloom.resolution.set(innerWidth * quality.bloomScale, innerHeight * quality.bloomScale);
    world.particles.geometry.setDrawRange(0, quality.particles);
    world.particles.material.uniforms.scale.value = innerHeight * quality.pixelRatio * 0.006;
    setMode("webgl", quality.name);
  }

  function resize() {
    const width = innerWidth;
    const height = innerHeight;
    renderer.setSize(width, height, false);
    composer.setSize(width, height);
    camera.aspect = width / Math.max(1, height);
    camera.updateProjectionMatrix();
    applyQuality();
    if (!running) renderFrame(0);
  }

  function renderFrame(dt) {
    clock += dt;
    for (const material of world.timed) material.uniforms.time.value = clock;
    world.core.rotation.y = clock * 0.08;
    world.rings.forEach((ring, index) => { ring.rotation.z = clock * (0.1 + index * 0.05) * (index % 2 ? -1 : 1); });
    world.inner.scale.setScalar(1 + Math.sin(clock * 1.7) * 0.025);
    const ease = dt > 0 ? 1 - Math.exp(-dt * 2.6) : 1;
    position.lerp(goalPosition, ease);
    target.lerp(goalTarget, ease);
    camera.position.copy(position);
    if (!reducedMotion()) camera.position.add(new THREE.Vector3(pointer.x * 0.35, pointer.y * 0.2, 0));
    camera.lookAt(target);
    composer.render(dt);
  }

  function loop(now) {
    if (!running) return;
    const dt = Math.min(0.1, (now - last) / 1000);
    last = now;
    renderFrame(dt);
    frameTime += dt * 1000;
    frames += 1;
    // Adaptive quality: average the frame interval; drop a step when it falls below ~50 fps.
    if (frames >= 90) {
      const average = frameTime / frames;
      if (average > 20 && qualityLevel < QUALITY.length - 1) { qualityLevel += 1; applyQuality(); }
      frames = 0;
      frameTime = 0;
    }
    requestAnimationFrame(loop);
  }

  function setRunning() {
    const shouldRun = !paused && !document.hidden && !reducedMotion();
    if (shouldRun && !running) {
      running = true;
      last = performance.now();
      requestAnimationFrame(loop);
    } else if (!shouldRun && running) {
      running = false;
    }
    if (!running) renderFrame(0);
  }

  function fly(name, instant) {
    view = name in VIEWS ? name : "library";
    goalPosition.set(...station(view).position);
    goalTarget.set(...station(view).target);
    if (instant || reducedMotion()) {
      position.copy(goalPosition);
      target.copy(goalTarget);
    }
    if (!running) renderFrame(0);
  }

  window.addEventListener("resize", resize);
  window.addEventListener("pointermove", (event) => {
    pointer.set(event.clientX / innerWidth - 0.5, 0.5 - event.clientY / innerHeight);
  });
  window.addEventListener("gamestrike:view", (event) => fly(event.detail?.view, Boolean(event.detail?.instant)));
  window.addEventListener("gamestrike:pause", (event) => { paused = Boolean(event.detail?.paused); setRunning(); });
  document.addEventListener("visibilitychange", setRunning);
  motionQuery?.addEventListener?.("change", () => { fly(view, true); setRunning(); });
  canvas.addEventListener("webglcontextlost", (event) => { event.preventDefault(); running = false; startStatic("context lost"); }, { once: true });

  resize();
  fly(body.dataset.view || "boot", true);
  setRunning();
}

// A still, hand-painted version of the scene for machines without WebGL 2.
function startStatic(reason) {
  const fallback = document.createElement("canvas");
  fallback.id = "scene";
  fallback.className = canvas.className;
  fallback.setAttribute("aria-hidden", "true");
  canvas.replaceWith(fallback);
  const g = fallback.getContext("2d");
  if (!g) { setMode("none"); return; }
  function paint() {
    const ratio = Math.min(window.devicePixelRatio || 1, 1.5);
    const width = innerWidth;
    const height = innerHeight;
    fallback.width = Math.round(width * ratio);
    fallback.height = Math.round(height * ratio);
    g.setTransform(ratio, 0, 0, ratio, 0, 0);
    const sky = g.createLinearGradient(0, 0, 0, height);
    sky.addColorStop(0, "#020604");
    sky.addColorStop(0.55, "#04130a");
    sky.addColorStop(1, "#010302");
    g.fillStyle = sky;
    g.fillRect(0, 0, width, height);
    const cx = width * 0.7;
    const cy = height * 0.42;
    const r = Math.min(width, height) * 0.14;
    g.globalCompositeOperation = "lighter";
    const halo = g.createRadialGradient(cx, cy, 0, cx, cy, r * 4.2);
    halo.addColorStop(0, "rgba(140,255,90,0.55)");
    halo.addColorStop(0.25, "rgba(70,210,40,0.22)");
    halo.addColorStop(1, "rgba(0,0,0,0)");
    g.fillStyle = halo;
    g.fillRect(0, 0, width, height);
    const curves = [
      [[-0.1, 0.8], [0.3, 0.62], [0.62, 0.5], [1.1, 0.3]],
      [[-0.1, 0.25], [0.35, 0.35], [0.75, 0.3], [1.1, 0.62]],
      [[0.0, 0.55], [0.4, 0.48], [0.7, 0.22], [0.95, -0.1]],
      [[0.1, 1.1], [0.45, 0.72], [0.8, 0.62], [1.1, 0.85]],
    ];
    for (const [index, points] of curves.entries()) {
      const [a, b, c, d] = points.map(([x, y]) => [x * width, y * height]);
      for (const [lineWidth, alpha] of [[16 - index * 2, 0.07], [5, 0.22], [1.5, 0.75]]) {
        g.beginPath();
        g.moveTo(...a);
        g.bezierCurveTo(...b, ...c, ...d);
        g.strokeStyle = `rgba(120,255,80,${alpha})`;
        g.lineWidth = lineWidth;
        g.shadowColor = "rgba(110,255,60,0.8)";
        g.shadowBlur = 18;
        g.stroke();
      }
    }
    g.shadowBlur = 0;
    const core = g.createRadialGradient(cx - r * 0.3, cy - r * 0.3, r * 0.05, cx, cy, r);
    core.addColorStop(0, "rgba(240,255,225,1)");
    core.addColorStop(0.35, "rgba(140,255,80,0.95)");
    core.addColorStop(0.85, "rgba(30,140,20,0.7)");
    core.addColorStop(1, "rgba(0,0,0,0)");
    g.fillStyle = core;
    g.beginPath();
    g.arc(cx, cy, r, 0, Math.PI * 2);
    g.fill();
    let seed = 99;
    const random = () => ((seed = (seed * 16807) % 2147483647) / 2147483647);
    for (let i = 0; i < 260; i += 1) {
      const angle = random() * Math.PI * 2;
      const distance = r * (1.3 + random() * 5);
      const size = 0.6 + random() * 2.2;
      g.fillStyle = `rgba(150,255,110,${0.15 + random() * 0.5})`;
      g.beginPath();
      g.arc(cx + Math.cos(angle) * distance, cy + Math.sin(angle) * distance * 0.6, size, 0, Math.PI * 2);
      g.fill();
    }
    g.globalCompositeOperation = "source-over";
  }
  paint();
  window.addEventListener("resize", paint);
  setMode("static", "static");
  body.dataset.sceneReason = reason;
}

if (!canvas) {
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
