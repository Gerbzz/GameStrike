//! Logical point geometry shared by painting, keyboard reveal, and pointer hits.
use eframe::egui::{Pos2, Rect, pos2, vec2};

#[derive(Clone, Debug)]
pub struct Geometry {
    pub viewport: Rect,
    pub frame: Rect,
    pub sidebar: Rect,
    pub library: Rect,
    pub bottom: Rect,
    pub header: Rect,
    pub emblem: Rect,
    pub navigation: [Rect; 4],
    pub context: Rect,
    pub refresh: Rect,
    pub launch: Rect,
    pub favorite: Rect,
    pub columns: usize,
    pub scale: f32,
    pub pixels_per_point: f32,
    tile_width: f32,
    tile_height: f32,
    padding: f32,
    gap: f32,
}

impl Geometry {
    pub fn new(viewport: Rect, pixels_per_point: f32) -> Self {
        let viewport = if viewport.is_finite() && viewport.is_positive() {
            viewport
        } else {
            Rect::from_min_size(Pos2::ZERO, vec2(1.0, 1.0))
        };
        let u = (viewport.width() / 1672.0)
            .min(viewport.height() / 941.0)
            .max(0.0001);
        let frame = Rect::from_min_max(
            viewport.min + vec2(20.0, 6.0) * u,
            viewport.max - vec2(22.0, 12.0) * u,
        );
        let side_width = (238.0 * u).max(170.0).min(frame.width() * 0.29);
        let sidebar = Rect::from_min_max(
            frame.min + vec2(40.0, 154.0) * u,
            pos2(
                frame.left() + 40.0 * u + side_width,
                frame.bottom() - 100.0 * u,
            ),
        );
        let library = Rect::from_min_max(
            pos2(
                frame.left() + side_width + 50.0 * u,
                frame.top() + 108.0 * u,
            ),
            frame.max - vec2(54.0, 104.0) * u,
        );
        let bottom = Rect::from_min_max(
            pos2(frame.left() + 186.0 * u, frame.bottom() - 94.0 * u),
            frame.max - vec2(28.0, 6.0) * u,
        );
        let header = Rect::from_min_size(frame.min + vec2(202.0, 30.0) * u, vec2(500.0, 58.0) * u);
        let emblem = Rect::from_min_size(frame.min + vec2(30.0, 2.0) * u, vec2(148.0, 148.0) * u);
        let row_h = (48.0 * u).max(26.0).min(sidebar.height() / 8.8);
        let row_gap = 7.5 * u;
        let navigation = std::array::from_fn(|i| {
            Rect::from_min_size(
                sidebar.min + vec2(0.0, (row_h + row_gap) * i as f32),
                vec2(side_width, row_h),
            )
        });
        let context = Rect::from_min_max(
            pos2(sidebar.left() + 9.0 * u, navigation[3].bottom() + 20.0 * u),
            sidebar.max - vec2(9.0, 8.0) * u,
        );
        let padding = 10.0 * u;
        let gap = 9.0 * u;
        // Match ui/core.js's three-row density (seven columns at 1672x941,
        // six at 1250x800), while fitting smaller native viewports safely.
        let target_height = ((library.height() - 19.0 * u - 28.0 * u) / 3.0).max(80.0);
        let target_width = target_height * (176.0 / 236.0);
        let columns =
            (((library.width() + gap) / (target_width + gap)).floor() as usize).clamp(1, 12);
        let tile_width = ((library.width() - 2.0 * padding - gap * (columns - 1) as f32)
            / columns as f32)
            .max(0.0001);
        // Width alone used to leave the third row clipped below the well. Keep
        // the cover-like aspect, but fit three complete rows at reference sizes.
        let tile_height = (tile_width * (236.0 / 176.0))
            .min(((library.height() - 2.0 * padding - 2.0 * gap) / 3.0).max(0.0001));
        let action_w = bottom.width() * 0.16;
        let action_h = bottom.height() * 0.60;
        let action_y = bottom.center().y - action_h * 0.5;
        let launch = Rect::from_min_size(
            pos2(bottom.right() - action_w * 3.15, action_y),
            vec2(action_w, action_h),
        );
        let favorite = Rect::from_min_size(
            pos2(bottom.right() - action_w * 2.1, action_y),
            vec2(action_w, action_h),
        );
        let refresh = Rect::from_min_size(
            pos2(bottom.right() - action_w * 1.05, action_y),
            vec2(action_w, action_h),
        );
        Self {
            viewport,
            frame,
            sidebar,
            library,
            bottom,
            header,
            emblem,
            navigation,
            context,
            refresh,
            launch,
            favorite,
            columns,
            scale: u,
            pixels_per_point: if pixels_per_point.is_finite() && pixels_per_point > 0.0 {
                pixels_per_point
            } else {
                1.0
            },
            tile_width,
            tile_height,
            padding,
            gap,
        }
    }

