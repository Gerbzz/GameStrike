use super::mesh_geometry::{clean_polygon, triangles};
use super::{
    boot::BootPose,
    geometry::{Geometry, chamfer},
};
use eframe::egui::{
    self, Color32, Mesh, Painter, Pos2, Rect, Shape, Stroke, TextureId, pos2, vec2,
};

pub(super) const AMBER: Color32 = Color32::from_rgb(255, 123, 12);
pub(super) const LIGHT: Color32 = Color32::from_rgb(255, 220, 148);
pub(super) const TEXT: Color32 = Color32::from_rgb(255, 246, 238);
pub(super) const MUTED: Color32 = Color32::from_rgb(191, 184, 173);

pub(super) fn faded(p: &Painter, opacity: f32) -> Painter {
    let mut painter = p.clone();
    painter.multiply_opacity(opacity);
    painter
}

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let a = a.to_array();
    let b = b.to_array();
    let c: [u8; 4] = std::array::from_fn(|i| {
        (a[i] as f32 + (b[i] as f32 - a[i] as f32) * t.clamp(0.0, 1.0)) as u8
    });
    Color32::from_rgba_premultiplied(c[0], c[1], c[2], c[3])
}
pub(super) fn polygon(p: &Painter, points: &[Pos2], top: Color32, bottom: Color32) {
    let cleaned = clean_polygon(points);
    let points = cleaned.as_slice();
    if points.len() < 3 {
        return;
    }
    let min_y = points.iter().map(|v| v.y).fold(f32::INFINITY, f32::min);
    let max_y = points.iter().map(|v| v.y).fold(f32::NEG_INFINITY, f32::max);
    let mut mesh = Mesh::default();
    for v in points {
        mesh.colored_vertex(
            *v,
            mix(top, bottom, (v.y - min_y) / (max_y - min_y).max(0.001)),
        );
    }
    for [a, b, c] in triangles(points) {
        mesh.add_triangle(a as u32, b as u32, c as u32);
    }
    p.add(Shape::mesh(mesh));
}

fn inset(points: &[Pos2], amount: f32) -> Vec<Pos2> {
    (0..points.len())
        .map(|i| {
            let prev = points[(i + points.len() - 1) % points.len()];
            let v = points[i];
            let next = points[(i + 1) % points.len()];
            let a = (v - prev).normalized();
            let b = (next - v).normalized();
            let na = vec2(-a.y, a.x);
            let nb = vec2(-b.y, b.x);
            let sum = na + nb;
            v + sum * (amount / (1.0 + na.dot(nb)).max(0.15))
        })
        .collect()
}

pub(super) fn plate(p: &Painter, points: &[Pos2], bevel: f32, face: Color32, lit: bool) {
    let cleaned = clean_polygon(points);
    let points = cleaned.as_slice();
    if points.len() < 3 {
        return;
    }
    let shadow: Vec<_> = points
        .iter()
        .map(|v| *v + vec2(bevel * 0.5, bevel * 1.4))
        .collect();
    polygon(
        p,
        &shadow,
        Color32::from_black_alpha(190),
        Color32::from_black_alpha(220),
    );
    // Separate the black casing, polished copper lip and recessed inner bevel.
    // A single cream-to-brown edge made every surface read as the same flat slab.
    let lip = inset(points, bevel * 0.27);
    let shoulder = inset(points, bevel * 0.58);
    let inner = inset(points, bevel);
    rim(p, points, &lip, Color32::from_rgb(26, 12, 5), AMBER);
    rim(p, &lip, &shoulder, Color32::from_rgb(104, 36, 5), LIGHT);
    rim(
        p,
        &shoulder,
        &inner,
        Color32::from_rgb(4, 4, 4),
        Color32::from_rgb(98, 61, 32),
    );
    polygon(
        p,
        &inner,
        mix(face, Color32::from_rgb(48, 46, 42), 0.22),
        face,
    );
    if lit {
        for (width, alpha) in [(bevel * 3.2, 8), (bevel * 1.8, 18), (0.9, 190)] {
            p.add(Shape::closed_line(
                lip.clone(),
                Stroke::new(width, Color32::from_rgba_unmultiplied(255, 105, 8, alpha)),
            ));
        }
    }
    p.add(Shape::closed_line(
        inner,
        Stroke::new(0.75, Color32::from_black_alpha(225)),
    ));
}

