// GameStrike's 3D backdrop: a near-black void lit by green light. A glowing core is the focal
// point; thick translucent glass conduits carry pulses of energy toward it through fog; a few
// wisps drift. Rendered with the bundled three.js and bloom on lit elements only. It is
// decoration: it never sees game content, and it talks to the launcher page only through two
// DOM events:
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
// Camera stations. Moving between views is a fly-through between these. In the library the core
// sits low on the right, where the cover grid usually leaves space.
const VIEWS = {
  boot: { position: [3.0, 0.5, 4.6], target: [2.7, 0.25, -1.6] },
  library: { position: [-1.6, 1.5, 10.8], target: [-1.6, 1.25, -1.6] },
  detail: { position: [5.8, 1.0, 6.8], target: [5.3, 0.45, -1.6] },
  saves: { position: [1.2, -0.4, 7.4], target: [1.4, 0.4, -1.6] },
  settings: { position: [-6.6, 2.0, 5.2], target: [-0.4, 0.5, -2.4] },
  about: { position: [4.4, 0.7, 4.0], target: [2.7, 0.25, -1.6] },
};
// Quality steps; the renderer drops a step whenever frames take too long.
const QUALITY = [
  { name: "high", pixelRatio: Math.min(window.devicePixelRatio || 1, 1.5), particles: 700, bloom: true, bloomScale: 1 },
  { name: "medium", pixelRatio: 1, particles: 450, bloom: true, bloomScale: 0.75 },
  { name: "low", pixelRatio: 0.75, particles: 250, bloom: true, bloomScale: 0.5 },
  { name: "minimal", pixelRatio: 0.6, particles: 120, bloom: false, bloomScale: 0.5 },
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
// Depth fog: things fade into the black as they recede.
const FOG = /* glsl */ `
  float fogFade(float depth) { return exp(-pow(max(depth - 6.0, 0.0) * 0.085, 1.4)); }
`;
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
        // Almost black, with a faint green floor of light and the barest nebula.
        float low = smoothstep(0.2, -0.7, vDir.y);
        vec3 color = vec3(0.0004, 0.0012, 0.0006) + vec3(0.0016, 0.0075, 0.0018) * low;
        float cloud = fbm(vDir * 3.1 + vec3(0.0, time * 0.008, time * 0.005));
        color += vec3(0.002, 0.012, 0.002) * smoothstep(0.58, 0.9, cloud);
        gl_FragColor = vec4(color, 1.0);
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
        float swirl = fbm(vPos * 2.4 + vec3(time * 0.22, -time * 0.16, time * 0.1));
        float veins = smoothstep(0.5, 0.78, swirl);
        float pulse = 0.8 + 0.2 * sin(time * 1.6);
        // A white-hot centre, green plasma veins, and a green limb.
        vec3 plasma = mix(vec3(0.04, 0.42, 0.03), vec3(0.9, 2.2, 0.55), veins);
        vec3 hot = vec3(1.2, 1.9, 1.0) * pow(facing, 9.0);
        vec3 rim = vec3(0.3, 1.9, 0.2) * pow(1.0 - facing, 2.0);
        gl_FragColor = vec4((plasma * (0.35 + 0.65 * facing) + hot + rim) * pulse, 1.0);
      }`,
  });
}

function glassShellMaterial() {
  return new THREE.ShaderMaterial({
    transparent: true,
    depthWrite: false,
    blending: THREE.AdditiveBlending,
    uniforms: { time: { value: 0 } },
    vertexShader: SHADED_VERTEX,
    fragmentShader: /* glsl */ `
      uniform float time; varying vec2 vUv; varying vec3 vNormal; varying vec3 vView; varying float vDepth;
      void main() {
        float facing = clamp(dot(normalize(vNormal), normalize(vView)), 0.0, 1.0);
        float fresnel = pow(1.0 - facing, 3.5);
        float spec = pow(max(dot(reflect(-normalize(vView), normalize(vNormal)), normalize(vec3(-0.4, 0.8, 0.5))), 0.0), 60.0);
        vec3 color = vec3(0.12, 0.8, 0.1) * fresnel + vec3(1.6) * spec;
        gl_FragColor = vec4(color, fresnel * 0.8 + spec);
      }`,
  });
}

// Thick glass tubes: dark, translucent bodies with bright fresnel edges and a specular streak,
// drawn with normal blending so they read against the black.
function tubeGlassMaterial(tint) {
  return new THREE.ShaderMaterial({
    transparent: true,
    depthWrite: false,
    side: THREE.DoubleSide,
    uniforms: { time: { value: 0 }, tint: { value: tint } },
    vertexShader: SHADED_VERTEX,
    fragmentShader: /* glsl */ `
      uniform float time; uniform float tint; varying vec2 vUv; varying vec3 vNormal; varying vec3 vView; varying float vDepth; ${FOG}
      void main() {
        vec3 n = normalize(vNormal); vec3 v = normalize(vView);
        float facing = abs(dot(n, v));
        float rim = pow(1.0 - facing, 2.4);
        float spec = pow(max(dot(reflect(-v, n), normalize(vec3(-0.35, 0.85, 0.4))), 0.0), 36.0);
        float streak = pow(max(dot(reflect(-v, n), normalize(vec3(0.5, -0.2, 0.84))), 0.0), 90.0);
        float ends = smoothstep(0.0, 0.06, vUv.x) * (1.0 - smoothstep(0.92, 1.0, vUv.x));
        float fade = fogFade(vDepth) * ends;
        vec3 body = vec3(0.004, 0.03, 0.006) * (0.6 + tint);
        vec3 edge = vec3(0.16, 0.95, 0.12) * rim;
        vec3 color = body + edge + vec3(1.2, 1.3, 1.15) * (spec * 0.7 + streak * 0.5);
        float alpha = (0.38 + rim * 0.55 + spec * 0.4) * fade;
        gl_FragColor = vec4(color * fade, alpha);
      }`,
  });
}

// The energy inside each tube: a bright filament with pulses travelling toward the core.
function energyMaterial(phase, speed) {
  return new THREE.ShaderMaterial({
    transparent: true,
    depthWrite: false,
    blending: THREE.AdditiveBlending,
    uniforms: { time: { value: 0 }, phase: { value: phase }, speed: { value: speed } },
    vertexShader: SHADED_VERTEX,
    fragmentShader: /* glsl */ `
      uniform float time; uniform float phase; uniform float speed;
      varying vec2 vUv; varying vec3 vNormal; varying vec3 vView; varying float vDepth; ${FOG}
      void main() {
        float flow = fract(vUv.x * 2.4 - time * speed + phase);
        float pulse = smoothstep(0.0, 0.015, flow) * (1.0 - smoothstep(0.015, 0.09, flow));
        float ends = smoothstep(0.0, 0.08, vUv.x) * (1.0 - smoothstep(0.9, 1.0, vUv.x));
        float fade = fogFade(vDepth) * ends;
        vec3 line = vec3(0.06, 0.42, 0.05);
        vec3 burst = vec3(1.3, 3.6, 0.9) * pulse;
        gl_FragColor = vec4((line + burst) * fade, (0.35 + pulse) * fade);
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
      uniform float time; uniform float scale; attribute float seed; varying float vGlow; varying float vDepth;
      void main() {
        vec3 p = position;
        float a = time * (0.02 + seed * 0.04) + seed * 6.2831;
        p.xz = mat2(cos(a), -sin(a), sin(a), cos(a)) * (p.xz - vec2(${ORB.x.toFixed(2)}, ${ORB.z.toFixed(2)})) + vec2(${ORB.x.toFixed(2)}, ${ORB.z.toFixed(2)});
        p.y += sin(time * 0.35 + seed * 40.0) * 0.2;
        vec4 view = modelViewMatrix * vec4(p, 1.0);
        vDepth = -view.z;
        // Most wisps stay dim; one in ten is bright enough to bloom.
        vGlow = seed > 0.92 ? 1.0 : 0.18 + 0.3 * fract(seed * 13.7 + time * 0.05);
        gl_PointSize = scale * (3.0 + seed * 7.0) / -view.z;
        gl_Position = projectionMatrix * view;
      }`,
    fragmentShader: /* glsl */ `
      varying float vGlow; varying float vDepth; ${FOG}
      void main() {
        float d = length(gl_PointCoord - 0.5);
        float soft = smoothstep(0.5, 0.05, d);
        float fade = fogFade(vDepth);
        gl_FragColor = vec4(vec3(0.3, 1.2, 0.25) * vGlow * soft * fade, soft * fade);
      }`,
  });
}

function haloTexture() {
  const size = 256;
  const paint = document.createElement("canvas");
  paint.width = paint.height = size;
  const g = paint.getContext("2d");
  const gradient = g.createRadialGradient(size / 2, size / 2, 0, size / 2, size / 2, size / 2);
  gradient.addColorStop(0, "rgba(190,255,160,0.55)");
  gradient.addColorStop(0.12, "rgba(110,255,70,0.22)");
  gradient.addColorStop(0.35, "rgba(40,170,25,0.05)");
  gradient.addColorStop(1, "rgba(0,0,0,0)");
  g.fillStyle = gradient;
  g.fillRect(0, 0, size, size);
  const texture = new THREE.CanvasTexture(paint);
  texture.colorSpace = THREE.SRGBColorSpace;
  return texture;
}

// Glass conduits: [points, radius]. The first three are bold hero tubes that sweep past the camera
// toward the core; the rest are thinner and further back, so the fog gives the scene depth.
const CONDUITS = [
  [[[-11, -4.2, 3.5], [-5.5, -2.6, 2.0], [-1.0, -1.2, 0.4], [ORB.x - 1.4, -0.4, -0.8], [ORB.x - 0.3, 0.1, -1.4]], 0.42],
  [[[-12, 4.8, -2], [-6, 3.6, -1.4], [-1.2, 2.4, -1.2], [ORB.x - 1.0, 1.1, -1.4], [ORB.x - 0.1, 0.45, -1.6]], 0.34],
  [[[10, -4.5, 2.5], [7.2, -2.6, 0.8], [ORB.x + 2.2, -1.0, -0.6], [ORB.x + 0.5, -0.2, -1.4]], 0.3],
  [[[-14, 1.0, -9], [-7, 0.6, -6], [-1.5, 0.4, -3.6], [ORB.x - 0.8, 0.3, -2.0]], 0.16],
  [[[12, 3.8, -10], [7, 2.2, -6.5], [ORB.x + 1.6, 0.9, -3.0], [ORB.x + 0.4, 0.35, -1.8]], 0.14],
  [[[-8, -6.0, -12], [-3, -3.0, -7], [ORB.x - 1.2, -0.9, -3.4], [ORB.x - 0.2, -0.1, -1.9]], 0.12],
];

function buildScene(quality) {
  const scene = new THREE.Scene();
  const background = new THREE.Mesh(new THREE.SphereGeometry(80, 48, 24), backgroundMaterial());
  scene.add(background);

  const core = new THREE.Group();
  core.position.copy(ORB);
  const inner = new THREE.Mesh(new THREE.SphereGeometry(0.85, 64, 32), coreMaterial());
  const shell = new THREE.Mesh(new THREE.SphereGeometry(1.25, 64, 32), glassShellMaterial());
  const halo = new THREE.Sprite(new THREE.SpriteMaterial({ map: haloTexture(), blending: THREE.AdditiveBlending, depthWrite: false, transparent: true }));
  halo.scale.setScalar(4.6);
  core.add(halo, inner, shell);
  const rings = [1.75, 2.25, 2.85].map((radius, index) => {
    const ring = new THREE.Mesh(
      new THREE.TorusGeometry(radius, 0.014 + index * 0.004, 8, 200),
      new THREE.MeshBasicMaterial({ color: new THREE.Color(0.5, 2.0, 0.4), transparent: true, opacity: 0.6 - index * 0.14, blending: THREE.AdditiveBlending, depthWrite: false }),
    );
    ring.rotation.set(1.1 + index * 0.35, index * 0.8, index * 0.4);
    core.add(ring);
    return ring;
  });
  scene.add(core);

  const timed = [background.material, inner.material, shell.material];
  CONDUITS.forEach(([points, radius], index) => {
    const curve = new THREE.CatmullRomCurve3(points.map((p) => new THREE.Vector3(...p)));
    const glass = new THREE.Mesh(new THREE.TubeGeometry(curve, 260, radius, 28, false), tubeGlassMaterial(index < 3 ? 0.4 : 0));
    const energy = new THREE.Mesh(new THREE.TubeGeometry(curve, 260, Math.max(0.025, radius * 0.16), 8, false), energyMaterial(index * 0.21, 0.1 + index * 0.02));
    glass.renderOrder = 2;
    energy.renderOrder = 1;
    scene.add(energy, glass);
    timed.push(glass.material, energy.material);
  });

  const geometry = new THREE.BufferGeometry();
  const count = QUALITY[0].particles;
  const positions = new Float32Array(count * 3);
  const seeds = new Float32Array(count);
  let state = 1234567;
  const random = () => ((state = (state * 1103515245 + 12345) & 0x7fffffff) / 0x7fffffff);
  for (let i = 0; i < count; i += 1) {
    const radius = 1.8 + Math.pow(random(), 0.8) * 12;
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
  timed.push(particles.material);

  return { scene, core, inner, rings, particles, timed };
}

function startWebGL() {
  let qualityLevel = 0;
  const renderer = new THREE.WebGLRenderer({
    canvas,
    antialias: true,
    alpha: false,
    powerPreference: "high-performance",
    // Kept so that the launcher's --capture-preview screenshots include the rendered scene.
    preserveDrawingBuffer: true,
  });
  renderer.toneMapping = THREE.ACESFilmicToneMapping;
  renderer.toneMappingExposure = 0.9;
  const world = buildScene(QUALITY[0]);
  const camera = new THREE.PerspectiveCamera(48, 1, 0.1, 200);
  const composer = new EffectComposer(renderer);
  composer.addPass(new RenderPass(world.scene, camera));
  // Only bright, lit elements (the core, pulses, a few wisps) pass the threshold and bloom.
  const bloom = new UnrealBloomPass(new THREE.Vector2(1, 1), 1.05, 0.32, 0.82);
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
    world.inner.scale.setScalar(1 + Math.sin(clock * 1.6) * 0.025);
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
    g.fillStyle = "#010302";
    g.fillRect(0, 0, width, height);
    const cx = width * 0.78;
    const cy = height * 0.66;
    const r = Math.min(width, height) * 0.1;
    g.globalCompositeOperation = "lighter";
    const halo = g.createRadialGradient(cx, cy, 0, cx, cy, r * 3);
    halo.addColorStop(0, "rgba(140,255,90,0.45)");
    halo.addColorStop(0.3, "rgba(60,200,35,0.12)");
    halo.addColorStop(1, "rgba(0,0,0,0)");
    g.fillStyle = halo;
    g.fillRect(0, 0, width, height);
    const curves = [
      [[-0.05, 1.05], [0.25, 0.92], [0.55, 0.78], [cx / width - 0.03, cy / height]],
      [[-0.05, 0.1], [0.3, 0.22], [0.6, 0.45], [cx / width - 0.02, cy / height - 0.02]],
      [[1.05, 1.0], [0.95, 0.9], [0.86, 0.76], [cx / width + 0.02, cy / height]],
    ];
    for (const [index, points] of curves.entries()) {
      const [a, b, c, d] = points.map(([x, y]) => [x * width, y * height]);
      for (const [lineWidth, color] of [[30 - index * 6, "rgba(30,120,25,0.16)"], [6, "rgba(120,255,80,0.22)"], [1.6, "rgba(170,255,130,0.8)"]]) {
        g.beginPath();
        g.moveTo(...a);
        g.bezierCurveTo(...b, ...c, ...d);
        g.strokeStyle = color;
        g.lineWidth = lineWidth;
        g.lineCap = "round";
        g.stroke();
      }
    }
    const core = g.createRadialGradient(cx - r * 0.25, cy - r * 0.25, r * 0.05, cx, cy, r);
    core.addColorStop(0, "rgba(245,255,235,1)");
    core.addColorStop(0.4, "rgba(140,255,80,0.95)");
    core.addColorStop(0.9, "rgba(25,120,15,0.6)");
    core.addColorStop(1, "rgba(0,0,0,0)");
    g.fillStyle = core;
    g.beginPath();
    g.arc(cx, cy, r, 0, Math.PI * 2);
    g.fill();
    let seed = 99;
    const random = () => ((seed = (seed * 16807) % 2147483647) / 2147483647);
    for (let i = 0; i < 160; i += 1) {
      const angle = random() * Math.PI * 2;
      const distance = r * (1.5 + random() * 6);
      g.fillStyle = `rgba(150,255,110,${0.08 + random() * 0.3})`;
      g.beginPath();
      g.arc(cx + Math.cos(angle) * distance, cy + Math.sin(angle) * distance * 0.6, 0.5 + random() * 1.6, 0, Math.PI * 2);
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
