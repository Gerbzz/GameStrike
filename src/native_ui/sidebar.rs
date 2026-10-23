//! Contextual library navigation. Filtering never scans, imports or launches a game.
use super::{
    ControlState,
    activation::activated,
    control, fonts,
    paint::{AMBER, MUTED, TEXT},
};
use crate::{
    HostOs,
    application::{AppCommand, AppSnapshot, GameCard, GameOrigin},
    stores::Store,
};
use eframe::egui::{
    Align2, FontId, Id, Rect, Response, ScrollArea, Sense, Ui, UiBuilder, WidgetInfo, WidgetType,
    pos2, vec2,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GameFilter {
    #[default]
    All,
    Favorites,
    Recent,
    Provider(Store),
    Html,
}

impl GameFilter {
    pub fn label(self) -> &'static str {
        match self {
            Self::All => "ALL GAMES",
            Self::Favorites => "FAVORITES",
            Self::Recent => "RECENTLY PLAYED",
            Self::Provider(Store::Custom) => "CUSTOM PROGRAMS",
            Self::Provider(store) => store.name(),
            Self::Html => "HTML GAMES",
        }
    }

    pub fn matches(self, game: &GameCard) -> bool {
        match self {
            Self::All => true,
            Self::Favorites => game.favorite,
            Self::Recent => game.last_played.is_some(),
            Self::Provider(store) => game.origin == GameOrigin::Store(store),
            Self::Html => game.origin == GameOrigin::Html,
        }
    }
}

#[derive(Default)]
pub struct GamesSidebar {
    pub filter: GameFilter,
}

impl GamesSidebar {
    pub fn visible(&self, snapshot: &AppSnapshot) -> Vec<usize> {
        let mut indices: Vec<_> = snapshot
            .games
            .iter()
            .enumerate()
            .filter_map(|(index, game)| self.filter.matches(game).then_some(index))
            .collect();
        if self.filter == GameFilter::Recent {
            indices.sort_by_key(|&index| std::cmp::Reverse(snapshot.games[index].last_played));
        }
        indices
    }

