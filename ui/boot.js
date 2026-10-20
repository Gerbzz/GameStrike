// The Collection style's startup, in the same WebGL scene as the chrome (about 5.5 s):
//   0.0-1.2  black; a point of amber light ignites, and glowing particles and liquid-light
//            tendrils begin to swirl in toward it.
//   1.2-3.0  the tendrils coil into a glowing core with depth and bloom; the camera pulls back
//            through drifting particles while the dome net fades in around it.
//   3.0-4.2  a flash and a shockwave ring; the PWDS emblem resolves out of the core as a glossy
//            disc in a bevelled ring (the page slides the GAMESTRIKE wordmark in beneath it), and a
//            light sweeps across both.
//   4.2-5.5  the emblem flies to its seat in the frame while the frame assembles around the camera
//            (scene.js and chrome.js), tubes extend, and the page's header, rows, and covers slide
//            and flip in.
// Every shape, particle, and shader here is original; the only artwork is the PWDS logo, drawn from
// the bundled PWDS-logo-orange.svg. Pure function of time: update(t) poses everything for second t,
// so captures can freeze any moment and a skip jumps straight to the end.
import * as THREE from "./vendor/three/three.module.min.js";

export const BOOT = { ignite: 0.15, core: 1.2, flash: 3.0, assemble: 4.2, end: 5.5 };
export const CORE = new THREE.Vector3(0, 0.25, -4);

const clamp01 = (t) => Math.min(1, Math.max(0, t));
const smooth = (a, b, x) => { const t = clamp01((x - a) / (b - a)); return t * t * (3 - 2 * t); };
const easeInOut = (t) => (t < 0.5 ? 4 * t * t * t : 1 - (-2 * t + 2) ** 3 / 2);
const easeBack = (t) => { const c = 1.7; return 1 + (c + 1) * (t - 1) ** 3 + c * (t - 1) ** 2; };
const mix = (a, b, t) => a + (b - a) * t;

// Where the camera is at second t, how bright the dome net is, and how far the frame has assembled.
export function bootPose(t) {
  const pull = easeInOut(smooth(BOOT.core, BOOT.flash, t));
  const home = easeInOut(smooth(BOOT.assemble, BOOT.end, t));
  const position = new THREE.Vector3(0, mix(0.25, 0.3, pull), mix(-1.2, 3.0, pull)).lerp(new THREE.Vector3(0, 0, 0), home);
  // A short shake when the core flashes.
  const shake = t >= BOOT.flash ? Math.exp(-(t - BOOT.flash) * 7) * 0.05 : 0;
  position.x += Math.sin(t * 93) * shake;
  position.y += Math.cos(t * 71) * shake;
  const flashGlow = t >= BOOT.flash ? Math.exp(-(t - BOOT.flash) * 5) * 0.35 : 0;
  const net = mix(smooth(1.3, BOOT.flash, t) * 0.7 + flashGlow, 1, home);
  return { position, pitch: mix(-1, 0, home), net, assembly: clamp01((t - BOOT.assemble) / (BOOT.end - 0.15 - BOOT.assemble)) };
}

const POINT_VERTEX = /* glsl */ `
  attribute vec4 seed; uniform float time; uniform float scale; varying float vAlpha; varying float vHeat;
  void main() {
    float r0 = seed.x; float a0 = seed.y; float h = seed.z; float sp = seed.w;
    float pull = smoothstep(0.0, 2.9, time * (0.75 + 0.45 * sp));
    float r = mix(r0, 0.12 + 0.3 * sp, pull);
    float ang = a0 + time * (0.5 + 1.4 * sp) * (1.0 + 2.5 * pull);
    vec3 p = vec3(cos(ang) * r, h * (1.0 - pull) * 1.3 + sin(ang * 2.0 + h * 3.0) * 0.18 * r, sin(ang) * r * 0.7);
    float burst = max(time - 3.0, 0.0);
    p += normalize(p + vec3(0.0, 0.001, 0.0)) * burst * (2.0 + 4.5 * sp) / (1.0 + burst * 0.6);
    vec4 mv = modelViewMatrix * vec4(p, 1.0);
    gl_Position = projectionMatrix * mv;
    gl_PointSize = (1.6 + 3.6 * sp) * scale / max(0.4, -mv.z);
    vAlpha = smoothstep(0.1, 0.9, time) * (1.0 - smoothstep(4.2, 5.4, time)) * (0.35 + 0.65 * sp);
    vHeat = pull;
  }`;
