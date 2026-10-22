//! Consume custom arrows once before focus changes can expose another widget.
use eframe::egui::{InputState, Key, Modifiers};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavAction {
    Select(usize),
    EnterGames,
}

pub fn take_nav_action(input: &mut InputState, index: usize) -> Option<NavAction> {
    if input.consume_key(Modifiers::NONE, Key::ArrowDown) {
        Some(NavAction::Select((index + 1) % 4))
    } else if input.consume_key(Modifiers::NONE, Key::ArrowUp) {
        Some(NavAction::Select((index + 3) % 4))
    } else if index == 0 && input.consume_key(Modifiers::NONE, Key::ArrowRight) {
        Some(NavAction::EnterGames)
    } else {
        None
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GridAction {
    Select(usize),
    Sidebar,
}

pub fn take_grid_action(
    input: &mut InputState,
    index: usize,
    count: usize,
    columns: usize,
) -> Option<GridAction> {
    if count == 0 {
        return None;
    }
    let index = index.min(count - 1);
    let columns = columns.max(1);
    let next = if input.consume_key(Modifiers::NONE, Key::ArrowLeft) {
        if index.is_multiple_of(columns) {
            return Some(GridAction::Sidebar);
        }
        index.saturating_sub(1)
    } else if input.consume_key(Modifiers::NONE, Key::ArrowRight) {
        index.saturating_add(1)
    } else if input.consume_key(Modifiers::NONE, Key::ArrowUp) {
        index.saturating_sub(columns)
    } else if input.consume_key(Modifiers::NONE, Key::ArrowDown) {
        index.saturating_add(columns)
    } else if input.consume_key(Modifiers::NONE, Key::Home) {
        0
    } else if input.consume_key(Modifiers::NONE, Key::End) {
        count - 1
    } else if input.consume_key(Modifiers::NONE, Key::PageUp) {
        index.saturating_sub(columns.saturating_mul(3))
    } else if input.consume_key(Modifiers::NONE, Key::PageDown) {
        index.saturating_add(columns.saturating_mul(3))
    } else {
        return None;
    };
    Some(GridAction::Select(next.min(count - 1)))
}
