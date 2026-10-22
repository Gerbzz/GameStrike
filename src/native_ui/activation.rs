use super::geometry::contains_chamfer;
use eframe::egui::{Key, PointerButton, Rect, Response, Ui};

pub fn clicked_without_pointer(ui: &Ui, response: &Response) -> bool {
    // egui's clicked() includes keyboard and AccessKit fake-primary activation;
    // clicked_by(Primary) intentionally only sees a real pointing device.
    response.clicked() && !ui.input(|input| input.pointer.button_clicked(PointerButton::Primary))
}

pub fn activated(ui: &Ui, response: &Response, rect: Rect, cut: f32, enabled: bool) -> bool {
    enabled
        && ((response.clicked_by(PointerButton::Primary)
            && response
                .interact_pointer_pos()
                .is_some_and(|point| contains_chamfer(rect, cut, point)))
            || clicked_without_pointer(ui, response)
            || (response.has_focus()
                && ui
                    .input(|input| input.key_pressed(Key::Enter) || input.key_pressed(Key::Space))))
}