const POINT_FRAGMENT = /* glsl */ `
  uniform vec3 tint; varying float vAlpha; varying float vHeat;
  void main() {
    float d = length(gl_PointCoord - 0.5);
    float a = smoothstep(0.5, 0.0, d) * vAlpha;
    vec3 c = mix(tint, vec3(1.0, 0.9, 0.7), vHeat * 0.6) * 1.6;
    gl_FragColor = vec4(c * a, a);
  }`;

// Liquid light along a tendril: visible up to the reveal front, brightest at the front, with
// pulses flowing toward the core.
function tendrilMaterial(tint, phase) {
  return new THREE.ShaderMaterial({
    transparent: true,
    depthWrite: false,
    blending: THREE.AdditiveBlending,
    uniforms: { tint: { value: tint.clone() }, time: { value: 0 }, reveal: { value: 0 }, fade: { value: 1 }, phase: { value: phase } },
    vertexShader: /* glsl */ `
      varying vec2 vUv; varying vec3 vNormal; varying vec3 vView;
      void main() { vUv = uv; vec4 mv = modelViewMatrix * vec4(position, 1.0); vNormal = normalize(normalMatrix * normal); vView = normalize(-mv.xyz); gl_Position = projectionMatrix * mv; }`,
    fragmentShader: /* glsl */ `
      uniform vec3 tint; uniform float time; uniform float reveal; uniform float fade; uniform float phase;
      varying vec2 vUv; varying vec3 vNormal; varying vec3 vView;
      void main() {
        if (vUv.x > reveal) discard;
        float facing = abs(dot(normalize(vNormal), normalize(vView)));
        float body = pow(facing, 1.5);
        float front = exp(-(reveal - vUv.x) * 18.0);
        float flow = fract(vUv.x * 5.0 - time * 1.6 + phase);
        float pulse = smoothstep(0.0, 0.05, flow) * (1.0 - smoothstep(0.05, 0.3, flow));
        float taper = smoothstep(0.0, 0.25, vUv.x);
        float a = (0.25 + 0.9 * pulse + 1.6 * front) * body * taper * fade;
        gl_FragColor = vec4(mix(tint, vec3(1.0, 0.92, 0.75), front * 0.7) * a * 1.4, a);
      }`,
  });
}

// The core: a glowing sphere of swirling light with a bright rim.
function coreMaterial(tint) {
  return new THREE.ShaderMaterial({
    transparent: true,
    depthWrite: false,
    blending: THREE.AdditiveBlending,
    uniforms: { tint: { value: tint.clone() }, time: { value: 0 }, heat: { value: 1 } },
    vertexShader: /* glsl */ `
      varying vec3 vPos; varying vec3 vNormal; varying vec3 vView;
      void main() { vPos = position; vec4 mv = modelViewMatrix * vec4(position, 1.0); vNormal = normalize(normalMatrix * normal); vView = normalize(-mv.xyz); gl_Position = projectionMatrix * mv; }`,
    fragmentShader: /* glsl */ `
      uniform vec3 tint; uniform float time; uniform float heat; varying vec3 vPos; varying vec3 vNormal; varying vec3 vView;
      void main() {
        vec3 p = normalize(vPos);
        float swirl = sin(p.x * 7.0 + time * 2.3 + sin(p.y * 5.0 - time * 1.7) * 2.0) * sin(p.y * 6.0 - time * 1.9 + sin(p.z * 4.0 + time) * 2.0);
        float bands = smoothstep(0.2, 0.9, swirl * 0.5 + 0.5);
        float facing = abs(dot(normalize(vNormal), normalize(vView)));
        float rim = pow(1.0 - facing, 2.0);
        float core = pow(facing, 3.0);
        vec3 c = tint * (0.5 + 1.4 * bands) * (0.4 + 0.8 * rim) + vec3(1.0, 0.85, 0.6) * core * 1.2;
        gl_FragColor = vec4(c * heat, 1.0);
      }`,
  });
}