fn rim(p: &Painter, outer: &[Pos2], inner: &[Pos2], dark: Color32, light: Color32) {
    let mut mesh = Mesh::default();
    for i in 0..outer.len() {
        let j = (i + 1) % outer.len();
        let edge = (outer[j] - outer[i]).normalized();
        let illumination = (0.42 + edge.x * 0.46 - edge.y * 0.34).clamp(0.0, 1.0);
        let color = mix(dark, light, illumination);
        let base = mesh.vertices.len() as u32;
        mesh.colored_vertex(outer[i], color);
        mesh.colored_vertex(outer[j], color);
        mesh.colored_vertex(inner[j], mix(color, dark, 0.40));
        mesh.colored_vertex(inner[i], mix(color, dark, 0.40));
        mesh.add_triangle(base, base + 1, base + 2);
        mesh.add_triangle(base, base + 2, base + 3);
    }
    p.add(Shape::mesh(mesh));
}

/// The active cartridge is a physical gold-orange collar, not just a thin outline.
/// Its extra three points fit inside the grid gutter; hit geometry is unchanged.
pub(super) fn cartridge(p: &Painter, rect: Rect, cut: f32, u: f32, selected: bool) {
    if !selected {
        plate(
            p,
            &chamfer(rect, [cut; 4]),
            (3.4 * u).max(1.0),
            Color32::from_rgb(10, 10, 11),
            false,
        );
        p.add(Shape::closed_line(
            chamfer(rect.shrink(2.0 * u), [cut * 0.8; 4]),
            Stroke::new((0.7 * u).max(0.5), Color32::from_rgb(125, 49, 9)),
        ));
        return;
    }
    let outer = chamfer(rect.expand(3.0 * u), [cut * 1.4; 4]);
    for (width, alpha) in [(18.0, 9), (11.0, 17), (6.0, 35)] {
        p.add(Shape::closed_line(
            outer.clone(),
            Stroke::new(
                width * u,
                Color32::from_rgba_unmultiplied(255, 119, 0, alpha),
            ),
        ));
    }
    plate(p, &outer, 7.5 * u, Color32::from_rgb(214, 83, 0), true);
    let collar = chamfer(rect.shrink(0.5 * u), [cut * 1.1; 4]);
    p.add(Shape::closed_line(
        collar,
        Stroke::new(3.0 * u, Color32::from_rgb(255, 178, 28)),
    ));
    // Highlight the upper/left facets only. This establishes a light direction
    // instead of making every edge equally luminous.
    p.add(Shape::line(
        vec![
            pos2(rect.left() - u, rect.bottom() - cut),
            pos2(rect.left() - u, rect.top() + cut),
            pos2(rect.left() + cut, rect.top() - u),
            pos2(rect.right() - cut, rect.top() - u),
        ],
        Stroke::new(2.0 * u, Color32::from_rgb(255, 237, 136)),
    ));
}

pub(super) fn display_title(p: &Painter, rect: Rect, title: &str, u: f32) {
    let make_job = |scale: f32| {
        let mut job = egui::text::LayoutJob::default();
        job.append(
            title,
            0.0,
            egui::TextFormat {
                font_id: super::fonts::display(42.0 * scale),
                extra_letter_spacing: 6.0 * scale,
                color: AMBER,
                ..Default::default()
            },
        );
        job
    };
    let mut galley = p.layout_job(make_job(u));
    if galley.size().x > rect.width() {
        galley = p.layout_job(make_job(u * rect.width() / galley.size().x));
    }
    let at = rect.left_center() - vec2(0.0, galley.size().y * 0.5);
    p.galley_with_override_text_color(
        at + vec2(0.0, 2.0 * u),
        galley.clone(),
        Color32::BLACK,
    );
    for offset in [-0.65, 0.0, 0.65] {
        p.galley_with_override_text_color(
            at + vec2(offset * u, 0.0),
            galley.clone(),
            AMBER,
        );
    }
}

