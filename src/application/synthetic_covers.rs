//! Original, deterministic geometric illustrations for the explicitly synthetic
//! capture library. These are not commercial game covers or a user's collection.
use super::CoverImage;

fn blend(a: [f32; 3], b: [f32; 3], amount: f32) -> [f32; 3] {
    let amount = amount.clamp(0.0, 1.0);
    std::array::from_fn(|channel| a[channel] + (b[channel] - a[channel]) * amount)
}

pub(super) fn cover(index: usize) -> CoverImage {
    const WIDTH: usize = 192;
    const HEIGHT: usize = 256;
    let palettes = [
        (
            [12.0, 27.0, 69.0],
            [79.0, 45.0, 143.0],
            [255.0, 175.0, 69.0],
        ),
        (
            [67.0, 21.0, 34.0],
            [213.0, 91.0, 40.0],
            [255.0, 222.0, 124.0],
        ),
        ([8.0, 17.0, 38.0], [24.0, 72.0, 111.0], [72.0, 237.0, 222.0]),
        (
            [38.0, 99.0, 139.0],
            [145.0, 199.0, 204.0],
            [255.0, 222.0, 155.0],
        ),
        (
            [8.0, 66.0, 91.0],
            [35.0, 181.0, 179.0],
            [244.0, 238.0, 172.0],
        ),
        (
            [48.0, 20.0, 42.0],
            [151.0, 37.0, 65.0],
            [255.0, 208.0, 71.0],
        ),
        (
            [19.0, 38.0, 46.0],
            [79.0, 107.0, 102.0],
            [233.0, 143.0, 63.0],
        ),
    ];
    let scene = index % palettes.len();
    let variant = (index / palettes.len()) as f32;
    let (dark, mid, light) = palettes[scene];
    let accent = if variant == 1.0 {
        [light[1], light[2], light[0]]
    } else if variant == 2.0 {
        [light[2], light[0], light[1]]
    } else {
        light
    };
    let mut rgba = Vec::with_capacity(WIDTH * HEIGHT * 4);
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let px = x as f32 / WIDTH as f32;
            let py = y as f32 / HEIGHT as f32;
            let phase = variant * 1.7;
            let mut color = blend(dark, mid, (1.0 - py) * 0.65);
            let sun_x = 0.67 - variant * 0.13;
            let sun_y = 0.24 + variant * 0.035;
            let distance = ((px - sun_x).powi(2) + (py - sun_y).powi(2)).sqrt();
            color = blend(
                color,
                accent,
                (1.0 - distance / 0.42).max(0.0).powi(3) * 0.45,
            );
            match scene {
                0 => {
                    // A tilted planetary ring, small moons and an angular glider.
                    let dx = px - 0.48;
                    let dy = py - 0.34;
                    let planet = (dx * dx + dy * dy).sqrt();
                    if planet < 0.20 {
                        color = blend(mid, accent, (1.0 - planet / 0.20) * 0.65 + (0.5 - dx) * 0.2);
                    }
                    let ring = ((dx * 0.82 + dy * 0.58).powi(2)
                        + ((dy * 0.82 - dx * 0.58) * 4.0).powi(2))
                    .sqrt();
                    if (0.28..0.32).contains(&ring) && (dy > 0.0 || planet > 0.20) {
                        color = blend(accent, [255.0; 3], (ring - 0.28) * 12.0);
                    }
                    let wing = (px - 0.55).abs();
                    if (0.57..0.69).contains(&py) && wing < (py - 0.57) * 2.3 {
                        color = blend(
                            [14.0, 20.0, 30.0],
                            accent,
                            if px < 0.55 { 0.9 } else { 0.35 },
                        );
                    }
                }
                1 => {
                    // Warm, layered canyon walls and a winding illuminated valley.
                    if distance < 0.105 {
                        color = accent;
                    }
                    for layer in 0..4 {
                        let k = layer as f32;
                        let ridge =
                            0.36 + k * 0.13 + 0.08 * (px * (7.0 + k * 3.0) + phase + k).sin();
                        if py > ridge {
                            color = blend(mid, dark, k * 0.24 + 0.12);
                        }
                    }
                    let river = 0.49 + (py * 9.0 + phase).sin() * (py - 0.35).max(0.0) * 0.18;
                    if py > 0.48 && (px - river).abs() < (py - 0.46) * 0.045 {
                        color = accent;
                    }
                }
                2 => {
                    // A neon skyline, window grids and a perspective light road.
                    let block = (px * 11.0).floor();
                    let roof = 0.28 + ((block * 1.73 + phase).sin() * 0.5 + 0.5) * 0.26;
                    if py > roof && py < 0.70 {
                        color = blend(dark, mid, 0.10 + (block % 3.0) * 0.12);
                        if (px * 55.0).fract() < 0.20 && (py * 46.0).fract() < 0.20 {
                            color = accent;
                        }
                    }
                    let road = (py - 0.50).max(0.0);
                    let edge = (px - 0.5).abs();
                    if py > 0.55 && (edge - road * 0.65).abs() < 0.012 {
                        color = accent;
                    }
                    if py > 0.58 && (py * 13.0).fract() < 0.03 && edge < road * 0.65 {
                        color = blend(mid, accent, 0.55);
                    }
                }
                3 => {
                    // Bright cloud bands, suspended islands and a narrow citadel.
                    if ((py + 0.025 * (px * 17.0 + phase).sin()) * 10.0).fract() < 0.16 && py < 0.5
                    {
                        color = blend(color, [242.0, 240.0, 213.0], 0.55);
                    }
                    for island in 0..3 {
                        let k = island as f32;
                        let center = 0.23 + k * 0.25;
                        let level = 0.48 + (k - 1.0).abs() * 0.13;
                        if py > level && py < level + 0.15 - (px - center).abs() * 0.9 {
                            color = blend(dark, mid, (py - level) * 3.0);
                        }
                    }
                    if (px - 0.48).abs() < 0.045 && (0.25..0.49).contains(&py) {
                        color = blend(accent, mid, if px > 0.48 { 0.55 } else { 0.12 });
                    }
                }
                4 => {
                    // A turquoise sea, curved wave bands and an original sailboat.
                    if distance < 0.095 {
                        color = accent;
                    }
                    if py > 0.38 {
                        let wave = (py * 47.0 + (px * 11.0 + phase).sin() * 2.0).sin();
                        color = blend(dark, mid, 0.55 + wave * 0.22);
                        if wave > 0.94 {
                            color = blend(mid, accent, 0.52);
                        }
                    }
                    if (0.28..0.56).contains(&py) && px > 0.40 && px < 0.40 + (py - 0.28) * 0.65 {
                        color = [247.0, 235.0, 200.0];
                    }
                    if (0.56..0.59).contains(&py) && (0.35..0.64).contains(&px) {
                        color = dark;
                    }
                }
                5 => {
                    // Overhead oval racetrack with offset racing strips and a car.
                    let ring = (((px - 0.5) * 1.4).powi(2) + ((py - 0.41) * 1.05).powi(2)).sqrt();
                    if (0.22..0.38).contains(&ring) {
                        color = [24.0, 25.0, 33.0];
                    }
                    if (ring - 0.22).abs() < 0.012 || (ring - 0.38).abs() < 0.012 {
                        color = accent;
                    }
                    if (ring - 0.30).abs() < 0.004 && ((px + py) * 18.0).fract() < 0.55 {
                        color = [220.0, 213.0, 194.0];
                    }
                    if (0.65..0.73).contains(&px) && (0.27..0.43).contains(&py) {
                        color = blend(accent, [255.0, 255.0, 255.0], 0.15);
                        if (0.68..0.71).contains(&px) {
                            color = mid;
                        }
                        if (0.30..0.34).contains(&py) {
                            color = dark;
                        }
                    }
                }
                _ => {
                    // Monumental angular structures against a cold industrial sky.
                    let disc = ((px - 0.5).powi(2) + (py - 0.31).powi(2)).sqrt();
                    if (disc - 0.21).abs() < 0.013 {
                        color = accent;
                    }
                    for tower in 0..5 {
                        let k = tower as f32;
                        let center = 0.10 + k * 0.20;
                        let top = 0.39 + (k - 2.0).abs() * 0.075;
                        if py > top + (px - center).abs() * 0.8 && (px - center).abs() < 0.075 {
                            color = blend(dark, mid, if px < center { 0.60 } else { 0.12 });
                            if (px - center).abs() < 0.006 {
                                color = accent;
                            }
                        }
                    }
                }
            }
            let noise = (x as u32).wrapping_mul(73_856_093)
                ^ (y as u32).wrapping_mul(19_349_663)
                ^ (index as u32).wrapping_mul(83_492_791);
            if scene == 0 && noise % 541 < 3 && py < 0.65 {
                color = [220.0, 237.0, 255.0];
            }
            // Fine deterministic print grain, not the repeated diagonal test bars.
            let grain = (noise % 9) as f32 - 4.0;
            for channel in color {
                rgba.push((channel + grain).clamp(0.0, 255.0) as u8);
            }
            rgba.push(255);
        }
    }
    CoverImage {
        size: [WIDTH, HEIGHT],
        rgba,
    }
}

#[cfg(test)]
mod tests {
    use super::cover;
    use std::collections::BTreeSet;

    #[test]
    fn original_fixture_illustrations_are_bounded_opaque_distinct_and_repeatable() {
        let mut identities = BTreeSet::new();
        for index in 0..21 {
            let image = cover(index);
            assert_eq!(image.size, [192, 256]);
            assert_eq!(image.rgba.len(), 192 * 256 * 4);
            let (pixels, remainder) = image.rgba.as_chunks::<4>();
            assert!(remainder.is_empty());
            assert!(pixels.iter().all(|pixel| pixel[3] == 255));
            assert_eq!(image.rgba, cover(index).rgba);
            assert!(identities.insert(image.rgba));
        }
    }
}