    /// The provider viewport scrolls independently; local HTML/import controls stay visible.
    pub fn show(
        &mut self,
        ui: &mut Ui,
        rect: Rect,
        scale: f32,
        snapshot: &AppSnapshot,
        enabled: bool,
    ) -> Vec<AppCommand> {
        let mut commands = Vec::new();
        if !rect.is_finite() || !rect.is_positive() {
            return commands;
        }
        let row = (28.0 * scale).max(22.0);
        let gap = (4.0 * scale).max(2.0);
        let heading = (19.0 * scale).max(15.0);
        if rect.height() < (row + gap) * 6.0 + heading * 2.0 {
            ui.painter().text(
                rect.center(),
                Align2::CENTER_CENTER,
                "Increase window height",
                fonts::control(12.0),
                MUTED,
            );
            return commands;
        }
        let width = rect.width();
        let mut y = rect.top();
        label(ui, pos2(rect.left(), y), "GAME LIBRARY", scale);
        y += heading;
        for (index, filter) in [GameFilter::All, GameFilter::Favorites, GameFilter::Recent]
            .into_iter()
            .enumerate()
        {
            let area = Rect::from_min_size(pos2(rect.left(), y), vec2(width, row));
            if filter_row(
                ui,
                area,
                Id::new(("pwds.games.filter", index)),
                filter,
                snapshot,
                ControlState {
                    enabled,
                    selected: self.filter == filter,
                },
                scale,
            ) {
                self.filter = filter;
            }
            y += row + gap;
        }
        label(ui, pos2(rect.left(), y + gap), "LAUNCHER CATALOGS", scale);
        y += heading + gap;
        let bottom = rect.bottom() - (row + gap) * 3.0;
        let providers = Rect::from_min_max(
            pos2(rect.left(), y),
            pos2(rect.right(), (bottom - gap).max(y)),
        );
        ui.scope_builder(UiBuilder::new().max_rect(providers), |ui| {
            ui.set_clip_rect(providers.intersect(ui.clip_rect()));
            ScrollArea::vertical().id_salt("pwds.games.providers").auto_shrink([false, false])
                .max_height(providers.height()).show(ui, |ui| {
                    let provider_gap = (2.0 * scale).max(2.0);
                    ui.spacing_mut().item_spacing.y = provider_gap;
                    let provider_height = ((providers.height() - provider_gap * 7.0) / 8.0)
                        .clamp(22.0, (26.0 * scale).max(22.0));
                    for store in Store::ALL {
                        let filter = GameFilter::Provider(store);
                        let status = snapshot.providers.iter().find(|status| status.key == store.key());
                        let supported = status.map_or_else(|| store.supported_on(HostOs::current()), |status| status.supported);
                        let scanner_enabled = status.is_none_or(|status| status.enabled);
                        // Existing retained catalog entries remain browsable on every OS.
                        let count = snapshot.games.iter().filter(|game| filter.matches(game)).count();
                        let selectable = enabled && (count > 0 || supported && scanner_enabled);
                        let (_, area) = ui.allocate_space(vec2(ui.available_width(), provider_height));
                        let hint = if snapshot.synthetic {
                            "Synthetic capture: no launcher discovery or game-launch authority.".to_owned()
                        } else if !supported {
                            "Discovery is Windows-only. A retained catalog entry is not proof it can launch on this OS.".to_owned()
                        } else if !scanner_enabled {
                            "This provider's scanner is off in the existing catalog settings.".to_owned()
                        } else {
                            format!("{} catalog games. Scan installed games reads local install records only; it does not install or sign in to this launcher.", count)
                        };
                        let count_text = if !supported && count == 0 { "WIN".to_owned() } else if !scanner_enabled { "OFF".to_owned() } else { count.to_string() };
                        let response = catalog_control(ui, area, Id::new(("pwds.games.provider", store.key())), filter.label(), &count_text,
                            ControlState { enabled: selectable, selected: self.filter == filter }, scale);
                        if selectable && response.gained_focus() { response.scroll_to_me(None); }
                        if response.interact_pointer_pos().is_none_or(|point| ui.clip_rect().contains(point))
                            && activated(ui, &response, area, 5.0 * scale, selectable) {
                            self.filter = filter;
                            response.request_focus();
                        }
                        response.on_hover_text(format!("{}\n{}", store.name(), hint));
                    }
                });
        });
        y = bottom;
        let html = Rect::from_min_size(pos2(rect.left(), y), vec2(width, row));
        if filter_row(
            ui,
            html,
            Id::new("pwds.games.html"),
            GameFilter::Html,
            snapshot,
            ControlState {
                enabled,
                selected: self.filter == GameFilter::Html,
            },
            scale,
        ) {
            self.filter = GameFilter::Html;
        }
        y += row + gap;
        let import = Rect::from_min_size(pos2(rect.left(), y), vec2(width, row));
        if action(
            ui,
            import,
            "pwds.games.import",
            "IMPORT HTML / ZIP…",
            enabled && !snapshot.synthetic,
            scale,
            "Choose a local .html, .htm or .zip file. Import copies validated content into your local library; it never launches it. Ctrl/Cmd+O also works.",
        ) {
            self.filter = GameFilter::Html;
            commands.push(AppCommand::ChooseHtmlFile);
        }
        y += row + gap;
        let scan = Rect::from_min_size(pos2(rect.left(), y), vec2(width, row));
        if action(
            ui,
            scan,
            "pwds.games.scan",
            if snapshot.refreshing {
                "SCANNING…"
            } else {
                "SCAN INSTALLED GAMES"
            },
            enabled && !snapshot.synthetic && !snapshot.refreshing,
            scale,
            "Refresh the existing local launcher scanners. Counts are saved catalog games, not connected accounts or guaranteed launch-ready games.",
        ) {
            commands.push(AppCommand::Refresh);
        }
        commands
    }
}

fn filter_row(
    ui: &Ui,
    rect: Rect,
    id: Id,
    filter: GameFilter,
    snapshot: &AppSnapshot,
    state: ControlState,
    scale: f32,
) -> bool {
    let enabled = state.enabled;
    let count = snapshot
        .games
        .iter()
        .filter(|game| filter.matches(game))
        .count();
    let response = catalog_control(
        ui,
        rect,
        id,
        filter.label(),
        &count.to_string(),
        state,
        scale,
    );
    let active = activated(ui, &response, rect, 5.0 * scale, enabled);
    if active {
        response.request_focus();
    }
    active
}