/// Machined slots: inset black, lower copper reflection and a short orange core.
pub(super) fn vents(p: &Painter, rect: Rect, u: f32) {
    for i in 0..9 {
        let x = rect.left() + i as f32 * rect.width() / 9.0;
        let slot = Rect::from_min_size(pos2(x, rect.top()), vec2(11.0 * u, rect.height()));
        polygon(
            p,
            &chamfer(slot, [3.0 * u; 4]),
            Color32::from_rgb(2, 2, 3),
            Color32::from_rgb(3, 3, 4),
        );
        p.line_segment(
            [slot.left_bottom(), slot.right_bottom()],
            Stroke::new(u, Color32::from_rgb(105, 44, 11)),
        );
        let y = slot.top() + slot.height() * 0.35;
        p.line_segment(
            [pos2(x + 3.0 * u, y), pos2(x + 3.0 * u, y + 6.0 * u)],
            Stroke::new(u, Color32::from_rgb(187, 66, 7)),
        );
    }
}

fn brushed(p: &Painter, points: &[Pos2], u: f32) {
    let top = points.iter().map(|v| v.y).fold(f32::INFINITY, f32::min);
    let bottom = points.iter().map(|v| v.y).fold(f32::NEG_INFINITY, f32::max);
    // Intersect each scanline with the actual polygon, not its bounding box.
    // This keeps fine surface grain inside angular/concave plate faces.
    let count = ((bottom - top) / (3.0 * u).max(1.0)) as usize;
    for row in 1..count.min(512) {
        let y = top + row as f32 * (3.0 * u).max(1.0);
        let mut crossings = Vec::new();
        for i in 0..points.len() {
            let a = points[i];
            let b = points[(i + 1) % points.len()];
            if (a.y <= y && b.y > y) || (b.y <= y && a.y > y) {
                crossings.push(a.x + (y - a.y) * (b.x - a.x) / (b.y - a.y));
            }
        }
        crossings.sort_by(f32::total_cmp);
        for pair in crossings.chunks_exact(2) {
            if pair[1] - pair[0] > 12.0 * u {
                p.line_segment(
                    [pos2(pair[0] + 6.0 * u, y), pos2(pair[1] - 6.0 * u, y)],
                    Stroke::new(
                        0.5 * u,
                        Color32::from_white_alpha(if row % 3 == 0 { 10 } else { 4 }),
                    ),
                );
            }
        }
    }
}

pub(super) fn slab(p: &Painter, rect: Rect, cut: f32, selected: bool, focus: bool) {
    let points = chamfer(rect, [cut * 0.5, cut, 0.0, cut * 0.5]);
    if selected {
        plate(p, &points, 2.2, Color32::from_rgb(242, 106, 12), true);
        polygon(
            p,
            &chamfer(rect.shrink(2.2), [cut * 0.4, cut, 0.0, cut * 0.4]),
            Color32::from_rgb(255, 195, 94),
            Color32::from_rgb(254, 116, 17),
        );
    } else {
        plate(p, &points, 1.6, Color32::from_rgb(8, 9, 11), focus);
    }
    if focus {
        p.add(Shape::closed_line(
            chamfer(rect.shrink(1.0), [cut; 4]),
            Stroke::new(2.0, LIGHT),
        ));
    }
}

