//! Native forms of the project's already-bundled OFL fonts. No OS font lookup.
use eframe::egui::{
    self, Color32, Context, FontData, FontDefinitions, FontFamily, FontId, Painter,
};
use std::sync::Arc;

pub(super) fn install(ctx: &Context) {
    let mut fonts = FontDefinitions::default();
    for (name, bytes) in [
        (
            "PWDS Display",
            include_bytes!("../../assets/fonts/pwds-display.ttf").as_slice(),
        ),
        (
            "PWDS Control",
            include_bytes!("../../assets/fonts/pwds-control.ttf").as_slice(),
        ),
    ] {
        fonts
            .font_data
            .insert(name.into(), FontData::from_static(bytes).into());
        // Retain egui's proportional fallback for titles outside the Latin subset.
        let mut fallback = fonts.families[&FontFamily::Proportional].clone();
        fallback.insert(0, name.into());
        fonts
            .families
            .insert(FontFamily::Name(name.into()), fallback);
    }
    ctx.set_fonts(fonts);
}

pub(super) fn display(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("PWDS Display".into()))
}

pub(super) fn control(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("PWDS Control".into()))
}

pub(super) fn single_line(
    p: &Painter,
    text: &str,
    size: f32,
    width: f32,
    color: Color32,
) -> Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::default();
    job.append(text, 0.0, egui::TextFormat::simple(control(size), color));
    job.wrap = egui::text::TextWrapping::truncate_at_width(width.max(1.0));
    p.layout_job(job)
}

pub(super) fn fitted_control(
    p: &Painter,
    text: &str,
    size: f32,
    width: f32,
    color: Color32,
) -> Arc<egui::Galley> {
    let natural = p.layout_no_wrap(text.to_owned(), control(size), color);
    let fit = (width.max(1.0) / natural.size().x.max(1.0)).min(1.0);
    // Leave a subpixel of breathing room after font/rasterizer rounding.
    single_line(p, text, size * fit * 0.99, width, color)
}
