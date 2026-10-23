use eframe::egui::Pos2;

pub fn clean_polygon(points: &[Pos2]) -> Vec<Pos2> {
    let mut clean = Vec::with_capacity(points.len());
    for &point in points {
        if point.x.is_finite()
            && point.y.is_finite()
            && clean
                .last()
                .is_none_or(|last: &Pos2| last.distance_sq(point) > 0.00000001)
        {
            clean.push(point);
        }
    }
    if clean.len() > 1 && clean[0].distance_sq(*clean.last().unwrap()) < 0.00000001 {
        clean.pop();
    }
    // All bevel offsets use the inward normal of clockwise screen-space edges.
    // Ear brackets arrive counter-clockwise in the original frame measurements.
    if clean.len() >= 3 {
        let area: f32 = (0..clean.len())
            .map(|i| {
                let a = clean[i];
                let b = clean[(i + 1) % clean.len()];
                a.x * b.y - b.x * a.y
            })
            .sum();
        if area < 0.0 {
            clean.reverse();
        }
    }
    clean
}

fn cross(a: Pos2, b: Pos2, c: Pos2) -> f32 {
    (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)
}

/// Ear clipping for a simple polygon, after adjacent duplicates are removed.
pub fn triangles(points: &[Pos2]) -> Vec<[usize; 3]> {
    if points.len() < 3 {
        return Vec::new();
    }
    let mut indices: Vec<_> = (0..points.len()).collect();
    let mut result = Vec::with_capacity(points.len() - 2);
    let area: f32 = (0..points.len())
        .map(|i| {
            let a = points[i];
            let b = points[(i + 1) % points.len()];
            a.x * b.y - b.x * a.y
        })
        .sum();
    if area < 0.0 {
        indices.reverse();
    }
    while indices.len() > 3 {
        let ear = (0..indices.len()).find(|&i| {
            let a = indices[(i + indices.len() - 1) % indices.len()];
            let b = indices[i];
            let c = indices[(i + 1) % indices.len()];
            cross(points[a], points[b], points[c]) > 0.000001
                && !indices.iter().any(|&p| {
                    p != a
                        && p != b
                        && p != c
                        && cross(points[a], points[b], points[p]) >= 0.0
                        && cross(points[b], points[c], points[p]) >= 0.0
                        && cross(points[c], points[a], points[p]) >= 0.0
                })
        });
        let Some(i) = ear else {
            return Vec::new();
        };
        result.push([
            indices[(i + indices.len() - 1) % indices.len()],
            indices[i],
            indices[(i + 1) % indices.len()],
        ]);
        indices.remove(i);
    }
    if indices.len() == 3 {
        result.push([indices[0], indices[1], indices[2]]);
    }
    result
}