pub(super) fn frame(p: &Painter, g: &Geometry, pose: BootPose) {
    if pose.frame <= 0.0 {
        return;
    }
    let u = g.scale;
    let f = g.frame;
    let w = f.width();
    let h = f.height();
    let c = 52.0 * u;
    let top = g.header.top() + g.header.height() * 0.3;
    let x = f.left();
    let y = f.top();
    let outer = vec![
        pos2(x, top + c * 1.6),
        pos2(x + c * 0.62, top + c * 0.9),
        pos2(x + c * 0.62, top),
        pos2(f.right() - c, top),
        pos2(f.right(), top + c),
        pos2(f.right(), y + h - c * 1.4),
        pos2(f.right() - c * 0.6, y + h - c * 0.8),
        pos2(f.right() - c * 0.6, y + h),
        pos2(x + c, y + h),
        pos2(x, y + h - c),
    ];
    // Assembly is a deterministic depth projection around the frame centre.
    let zoom = 0.82 + 0.18 * pose.frame;
    let p = p.with_clip_rect(g.viewport);
    let assembled: Vec<_> = outer
        .iter()
        .map(|v| f.center() + (*v - f.center()) * zoom)
        .collect();
    plate(&p, &assembled, 7.0 * u, Color32::from_rgb(15, 15, 16), true);
    if pose.frame < 0.45 {
        return;
    }
    let interior = Rect::from_min_max(
        pos2(g.sidebar.left() - 18.0 * u, g.library.top() - 12.0 * u),
        g.library.max + vec2(14.0, 12.0) * u,
    );
    plate(
        &p,
        &chamfer(interior, [18.0 * u, 30.0 * u, 26.0 * u, 18.0 * u]),
        5.0 * u,
        Color32::from_rgb(5, 5, 6),
        true,
    );
    let glass = Rect::from_min_max(
        pos2(g.header.right() + 40.0 * u, top + 7.0 * u),
        pos2(f.right() - c - 26.0 * u, interior.top() - 8.0 * u),
    );
    if glass.is_positive() {
        plate(
            &p,
            &chamfer(glass, [0.0, 10.0 * u, 0.0, 10.0 * u]),
            2.0 * u,
            Color32::from_rgb(11, 12, 13),
            false,
        );
    }
    let side = g.sidebar.expand2(vec2(9.0, 12.0) * u);
    plate(
        &p,
        &chamfer(side, [16.0 * u, 24.0 * u, 16.0 * u, 16.0 * u]),
        5.0 * u,
        Color32::from_rgb(10, 10, 11),
        true,
    );
    for fraction in [0.16, 0.60] {
        let ey = side.top() + side.height() * fraction;
        let ear = vec![
            pos2(side.left(), ey),
            pos2(side.left() - 13.0 * u, ey + 9.0 * u),
            pos2(side.left() - 13.0 * u, ey + 64.0 * u),
            pos2(side.left(), ey + 73.0 * u),
        ];
        plate(&p, &ear, 2.0 * u, Color32::from_rgb(35, 15, 5), false);
        for dy in [22.0, 51.0] {
            rivet(&p, pos2(side.left() - 5.0 * u, ey + dy * u), 3.0 * u);
        }
    }
    plate(
        &p,
        &chamfer(
            g.library.expand(6.0 * u),
            [12.0 * u, 20.0 * u, 20.0 * u, 12.0 * u],
        ),
        4.0 * u,
        Color32::from_rgb(3, 4, 5),
        true,
    );
    let x0 = g.emblem.left() + g.emblem.width() * 0.6;
    let x1 = g.header.right() + 22.0 * u;
    let y0 = g.header.top() - 6.0 * u;
    let y1 = g.header.bottom() + 4.0 * u;
    let tip = (y1 - y0) * 0.5;
    let tab = vec![
        pos2(x0, y0 + 10.0 * u),
        pos2(x0 + 12.0 * u, y0),
        pos2(x1 - tip * 0.3, y0),
        pos2(x1 + tip * 0.7, y0 + tip),
        pos2(x1 - tip * 0.3, y1),
        pos2(x0 + 12.0 * u, y1),
    ];
    plate(&p, &tab, 5.0 * u, Color32::from_rgb(12, 13, 15), true);
    brushed(&p, &inset(&tab, 7.0 * u), u);
    p.add(Shape::line(
        vec![
            pos2(x1 + tip * 0.95, y0 + tip),
            pos2(x1 + tip * 1.5, top + 4.0 * u),
            pos2(f.right() - c * 1.2, top + 4.0 * u),
        ],
        Stroke::new(1.4 * u, AMBER.gamma_multiply(0.55)),
    ));
    let b = g.bottom;
    let mid = b.center().y;
    let bx = b.left() - 40.0 * u;
    let right = g.launch.left() - 22.0 * u;
    let title = vec![
        pos2(bx, mid),
        pos2(bx + 28.0 * u, b.top()),
        pos2(right - 26.0 * u, b.top()),
        pos2(right, b.top() + 26.0 * u),
        pos2(right, b.bottom() - 18.0 * u),
        pos2(right - 18.0 * u, b.bottom()),
        pos2(bx + 28.0 * u, b.bottom()),
    ];
    if right > bx + 50.0 * u {
        plate(&p, &title, 5.0 * u, Color32::from_rgb(12, 13, 15), true);
        brushed(&p, &inset(&title, 7.0 * u), u);
    }
    let prompt = Rect::from_min_max(
        pos2(g.launch.left() - 10.0 * u, b.top() + 10.0 * u),
        pos2(b.right() + 10.0 * u, b.bottom()),
    );
    plate(
        &p,
        &chamfer(prompt, [20.0 * u, 20.0 * u, 20.0 * u, 20.0 * u]),
        4.0 * u,
        Color32::from_rgb(12, 13, 15),
        true,
    );
    let side_glass = Rect::from_min_max(
        pos2(interior.right() + 9.0 * u, interior.top() + c * 0.5),
        pos2(
            f.right() - 9.0 * u,
            interior.top() + interior.height() * 0.66,
        ),
    );
    if side_glass.is_positive() {
        plate(
            &p,
            &chamfer(side_glass, [0.0, 8.0 * u, 8.0 * u, 0.0]),
            1.7 * u,
            Color32::from_rgb(53, 23, 8),
            false,
        );
    }
    for at in [
        pos2(x + w - c * 0.7, top + c * 0.55),
        pos2(x + w - 15.0 * u, interior.top() + c * 0.3),
        pos2(x + w - 15.0 * u, interior.bottom() - c * 0.5),
        pos2(x + 14.0 * u, y + h - c * 0.75),
    ] {
        rivet(&p, at, 5.0 * u);
    }
}