// The measured badge has its own region; long names and large counts cannot overlap.
fn catalog_control(
    ui: &Ui,
    rect: Rect,
    id: Id,
    label: &str,
    badge: &str,
    state: ControlState,
    scale: f32,
) -> Response {
    let ControlState { enabled, selected } = state;
    let response = ui.interact(
        rect,
        id,
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    response.widget_info(|| {
        WidgetInfo::selected(
            WidgetType::Button,
            enabled,
            selected,
            format!("{label}: {badge}"),
        )
    });
    let painter = super::paint::faded(ui.painter(), if enabled { 1.0 } else { 0.43 });
    super::paint::slab(&painter, rect, 5.0 * scale, selected, response.has_focus());
    let color = if selected {
        eframe::egui::Color32::from_rgb(22, 6, 0)
    } else {
        TEXT
    };
    let count = painter.layout_no_wrap(
        badge.to_owned(),
        FontId::monospace((11.0 * scale).max(9.0)),
        color,
    );
    let padding = (8.0 * scale).max(5.0);
    let badge_left = rect.right() - padding - count.size().x;
    let title = Rect::from_min_max(
        rect.min + vec2(padding, 0.0),
        pos2(
            (badge_left - padding).max(rect.left() + padding),
            rect.bottom(),
        ),
    );
    let text = fonts::fitted_control(
        &painter,
        label,
        (rect.height() * 0.48).max(11.0),
        title.width(),
        color,
    );
    painter
        .with_clip_rect(title)
        .galley(title.center() - text.size() * 0.5, text, color);
    painter.with_clip_rect(rect).galley(
        pos2(badge_left, rect.center().y - count.size().y * 0.5),
        count,
        color,
    );
    response
}

pub(super) fn label(ui: &Ui, at: eframe::egui::Pos2, text: &str, scale: f32) {
    ui.painter().text(
        at,
        Align2::LEFT_TOP,
        text,
        fonts::control((15.0 * scale).max(11.0)),
        AMBER,
    );
}

pub(super) fn action(
    ui: &Ui,
    rect: Rect,
    id: &str,
    text: &str,
    enabled: bool,
    scale: f32,
    hint: &str,
) -> bool {
    let response = control(
        ui,
        ui.painter(),
        rect,
        Id::new(id),
        text,
        ControlState {
            enabled,
            selected: false,
        },
        5.0 * scale,
    );
    let clicked = activated(ui, &response, rect, 5.0 * scale, enabled);
    response.on_hover_text(hint);
    clicked
}

pub(super) fn unavailable(ui: &mut Ui, rect: Rect, scale: f32, section: usize) {
    let (title, detail) = if section == 1 {
        (
            "SOUND COLLECTIONS",
            "Soundboard library is not connected in this build. No clips are loaded or sent to an output.",
        )
    } else {
        (
            "VOICE PRESETS",
            "Voice processing is not connected in this build. No microphone or voice device is opened.",
        )
    };
    label(ui, rect.min, title, scale);
    let body = rect
        .shrink2(vec2(2.0, 0.0))
        .translate(vec2(0.0, (30.0 * scale).max(24.0)));
    let text = ui.painter().layout(
        detail.into(),
        fonts::control((17.0 * scale).max(12.0)),
        TEXT,
        body.width(),
    );
    ui.painter()
        .with_clip_rect(rect)
        .galley(body.min, text, TEXT);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_ui::{configure_context, geometry::Geometry};
    use eframe::egui::{Context, RawInput, Shape};

    fn text_bounds(shape: &Shape, out: &mut Vec<(String, Rect)>) {
        match shape {
            Shape::Text(text) => out.push((
                text.galley.job.text.clone(),
                Rect::from_min_size(text.pos, text.galley.size()),
            )),
            Shape::Vec(shapes) => {
                for shape in shapes {
                    text_bounds(shape, out);
                }
            }
            _ => {}
        }
    }

    #[test]
    fn real_font_catalog_names_and_large_badges_never_overlap() {
        let ctx = Context::default();
        configure_context(&ctx);
        for size in [vec2(880.0, 620.0), vec2(1250.0, 800.0), vec2(1672.0, 941.0)] {
            let viewport = Rect::from_min_size(pos2(0.0, 0.0), size);
            let g = Geometry::new(viewport, 1.0);
            for name in Store::ALL
                .into_iter()
                .map(|store| GameFilter::Provider(store).label())
                .chain(["RECENTLY PLAYED"])
            {
                ctx.begin_pass(RawInput {
                    screen_rect: Some(viewport),
                    ..Default::default()
                });
                let ui = Ui::new(
                    ctx.clone(),
                    Id::new("catalog.font.test"),
                    UiBuilder::new().max_rect(g.context),
                );
                let rect = Rect::from_min_size(g.context.min, vec2(g.context.width(), 22.0));
                catalog_control(
                    &ui,
                    rect,
                    Id::new(name),
                    name,
                    "100000",
                    ControlState {
                        enabled: true,
                        selected: false,
                    },
                    g.scale,
                );
                drop(ui);
                let output = ctx.end_pass();
                let mut bounds = Vec::new();
                for shape in &output.shapes {
                    text_bounds(&shape.shape, &mut bounds);
                }
                let title = bounds.iter().find(|(text, _)| text == name).unwrap().1;
                let count = bounds.iter().find(|(text, _)| text == "100000").unwrap().1;
                assert!(
                    rect.contains_rect(title) && rect.contains_rect(count),
                    "{name} at {size:?}"
                );
                assert!(
                    title.right() + 2.0 < count.left(),
                    "{name} overlaps its measured badge at {size:?}"
                );
                output.drop_without_applying_deltas();
            }
        }
    }
}