    pub fn card_cut(&self) -> f32 {
        9.0 * self.scale
    }
    pub fn row_stride(&self) -> f32 {
        self.tile_height + self.gap
    }

    pub fn card_rect(&self, index: usize, scroll: f32) -> Rect {
        let scroll = if scroll.is_finite() {
            scroll.max(0.0)
        } else {
            0.0
        };
        let row = (index / self.columns).min(1_000_000) as f32;
        Rect::from_min_size(
            self.library.min
                + vec2(
                    self.padding + (index % self.columns) as f32 * (self.tile_width + self.gap),
                    self.padding + row * self.row_stride() - scroll,
                ),
            vec2(self.tile_width, self.tile_height),
        )
    }

    pub fn max_scroll(&self, count: usize) -> f32 {
        let rows = count.div_ceil(self.columns).min(1_000_000) as f32;
        if rows == 0.0 {
            return 0.0;
        }
        // There is no trailing inter-row gap after the final cartridge.
        let overflow = (rows - 1.0) * self.row_stride() + self.tile_height + self.padding * 2.0
            - self.library.height();
        // Arithmetic residue below 1/100 point must not create a phantom scroll bar.
        if overflow < 0.01 { 0.0 } else { overflow }
    }

    pub fn reveal_scroll(&self, index: usize, count: usize, current: f32) -> f32 {
        let current = if current.is_finite() {
            current.max(0.0)
        } else {
            0.0
        };
        let tile = self.card_rect(index, current);
        let next = if tile.top() < self.library.top() + self.padding {
            current + tile.top() - self.library.top() - self.padding
        } else if tile.bottom() > self.library.bottom() - self.padding {
            current + tile.bottom() - self.library.bottom() + self.padding
        } else {
            current
        };
        next.clamp(0.0, self.max_scroll(count))
    }

    pub fn navigation_at(&self, point: Pos2) -> Option<usize> {
        self.navigation
            .iter()
            .position(|r| contains_chamfer(*r, 8.0 * self.scale, point))
    }

    pub fn card_at(&self, point: Pos2, count: usize, scroll: f32) -> Option<usize> {
        if !point.x.is_finite() || !point.y.is_finite() || !self.library.contains(point) {
            return None;
        }
        let scroll = if scroll.is_finite() {
            scroll.max(0.0)
        } else {
            0.0
        };
        let local = point - self.library.min;
        let column = ((local.x - self.padding) / (self.tile_width + self.gap)).floor();
        let row = ((local.y - self.padding + scroll) / self.row_stride()).floor();
        if column < 0.0 || column >= self.columns as f32 || row < 0.0 {
            return None;
        }
        let index = (row as usize)
            .saturating_mul(self.columns)
            .saturating_add(column as usize);
        (index < count && contains_chamfer(self.card_rect(index, scroll), self.card_cut(), point))
            .then_some(index)
    }
}

/// Clockwise corners in top-left, top-right, bottom-right, bottom-left order.
pub fn chamfer(rect: Rect, cuts: [f32; 4]) -> Vec<Pos2> {
    let limit = rect.width().min(rect.height()).max(0.0) * 0.5;
    let [a, b, c, d] = cuts.map(|v| v.clamp(0.0, limit));
    vec![
        pos2(rect.left() + a, rect.top()),
        pos2(rect.right() - b, rect.top()),
        pos2(rect.right(), rect.top() + b),
        pos2(rect.right(), rect.bottom() - c),
        pos2(rect.right() - c, rect.bottom()),
        pos2(rect.left() + d, rect.bottom()),
        pos2(rect.left(), rect.bottom() - d),
        pos2(rect.left(), rect.top() + a),
    ]
}

pub fn contains_chamfer(rect: Rect, cut: f32, p: Pos2) -> bool {
    if !rect.contains(p) || !p.x.is_finite() || !p.y.is_finite() {
        return false;
    }
    let c = cut.max(0.0).min(rect.width().min(rect.height()) * 0.5);
    let x = (p.x - rect.left()).min(rect.right() - p.x);
    let y = (p.y - rect.top()).min(rect.bottom() - p.y);
    x + y >= c
}
