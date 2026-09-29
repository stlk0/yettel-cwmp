//! Terminal presentation. Every dynamic value is sanitized before rendering.
use crate::i18n::{self, t, tf};
use crate::sanitize::safe;
use crate::ui::{
    app::{App, Screen},
    form::Field,
    theme::{self, selection_list, usable},
};
use crossterm::event::KeyCode;
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    text::{Line, Span, Text},
    widgets::{
        Block, Borders, ListState, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap,
    },
};
use std::{path::Path, time::SystemTime};
mod details;

impl App {
    /// Draw the current screen, including masked secrets and navigation hints.
    pub fn render(&mut self, frame: &mut Frame, root: &Path) {
        let area = frame.area();
        let [header, content, footer] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(
                if matches!(self.screen, Screen::Create | Screen::ChangeKey(_)) {
                    2
                } else {
                    1
                },
            ),
        ])
        .areas(area);
        let heading = format!(
            "{}  |  {}",
            t("common.header"),
            self.device.template.short_name
        );
        #[cfg(feature = "dev-provider-override")]
        let heading = format!("{heading}  {}", crate::dev::banner());
        frame.render_widget(Paragraph::new(heading).style(theme::title()), header);
        let border = Block::default().borders(Borders::ALL);
        let inner = border.inner(content);
        frame.render_widget(border, content);
        if !usable(area.as_size()) {
            frame.render_widget(
                Paragraph::new(t("common.too_small")).wrap(Wrap { trim: false }),
                inner,
            );
            frame.render_widget(Paragraph::new(t("common.small_footer")), footer);
            return;
        }
        if self.help_open {
            let body = tf(
                "help.body",
                &[
                    ("path", &safe(&root.to_string_lossy())),
                    ("device", &self.device.template.short_name),
                    ("url", i18n::README_URL),
                ],
            );
            let mut lines = vec![Line::from(t("help.title")), Line::from("")];
            lines.extend(body.lines().map(|line| Line::from(line.to_owned())));
            self.max_scroll = wrapped_line_count(&lines, inner.width as usize)
                .saturating_sub(inner.height as usize)
                .try_into()
                .unwrap_or(u16::MAX);
            self.scroll = self.scroll.min(self.max_scroll);
            frame.render_widget(
                Paragraph::new(Text::from(lines))
                    .wrap(Wrap { trim: false })
                    .scroll((self.scroll, 0)),
                inner,
            );
            frame.render_widget(Paragraph::new(self.footer()), footer);
            return;
        }
        match &self.screen {
            Screen::Profiles => self.render_profiles(frame, inner),
            Screen::Create | Screen::ChangeKey(_) => self.render_form(frame, inner),
            _ => self.render_details_and_menu(frame, inner),
        }
        let mut footer_text = self.footer();
        if self
            .copied_at
            .is_some_and(|when| when.elapsed().as_secs() < 3)
            && matches!(self.screen, Screen::Result { .. })
        {
            footer_text = t("result.copied").into();
        }
        frame.render_widget(Paragraph::new(footer_text).style(theme::key()), footer);
    }
    fn render_profiles(&mut self, frame: &mut Frame, area: Rect) {
        let title_len = if self.profiles.is_empty() { 9 } else { 2 };
        let [title, list] =
            Layout::vertical([Constraint::Length(title_len), Constraint::Min(1)]).areas(area);
        if self.profiles.is_empty() {
            frame.render_widget(
                Paragraph::new(format!("{}\n\n{}", t("welcome.title"), t("welcome.body")))
                    .wrap(Wrap { trim: false }),
                title,
            );
        } else {
            frame.render_widget(
                Paragraph::new(t("profiles.title")).style(theme::title()),
                title,
            );
        }
        let mut items: Vec<String> =
            self.profiles
                .iter()
                .map(|serial| {
                    let status =
                        if self.store.as_ref().is_some_and(|store| {
                            store.export_modified(serial).ok().flatten().is_some()
                        }) {
                            t("profiles.saved")
                        } else {
                            t("profiles.not_received")
                        };
                    format!("{}  ·  {}", safe(serial), status)
                })
                .collect();
        items.extend(self.menu().iter().map(|hint| hint.display()));
        frame.render_stateful_widget(
            selection_list(items.iter().map(String::as_str)),
            list,
            &mut self.profile_list,
        );
    }
    fn render_form(&self, frame: &mut Frame, area: Rect) {
        let change = matches!(self.screen, Screen::ChangeKey(_));
        let [title, explanation, fields, hint, error] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(if change { 2 } else { 0 }),
            Constraint::Length(if change { 1 } else { 3 }),
            Constraint::Length(2),
            Constraint::Min(1),
        ])
        .areas(area);
        frame.render_widget(
            Paragraph::new(if change {
                t("form.change.title")
            } else {
                t("form.add.title")
            })
            .style(theme::title()),
            title,
        );
        if change {
            frame.render_widget(
                Paragraph::new(t("form.change.body")).wrap(Wrap { trim: false }),
                explanation,
            );
        }
        let serial = field_tail(
            &safe(&self.form.serial),
            fields.width.saturating_sub(17) as usize,
        );
        let mac = safe(&self.form.mac);
        let key = if self.form.reveal {
            safe(&self.form.key)
        } else {
            "•".repeat(self.form.key.chars().count().min(40))
        };
        let lines = if change {
            vec![tf(
                "form.key",
                &[(
                    "value",
                    &field_tail(&key, fields.width.saturating_sub(13) as usize),
                )],
            )]
        } else {
            vec![
                tf("form.serial", &[("value", &serial)]),
                tf("form.mac", &[("value", &mac)]),
                tf(
                    "form.key",
                    &[(
                        "value",
                        &field_tail(&key, fields.width.saturating_sub(13) as usize),
                    )],
                ),
            ]
        };
        let selected = if change {
            0
        } else {
            match self.form.field {
                Field::Serial => 0,
                Field::Mac => 1,
                Field::Key => 2,
            }
        };
        frame.render_stateful_widget(
            selection_list(lines.iter().map(String::as_str)),
            fields,
            &mut ListState::default().with_selected(Some(selected)),
        );
        let field_hint = if change {
            t("form.key.hint").to_owned()
        } else {
            match self.form.field {
                Field::Serial => tf(
                    "form.serial.hint",
                    &[("prefix", self.device.serial_prefix())],
                ),
                Field::Mac => t("form.mac.hint").to_owned(),
                Field::Key => t("form.key.hint").to_owned(),
            }
        };
        frame.render_widget(
            Paragraph::new(field_hint)
                .wrap(Wrap { trim: false })
                .style(theme::muted()),
            hint,
        );
        self.render_form_error(frame, error);
        let current = &lines[selected];
        let cursor_col = Span::raw(current.as_str())
            .width()
            .min(fields.width.saturating_sub(2) as usize);
        frame.set_cursor_position((
            fields
                .x
                .saturating_add(2 + u16::try_from(cursor_col).unwrap_or(u16::MAX)),
            fields
                .y
                .saturating_add(u16::try_from(selected).unwrap_or(u16::MAX)),
        ));
    }
    fn render_form_error(&self, frame: &mut Frame, area: Rect) {
        if let Some(error_value) = &self.form.error {
            let message = i18n::error_message(*error_value);
            frame.render_widget(
                Paragraph::new(message)
                    .wrap(Wrap { trim: false })
                    .style(theme::error()),
                area,
            );
        }
    }

    fn render_details_and_menu(&mut self, frame: &mut Frame, area: Rect) {
        let hints = self.menu();
        let menu_height = u16::try_from(hints.len())
            .unwrap_or(u16::MAX)
            .min(area.height.saturating_sub(3));
        let [details_area, menu_area] =
            Layout::vertical([Constraint::Min(1), Constraint::Length(menu_height)]).areas(area);
        let lines = self.detail_lines();
        let available = details_area.width.saturating_sub(1).max(1) as usize;
        self.max_scroll = wrapped_line_count(&lines, available)
            .saturating_sub(details_area.height as usize)
            .try_into()
            .unwrap_or(u16::MAX);
        self.scroll = self.scroll.min(self.max_scroll);
        frame.render_widget(
            Paragraph::new(Text::from(lines))
                .wrap(Wrap { trim: false })
                .scroll((self.scroll, 0)),
            details_area,
        );
        if self.max_scroll > 0 {
            let mut state =
                ScrollbarState::new(self.max_scroll as usize + 1).position(self.scroll as usize);
            frame.render_stateful_widget(
                Scrollbar::new(ScrollbarOrientation::VerticalRight),
                details_area,
                &mut state,
            );
        }
        let items: Vec<String> = hints
            .iter()
            .map(|hint| {
                if let Screen::Result { reveal: true, .. } = self.screen
                    && hint.code == KeyCode::Char('s')
                {
                    return format!("{} (S)", t("result.hide"));
                }
                hint.display()
            })
            .collect();
        frame.render_stateful_widget(
            selection_list(items.iter().map(String::as_str)),
            menu_area,
            &mut self.menu_list,
        );
    }
}