function ringMaterial(tint) {
  return new THREE.ShaderMaterial({
    transparent: true,
    depthWrite: false,
    blending: THREE.AdditiveBlending,
    side: THREE.DoubleSide,
    uniforms: { tint: { value: tint.clone() }, alpha: { value: 0 } },
    vertexShader: /* glsl */ `varying vec2 vUv; void main() { vUv = uv; gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0); }`,
    fragmentShader: /* glsl */ `
      uniform vec3 tint; uniform float alpha; varying vec2 vUv;
      void main() {
        float r = length(vUv - 0.5) * 2.0;
        float band = exp(-pow((r - 0.93) / 0.035, 2.0)) + exp(-pow((r - 0.86) / 0.08, 2.0)) * 0.35;
        gl_FragColor = vec4(mix(tint, vec3(1.0, 0.95, 0.85), 0.5) * band * alpha * 1.8, band * alpha);
      }`,
  });
}

// A soft round glow, painted once.
function glowTexture() {
  const size = 128;
  const paint = document.createElement("canvas");
  paint.width = paint.height = size;
  const g = paint.getContext("2d");
  const gradient = g.createRadialGradient(size / 2, size / 2, 0, size / 2, size / 2, size / 2);
  gradient.addColorStop(0, "rgba(255,255,255,1)");
  gradient.addColorStop(0.2, "rgba(255,255,255,0.45)");
  gradient.addColorStop(0.55, "rgba(255,255,255,0.08)");
  gradient.addColorStop(1, "rgba(255,255,255,0)");
  g.fillStyle = gradient;
  g.fillRect(0, 0, size, size);
  const texture = new THREE.CanvasTexture(paint);
  texture.colorSpace = THREE.SRGBColorSpace;
  return texture;
}

// The PWDS logo on the emblem's face, drawn from the bundled SVG into a canvas texture. It
// resolves (onLoad) once the image has decoded; until then the face is plain.
function logoTexture(url, onLoad) {
  const paint = document.createElement("canvas");
  paint.width = 512;
  paint.height = 256;
  const texture = new THREE.CanvasTexture(paint);
  texture.colorSpace = THREE.SRGBColorSpace;
  const image = new Image();
  image.decoding = "async";
  image.onload = () => {
    const g = paint.getContext("2d");
    const ratio = (image.naturalWidth || 400) / (image.naturalHeight || 200);
    const w = Math.min(paint.width * 0.96, paint.height * 0.96 * ratio);
    const h = w / ratio;
    g.clearRect(0, 0, paint.width, paint.height);
    g.drawImage(image, (paint.width - w) / 2, (paint.height - h) / 2, w, h);
    texture.needsUpdate = true;
    onLoad?.();
  };
  image.src = url;
  return texture;
}

// A light sweep across a disc: a soft diagonal band, clipped to the circle.
function sweepMaterial() {
  return new THREE.ShaderMaterial({
    transparent: true,
    depthWrite: false,
    blending: THREE.AdditiveBlending,
    uniforms: { offset: { value: -2 } },
    vertexShader: /* glsl */ `varying vec2 vUv; void main() { vUv = uv; gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0); }`,
    fragmentShader: /* glsl */ `
      uniform float offset; varying vec2 vUv;
      void main() {
        vec2 p = (vUv - 0.5) * 2.0;
        if (length(p) > 1.0) discard;
        float band = exp(-pow((p.x * 0.8 + p.y * 0.6 - offset) / 0.16, 2.0));
        gl_FragColor = vec4(vec3(1.0, 0.92, 0.8) * band * 0.9, band * 0.9);
      }`,
  });
}

