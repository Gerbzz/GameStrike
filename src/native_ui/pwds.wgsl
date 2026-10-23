// Original native procedural scene. Pose channels are supplied by boot.rs;
// there is no shader clock, random input, texture download or browser renderer.
struct Uniforms {
    viewport_time: vec4<f32>, // logical width/height, pixels per point, seconds
    assembly: vec4<f32>,    // frame, net, ignition, core
    energy: vec4<f32>,      // flash, particles, shockwave, emblem
    reveal: vec4<f32>,      // fly, content, foreground, reserved
    origin: vec4<f32>,
    reserved: vec4<f32>,
};
@group(0) @binding(0) var<uniform> u: Uniforms;
struct VertexOut { @builtin(position) position: vec4<f32>, @location(0) uv: vec2<f32> };
@vertex fn vs_main(@builtin(vertex_index) i: u32) -> VertexOut {
    let xy = array<vec2<f32>, 3>(vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
    var out: VertexOut;
    out.position = vec4(xy[i], 0.0, 1.0);
    out.uv = vec2((xy[i].x + 1.0) * 0.5, (1.0 - xy[i].y) * 0.5);
    return out;
}
fn hash(p: f32) -> f32 { return fract(sin(p * 127.1 + 311.7) * 43758.547); }
fn composite(back: vec4<f32>, front: vec4<f32>) -> vec4<f32> {
    return front + back * (1.0 - front.a);
}
fn catmull(a: vec2<f32>, b: vec2<f32>, c: vec2<f32>, d: vec2<f32>, t: f32) -> vec2<f32> {
    return 0.5 * (2.0*b + (-a+c)*t + (2.0*a-5.0*b+4.0*c-d)*t*t + (-a+3.0*b-3.0*c+d)*t*t*t);
}
fn path_point(tube: u32, i: u32) -> vec2<f32> {
    // Original Collection pipe routes in normalized viewport coordinates.
    let paths = array<vec2<f32>, 40>(
        vec2(0.78,-0.16),vec2(0.845,-0.02),vec2(0.93,0.035),vec2(1.0,0.07),vec2(1.08,0.16),
        vec2(0.875,-0.16),vec2(0.915,-0.015),vec2(0.985,0.02),vec2(1.08,0.035),vec2(1.16,0.05),
        vec2(-0.12,0.31),vec2(-0.004,0.39),vec2(0.008,0.5),vec2(-0.004,0.61),vec2(-0.12,0.69),
        vec2(-0.15,0.62),vec2(-0.02,0.73),vec2(0.018,0.86),vec2(0.06,0.97),vec2(0.085,1.14),
        vec2(-0.12,0.86),vec2(-0.01,0.93),vec2(0.025,1.01),vec2(0.03,1.14),vec2(0.04,1.24),
        vec2(0.52,-0.18),vec2(0.6,-0.012),vec2(0.73,0.004),vec2(0.84,-0.03),vec2(0.9,-0.2),
        vec2(1.14,0.33),vec2(1.0,0.4),vec2(0.99,0.52),vec2(1.0,0.64),vec2(1.14,0.72),
        vec2(0.93,1.16),vec2(0.95,1.08),vec2(0.975,0.97),vec2(1.04,0.92),vec2(1.12,0.9));
    return paths[tube * 5u + min(i, 4u)] * u.viewport_time.xy;
}
fn tube_layer(pixel: vec2<f32>, foreground: bool) -> vec4<f32> {
    let radii = array<f32, 8>(23.0,14.0,18.0,27.0,16.0,18.0,20.0,17.0);
    let fit = min(u.viewport_time.x / 1672.0, u.viewport_time.y / 941.0);
    var result = vec4(0.0);
    for (var tube = 0u; tube < 8u; tube = tube + 1u) {
        if ((tube < 5u) != foreground) { continue; }
        let lower=array<vec2<f32>,8>(vec2(0.76,-0.20),vec2(0.85,-0.18),vec2(-0.15,0.28),vec2(-0.18,0.59),
            vec2(-0.16,0.83),vec2(0.49,-0.23),vec2(0.94,0.30),vec2(0.90,0.86));
        let upper=array<vec2<f32>,8>(vec2(1.12,0.19),vec2(1.18,0.08),vec2(0.05,0.72),vec2(0.12,1.20),
            vec2(0.07,1.28),vec2(0.94,0.06),vec2(1.18,0.75),vec2(1.18,1.20));
        let uv=pixel/u.viewport_time.xy;
        if (any(uv<lower[tube]) || any(uv>upper[tube])) { continue; }
        let radius = max(0.2, radii[tube] * fit);
        var nearest = 1e10;
        var delta = vec2(0.0);
        var tangent = vec2(1.0,0.0);
        var along = 0.0;
        for (var k = 0u; k < 24u; k = k + 1u) {
            let seg = k / 6u;
            let t = f32(k % 6u) / 6.0;
            let a = path_point(tube, select(seg-1u, 0u, seg == 0u));
            let b = path_point(tube, seg);
            let c = path_point(tube, seg+1u);
            let d = path_point(tube, min(seg+2u,4u));
            let start = catmull(a,b,c,d,t);
            let end = catmull(a,b,c,d,t+1.0/6.0);
            let v = end-start;
            let h = clamp(dot(pixel-start,v) / max(dot(v,v),0.0001),0.0,1.0);
            let q = pixel - start - v*h;
            let distance = length(q);
            if (distance < nearest) {
                nearest = distance; delta = q; tangent = normalize(v+vec2(0.00001));
                along = (f32(k)+h)/24.0;
            }
        }
        // Pipes extend along their own route as the frame comes together.
        let extension = smoothstep(along-0.04,along+0.04,u.assembly.x);
        let halo = exp(-pow(nearest / (radius*1.9),2.0)) * 0.17 * extension;
        result = composite(result, vec4(vec3(1.0,0.22,0.025)*halo,halo));
        if (nearest > radius+1.2 || extension < 0.001) { continue; }
        let normal2 = delta / radius;
        let z = sqrt(max(0.0,1.0-dot(normal2,normal2)));
        let n = normalize(vec3(normal2,z));
        let light = normalize(vec3(-0.35,-0.9,0.65));
        let lit = max(dot(n,light),0.0);
        let rim = pow(1.0-z,2.0);
        let reflected = reflect(vec3(0.0,0.0,-1.0),n);
        let strip = exp(-pow((dot(reflected.xy,vec2(-0.4,-0.9))-0.36)/0.065,2.0));
        let box = pow(max(dot(reflected,normalize(vec3(-0.45,-0.6,0.65))),0.0),36.0);
        let collar = 1.0-smoothstep(0.045,0.075,abs(fract(along*5.0+0.13)-0.5));
        let core = exp(-pow(dot(delta,vec2(-tangent.y,tangent.x))/(radius*0.22),2.0));
        let amber = vec3(1.0,0.32,0.045);
        // Dark glass between narrow hot reflections; broad brown diffuse light
        // flattened the tubes into rubber hoses. Collars remain metallic/opaque.
        var rgb = amber*(0.025+0.23*lit+1.55*rim+0.22*core) + vec3(1.0,0.86,0.63)*(strip*1.15+box*1.25);
        let seam = 1.0-smoothstep(0.008,0.025,abs(abs(fract(along*5.0+0.13)-0.5)-0.06));
        rgb = mix(rgb,amber*(0.055+0.36*lit)+vec3(1.0,0.75,0.45)*(strip*0.9+seam*0.55),collar*0.92);
        let alpha = (1.0-smoothstep(radius-1.0,radius+1.0,nearest))*extension;
        result = composite(result, vec4(rgb*alpha,alpha));
    }
    return result;
}
fn dome(uv: vec2<f32>) -> vec3<f32> {
    let aspect = u.viewport_time.x/max(1.0,u.viewport_time.y);
    let xy = (uv-vec2(0.5,0.47))*vec2(aspect,1.0);
    let ray = normalize(vec3(xy*1.65,1.0));
    let axis = normalize(vec3(-0.62,0.42,-0.66));
    let longitude = atan2(ray.x,ray.z) + 0.25*sin(ray.y*4.0);
    let latitude = acos(clamp(dot(ray,axis),-1.0,1.0));
    let grid = vec2(longitude*68.0/6.2831853,latitude*34.0/3.14159265);
    let cell = abs(fract(grid+0.5)-0.5);
    let width = max(fwidth(grid),vec2(0.0001));
    let diagonal = abs(fract(grid.x + grid.y + 0.5)-0.5) / max(fwidth(grid.x + grid.y),0.0001);
    let line = 1.0-min(min(min(cell.x/width.x,cell.y/width.y),diagonal),1.0);
    let warm = exp(-length((uv-vec2(0.16,0.15))*vec2(1.2,1.0))*2.2);
    let vignette = 1.0-smoothstep(0.25,0.85,length(xy));
    return vec3(0.009,0.004,0.003) + vec3(0.34,0.085,0.012)*line*u.assembly.y*(0.30+warm)*vignette;
}
fn ignition(uv: vec2<f32>) -> vec3<f32> {
    let t = u.viewport_time.w;
    let aspect = u.viewport_time.x/max(1.0,u.viewport_time.y);
    let p = (uv-vec2(0.5,0.43))*vec2(aspect,1.0);
    let radius = length(p);
    let angle = atan2(p.y,p.x);
    var color = vec3(0.0);
    let ignite = u.assembly.z;
    let core = u.assembly.w;
    let core_r = 0.018+0.075*core;
    let swirl = 0.65+0.35*sin(angle*7.0-radius*65.0+t*2.3);
    color += vec3(1.0,0.20,0.012)*exp(-radius*radius/(core_r*core_r))*(ignite+core)*swirl*2.4;
    color += vec3(1.0,0.8,0.45)*exp(-radius*radius/0.00016)*ignite*1.8;
    let tendril = pow(max(0.0,cos(angle*7.0+radius*35.0-t*2.1)),28.0);
    color += vec3(1.0,0.26,0.025)*tendril*exp(-radius*5.0)*smoothstep(0.15,2.2,t)*(1.0-smoothstep(2.4,3.05,t))*0.9;
    // Fixed seeds make every requested second repeatable.
    if (u.energy.y>0.00001) { for (var i=0u; i<80u; i=i+1u) {
        let seed = f32(i);
        let pull = smoothstep(0.0,2.9,t*(0.75+0.45*hash(seed+2.0)));
        let r = mix(0.1+hash(seed)*0.9,0.015+0.04*hash(seed),pull) + max(0.0,t-3.0)*hash(seed+9.0)*0.3;
        let a = hash(seed+1.0)*6.2831853+t*(0.5+hash(seed+2.0))*(1.0+2.5*pull);
        let particle = vec2(cos(a),sin(a)*0.7)*r;
        let distance = length(p-particle);
        color += vec3(1.0,0.47,0.12)*exp(-distance*distance/0.000015)*u.energy.y*(0.4+hash(seed+7.0));
    } }
    let wave = exp(-pow((radius-max(0.0,t-3.0)*0.9)/0.009,2.0))*u.energy.z;
    color += vec3(1.0,0.63,0.29)*wave;
    color += vec3(1.0,0.8,0.58)*u.energy.x*exp(-radius*1.4);
    return color;
}
fn scene(uv: vec2<f32>) -> vec4<f32> {
    let pixel = uv*u.viewport_time.xy;
    if (u.reveal.z > 0.5) { return tube_layer(pixel,true); }
    let lit = max(u.assembly.y,u.assembly.z);
    var energy=vec3(0.0);
    if (u.assembly.z>0.00001 || u.assembly.w>0.00001 || u.energy.x>0.00001 || u.energy.y>0.00001 || u.energy.z>0.00001) {
        energy=ignition(uv);
    }
    var back = vec4(dome(uv)*smoothstep(0.15,1.3,u.viewport_time.w) + energy,1.0);
    // Darkness before ignition includes the dome and the pipes.
    back = vec4(back.rgb*max(lit,step(1.3,u.viewport_time.w)),1.0);
    return composite(back,tube_layer(pixel,false));
}
@fragment fn fs_gamma(in: VertexOut) -> @location(0) vec4<f32> { return scene(in.uv); }
@fragment fn fs_linear(in: VertexOut) -> @location(0) vec4<f32> {
    let c = scene(in.uv);
    if (c.a <= 0.00001) { return vec4(0.0); }
    let straight = max(c.rgb/c.a,vec3(0.0));
    let low = straight/12.92;
    let high = pow((straight+vec3(0.055))/1.055,vec3(2.4));
    return vec4(select(high,low,straight <= vec3(0.04045))*c.a,c.a);
}
