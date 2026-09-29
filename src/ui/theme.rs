//! Terminal styles and minimum viewport.
use super::{MIN_HEIGHT, MIN_WIDTH};
use ratatui::{
    layout::Size,
    style::{Color, Modifier, Style},
    widgets::List,
};

fn colors_enabled() -> bool {
    std::env::var_os("NO_COLOR").is_none_or(|value| value.is_empty())
}
fn color(value: Color) -> Style {
    if colors_enabled() {
        Style::default().fg(value)
    } else {
        Style::default()
    }
}
pub(super) fn title() -> Style {
    Style::default().add_modifier(Modifier::BOLD)
}
pub(super) fn selected() -> Style {
    Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD)
}
pub(super) fn error() -> Style {
    color(Color::Red).add_modifier(Modifier::BOLD)
}
pub(super) fn warning() -> Style {
    color(Color::Yellow)
}
pub(super) fn success() -> Style {
    color(Color::Green)
}
pub(super) fn muted() -> Style {
    Style::default().add_modifier(Modifier::DIM)
}
pub(super) fn key() -> Style {
    Style::default().add_modifier(Modifier::BOLD)
}
pub(super) fn usable(size: Size) -> bool {
    size.width >= MIN_WIDTH && size.height >= MIN_HEIGHT
}
pub(super) fn selection_list<'a>(items: impl IntoIterator<Item = &'a str>) -> List<'a> {
    List::new(items)
        .highlight_symbol("> ")
        .scroll_padding(2)
        .highlight_style(selected())
}