export function createBoot({ tint, envMap }) {
  const world = new THREE.Group();
  world.position.copy(CORE);
  const rig = new THREE.Group();
  const color = new THREE.Color(...tint);
  const glow = glowTexture();
  let logoReady = false;

  // Particles, spiralling in.
  const count = 1600;
  const seeds = new Float32Array(count * 4);
  let s = 1234567;
  const random = () => ((s = (s * 16807) % 2147483647) / 2147483647);
  for (let i = 0; i < count; i += 1) {
    seeds[i * 4] = 1.2 + random() * 7.5;
    seeds[i * 4 + 1] = random() * Math.PI * 2;
    seeds[i * 4 + 2] = (random() - 0.5) * 2.2;
    seeds[i * 4 + 3] = random();
  }
  const pointsGeometry = new THREE.BufferGeometry();
  pointsGeometry.setAttribute("position", new THREE.BufferAttribute(new Float32Array(count * 3), 3));
  pointsGeometry.setAttribute("seed", new THREE.BufferAttribute(seeds, 4));
  pointsGeometry.boundingSphere = new THREE.Sphere(new THREE.Vector3(), 40);
  const points = new THREE.Points(pointsGeometry, new THREE.ShaderMaterial({
    transparent: true, depthWrite: false, blending: THREE.AdditiveBlending,
    uniforms: { time: { value: 0 }, scale: { value: 4.5 }, tint: { value: color.clone() } },
    vertexShader: POINT_VERTEX, fragmentShader: POINT_FRAGMENT,
  }));
  world.add(points);

  // Tendrils: helices coiling in from far around toward the core.
  const tendrils = new THREE.Group();
  for (let i = 0; i < 7; i += 1) {
    const phase = (i / 7) * Math.PI * 2;
    const turns = 1.6 + (i % 3) * 0.35;
    const height = (random() - 0.5) * 3.2;
    const pts = [];
    for (let k = 0; k <= 60; k += 1) {
      const f = k / 60;
      const angle = phase + f * turns * Math.PI * 2;
      const r = mix(3.8, 0.1, Math.pow(f, 0.75));
      pts.push(new THREE.Vector3(Math.cos(angle) * r, mix(height, 0, f) + Math.sin(f * Math.PI * 3 + phase) * 0.35 * (1 - f), Math.sin(angle) * r * 0.7));
    }
    const curve = new THREE.CatmullRomCurve3(pts);
    const tendril = new THREE.Mesh(new THREE.TubeGeometry(curve, 220, 0.045 + (i % 2) * 0.02, 8, false), tendrilMaterial(color, i * 0.37));
    tendrils.add(tendril);
  }
  world.add(tendrils);

  // The spark that ignites, the core, its halo, and the shockwave rings.
  const spark = new THREE.Sprite(new THREE.SpriteMaterial({ map: glow, color: new THREE.Color(1, 0.75, 0.45), blending: THREE.AdditiveBlending, depthWrite: false, transparent: true }));
  world.add(spark);
  const core = new THREE.Mesh(new THREE.SphereGeometry(0.62, 64, 48), coreMaterial(color));
  world.add(core);
  const halo = new THREE.Sprite(new THREE.SpriteMaterial({ map: glow, color: color.clone(), blending: THREE.AdditiveBlending, depthWrite: false, transparent: true }));
  world.add(halo);
  const rings = [0, 0.09].map(() => {
    const ring = new THREE.Mesh(new THREE.PlaneGeometry(2, 2), ringMaterial(color));
    world.add(ring);
    return ring;
  });

  // The flash: a glow filling the view, just in front of the camera.
  const flash = new THREE.Mesh(new THREE.PlaneGeometry(6, 6), new THREE.MeshBasicMaterial({ color: new THREE.Color(1, 0.8, 0.6), transparent: true, opacity: 0, blending: THREE.AdditiveBlending, depthWrite: false, depthTest: false }));
  flash.position.z = -1.5;
  flash.renderOrder = 10;
  rig.add(flash);

  // The emblem: a glossy disc with the PWDS logo, in a thick bevelled ring (unit radius).
  const emblem = new THREE.Group();
  const ring = new THREE.LatheGeometry([[0.82, -0.08], [0.84, 0.05], [0.88, 0.13], [0.95, 0.16], [1.02, 0.13], [1.08, 0.05], [1.1, -0.04], [1.1, -0.2], [0.82, -0.2]].map(([x, y]) => new THREE.Vector2(x, y)), 96);
  ring.rotateX(Math.PI / 2);
  // A shallow glossy dome, lit from inside by a radial glow.
  const disc = new THREE.LatheGeometry(Array.from({ length: 12 }, (_, i) => { const a = (i / 11) * (Math.PI / 2); return new THREE.Vector2(Math.sin(a) * 0.84 + 0.0001, Math.cos(a) * 0.07 - 0.05); }), 96);
  disc.rotateX(Math.PI / 2);
  const glowPaint = document.createElement("canvas");
  glowPaint.width = glowPaint.height = 128;
  const glowContext = glowPaint.getContext("2d");
  const radial = glowContext.createRadialGradient(64, 72, 4, 64, 64, 64);
  radial.addColorStop(0, "#ffffff");
  radial.addColorStop(0.5, "#6a6a6a");
  radial.addColorStop(1, "#000000");
  glowContext.fillStyle = radial;
  glowContext.fillRect(0, 0, 128, 128);
  const discGlow = new THREE.CanvasTexture(glowPaint);
  discGlow.colorSpace = THREE.SRGBColorSpace;
  const ringMat = new THREE.MeshPhysicalMaterial({ color: 0xff7a1c, emissive: 0xff4800, emissiveIntensity: 0.5, metalness: 0.3, roughness: 0.24, clearcoat: 1, clearcoatRoughness: 0.04, envMap, envMapIntensity: 1.4, side: THREE.DoubleSide });
  const discMat = new THREE.MeshPhysicalMaterial({ color: 0x140602, emissive: 0x9a2a00, emissiveIntensity: 1, emissiveMap: discGlow, specularIntensity: 0.3, metalness: 0, roughness: 0.35, clearcoat: 1, clearcoatRoughness: 0.03, envMap, envMapIntensity: 1.0, side: THREE.DoubleSide });
  const logo = new THREE.Mesh(new THREE.PlaneGeometry(1.25, 0.625), new THREE.MeshBasicMaterial({ map: logoTexture("/pwds.svg", () => { logoReady = true; }), transparent: true, depthWrite: false, toneMapped: false }));
  logo.material.color.setRGB(1.6, 1.6, 1.6);
  logo.position.z = 0.04;
  const sweep = new THREE.Mesh(new THREE.PlaneGeometry(2.2, 2.2), sweepMaterial());
  sweep.position.z = 0.05;
  emblem.add(new THREE.Mesh(ring, ringMat), new THREE.Mesh(disc, discMat), logo, sweep);
  rig.add(emblem);

  // A pose for second t. `place` gives the emblem's start (rig space, radius) and its seat in
  // the frame (rig space, radius), both from scene.js.
  function update(t, { start, seat }) {
    const active = t < BOOT.end;
    world.visible = active;
    points.material.uniforms.time.value = t;
    points.material.uniforms.scale.value = 4.5 * (window.innerHeight / 800);
    tendrils.rotation.set(Math.sin(t * 0.7) * 0.15, t * 0.55, Math.sin(t * 0.5) * 0.1);
    const reveal = smooth(BOOT.ignite, 2.2, t);
    const fade = 1 - smooth(2.4, 3.05, t);
    tendrils.visible = fade > 0.001 && reveal > 0;
    for (const tendril of tendrils.children) {
      const u = tendril.material.uniforms;
      u.time.value = t;
      u.reveal.value = reveal * 1.02;
      u.fade.value = fade;
    }
    const ignite = smooth(BOOT.ignite, BOOT.ignite + 0.35, t) * (1 - smooth(1.4, 2.0, t));
    spark.visible = ignite > 0.001;
    spark.scale.setScalar((0.1 + 0.22 * ignite + Math.sin(t * 31) * 0.015) * (1 + t * 0.5));
    spark.material.opacity = ignite;
    const grow = easeBack(smooth(1.0, 2.6, t)) * (1 + Math.sin(t * 9) * 0.035);
    const collapse = 1 - smooth(BOOT.flash, BOOT.flash + 0.35, t);
    core.visible = grow * collapse > 0.001;
    core.scale.setScalar(Math.max(0.001, grow * collapse));
    core.material.uniforms.time.value = t;
    core.material.uniforms.heat.value = 0.8 + 0.6 * smooth(2.2, BOOT.flash, t);
    const haloOn = smooth(0.9, 2.4, t) * collapse + (t >= BOOT.flash ? Math.exp(-(t - BOOT.flash) * 5) * 2 : 0);
    halo.visible = haloOn > 0.001;
    halo.scale.setScalar(2.2 + 2.5 * smooth(1.0, 3.0, t) + (t >= BOOT.flash ? (t - BOOT.flash) * 6 : 0));
    halo.material.opacity = Math.min(1, haloOn * 0.8);
    rings.forEach((r, i) => {
      const local = smooth(BOOT.flash + i * 0.09, BOOT.flash + 0.95 + i * 0.09, t);
      r.visible = local > 0 && local < 1;
      r.scale.setScalar(0.3 + 13 * (1 - (1 - local) ** 3));
      r.material.uniforms.alpha.value = (1 - local) * (i ? 0.6 : 1);
    });
    const flashOn = t < BOOT.flash ? smooth(BOOT.flash - 0.1, BOOT.flash, t) * 0.45 : 0.75 * Math.exp(-(t - BOOT.flash) * 13);
    flash.visible = flashOn > 0.002 && active;
    flash.material.opacity = Math.min(1, flashOn);

    // The emblem: resolves out of the core, then flies to its seat and hands over to the chrome.
    const appear = smooth(BOOT.flash, BOOT.flash + 0.45, t);
    const fly = easeInOut(smooth(BOOT.assemble, 5.0, t));
    emblem.visible = appear > 0.001 && t < 5.0;
    const pop = Math.max(0.001, easeBack(appear));
    emblem.position.copy(start.position).lerp(seat.position, fly);
    emblem.scale.setScalar(mix(start.radius, seat.radius, fly) * pop);
    emblem.rotation.set(0, (1 - easeBack(appear)) * -1.4 + Math.sin(t * 1.3) * 0.05 * (1 - fly), 0);
    logo.visible = logoReady;
    sweep.material.uniforms.offset.value = mix(-1.8, 1.8, smooth(3.55, 4.15, t));
    return { active, emblemLanded: t >= 5.0 };
  }

  function setTint(next) {
    color.setRGB(...next);
    points.material.uniforms.tint.value.copy(color);
    for (const tendril of tendrils.children) tendril.material.uniforms.tint.value.copy(color);
    core.material.uniforms.tint.value.copy(color);
    halo.material.color.copy(color);
    for (const r of rings) r.material.uniforms.tint.value.copy(color);
  }

  return { world, rig, update, setTint, logoReady: () => logoReady };
}