fn rivet(p: &Painter, at: Pos2, r: f32) {
    p.circle_filled(at, r * 1.5, Color32::from_rgb(20, 7, 2));
    p.circle_stroke(
        at,
        r * 1.5,
        Stroke::new((r * 0.25).max(0.5), Color32::from_rgb(147, 72, 20)),
    );
    p.circle_filled(at, r, Color32::from_rgb(5, 4, 3));
    p.circle_filled(
        at + vec2(-0.25, -0.3) * r,
        r * 0.28,
        Color32::from_rgb(133, 112, 88),
    );
    p.line_segment(
        [at - vec2(r * 0.48, r * 0.48), at + vec2(r * 0.48, r * 0.48)],
        Stroke::new((r * 0.22).max(0.4), Color32::from_rgb(62, 57, 49)),
    );
}

pub(super) fn emblem(p: &Painter, g: &Geometry, pose: BootPose, logo: TextureId) {
    if pose.emblem <= 0.001 {
        return;
    }
    let seat = g.emblem.center();
    let start = pos2(
        g.viewport.center().x,
        g.viewport.top() + g.viewport.height() * 0.43,
    );
    let at = start + (seat - start) * pose.fly;
    let start_r = g.viewport.height().min(g.viewport.width()) * 0.145;
    let radius = (start_r + (g.emblem.width() * 0.5 - start_r) * pose.fly) * pose.emblem;
    let housing = Rect::from_center_size(
        at + vec2(0.0, radius * 0.12),
        vec2(radius * 2.32, radius * 1.86),
    );
    plate(
        p,
        &chamfer(housing, [radius * 0.36; 4]),
        (4.0 * g.scale).min(radius * 0.05),
        Color32::from_rgb(12, 13, 14),
        true,
    );
    let mut mesh = Mesh::default();
    let bands = [
        (0.0, Color32::from_rgb(68, 17, 2)),
        (0.74, Color32::from_rgb(25, 7, 2)),
        (0.82, Color32::from_rgb(16, 5, 2)),
        (0.86, Color32::from_rgb(235, 105, 29)),
        (0.91, Color32::from_rgb(255, 192, 103)),
        (0.97, Color32::from_rgb(123, 49, 11)),
        (1.04, Color32::from_rgb(244, 117, 36)),
        (1.08, Color32::from_rgb(65, 22, 5)),
    ];
    for band in 0..bands.len() - 1 {
        for i in 0..96 {
            let a = i as f32 / 96.0 * std::f32::consts::TAU;
            let b = (i + 1) as f32 / 96.0 * std::f32::consts::TAU;
            let base = mesh.vertices.len() as u32;
            for (angle, r, color) in [
                (a, bands[band].0, bands[band].1),
                (b, bands[band].0, bands[band].1),
                (b, bands[band + 1].0, bands[band + 1].1),
                (a, bands[band + 1].0, bands[band + 1].1),
            ] {
                let light = (0.78 + (-angle.sin() - angle.cos()) * 0.13).clamp(0.5, 1.0);
                mesh.colored_vertex(
                    at + vec2(angle.cos(), angle.sin()) * r * radius,
                    color.gamma_multiply(light * pose.emblem),
                );
            }
            mesh.add_triangle(base, base + 1, base + 2);
            mesh.add_triangle(base, base + 2, base + 3);
        }
    }
    p.add(Shape::mesh(mesh));
    for angle in [-160.0_f32, -20.0, 50.0, 130.0] {
        let a = angle.to_radians();
        let n = vec2(a.cos(), a.sin());
        let t = vec2(-n.y, n.x);
        let center = at + n * radius * 1.08;
        let pts = [
            center - n * radius * 0.10 - t * radius * 0.07,
            center + n * radius * 0.10 - t * radius * 0.07,
            center + n * radius * 0.10 + t * radius * 0.07,
            center - n * radius * 0.10 + t * radius * 0.07,
        ];
        plate(p, &pts, 1.5 * g.scale, Color32::from_rgb(68, 24, 7), false);
    }
    let glint: Vec<_> = (0..=36)
        .map(|i| {
            let a = std::f32::consts::PI + (i as f32 / 36.0) * 1.9;
            at + vec2(a.cos(), a.sin()) * radius * 0.98
        })
        .collect();
    p.add(Shape::line(
        glint,
        Stroke::new(3.6 * g.scale, LIGHT.gamma_multiply(0.7 * pose.emblem)),
    ));
    image(
        p,
        Rect::from_center_size(at, vec2(radius * 1.45, radius * 0.625)),
        0.0,
        logo,
        Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)),
        Color32::WHITE.gamma_multiply(pose.emblem),
    );
}

pub(super) fn image(
    p: &Painter,
    rect: Rect,
    cut: f32,
    texture: TextureId,
    uv: Rect,
    tint: Color32,
) {
    let points = chamfer(rect, [cut; 4]);
    let mut mesh = Mesh::with_texture(texture);
    mesh.vertices.push(egui::epaint::Vertex {
        pos: rect.center(),
        uv: uv.center(),
        color: tint,
    });
    for v in points.iter() {
        let factor = (*v - rect.min) / rect.size();
        mesh.vertices.push(egui::epaint::Vertex {
            pos: *v,
            uv: pos2(
                uv.left() + factor.x * uv.width(),
                uv.top() + factor.y * uv.height(),
            ),
            color: tint,
        });
    }
    for i in 0..8 {
        mesh.add_triangle(0, i + 1, (i + 1) % 8 + 1);
    }
    p.add(Shape::mesh(mesh));
}