fn field_tail(value: &str, width: usize) -> String {
    if value.chars().count() <= width {
        return value.into();
    }
    format!(
        "…{}",
        value
            .chars()
            .skip(
                value
                    .chars()
                    .count()
                    .saturating_sub(width.saturating_sub(1))
            )
            .collect::<String>()
    )
}

/// Local calendar date of a saved file, for display.
fn saved_date(value: SystemTime) -> String {
    let offset = super::LOCAL_OFFSET
        .get()
        .copied()
        .unwrap_or(time::UtcOffset::UTC);
    let date = time::OffsetDateTime::from(value).to_offset(offset).date();
    format!(
        "{:04}-{:02}-{:02}",
        date.year(),
        u8::from(date.month()),
        date.day()
    )
}

fn wrapped_line_count(lines: &[Line<'_>], width: usize) -> usize {
    let width = width.max(1);
    lines
        .iter()
        .map(|line| {
            let value = line.to_string();
            if value.is_empty() {
                return 1;
            }
            let mut rows = 1;
            let mut occupied = 0;
            for word in value.split_whitespace() {
                let size = Span::raw(word).width();
                let separator = usize::from(occupied > 0);
                if occupied + separator + size > width && occupied > 0 {
                    rows += 1;
                    occupied = 0;
                }
                if size > width {
                    rows += size.saturating_sub(1) / width;
                    occupied = size % width;
                } else {
                    occupied += separator + size;
                }
            }
            rows
        })
        .sum()
}
