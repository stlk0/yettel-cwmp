//! Detail text for running, result, and error screens.
use super::saved_date;
use crate::{
    capture::Outcome,
    error::Error,
    i18n::{self, t, tf},
    progress::Stage,
    sanitize::safe,
    ui::{
        app::{App, Screen},
        theme,
    },
};
use ratatui::{style::Style, text::Line};

impl App {
    fn result_lines(&self, outcome: &Outcome, reveal: bool, fresh: bool) -> Vec<String> {
        let settings = &outcome.export.internet;
        let config = &settings.config;
        vec![
            if fresh {
                t("result.done").into()
            } else {
                tf(
                    "result.saved",
                    &[(
                        "date",
                        &self
                            .export_modified
                            .map(saved_date)
                            .unwrap_or_else(|| t("result.unknown_date").into()),
                    )],
                )
            },
            String::new(),
            tf(
                "result.connection_type",
                &[("protocol", &config.protocol.to_string())],
            ),
            tf(
                "result.username",
                &[(
                    "value",
                    &if reveal {
                        safe(settings.username.expose())
                    } else {
                        t("result.masked").into()
                    },
                )],
            ),
            tf(
                "result.password",
                &[(
                    "value",
                    &if reveal {
                        safe(settings.password.expose())
                    } else {
                        t("result.masked").into()
                    },
                )],
            ),
            tf("result.vlan", &[("vlan", &config.vlan_id.to_string())]),
            tf("result.mtu", &[("mtu", &config.mtu.to_string())]),
            String::new(),
            t("result.preset_note").into(),
            String::new(),
            t("result.setup.title").into(),
            t("result.setup.one").into(),
            t("result.setup.two").into(),
            tf(
                "result.setup.three",
                &[("vlan", &config.vlan_id.to_string())],
            ),
            tf("result.setup.four", &[("mtu", &config.mtu.to_string())]),
            String::new(),
            tf(
                "result.iptv",
                &[(
                    "url",
                    &format!("{}#tv--iptv", i18n::README_URL.trim_end_matches("#readme")),
                )],
            ),
            String::new(),
            tf(
                "result.saved_to",
                &[("path", &safe(&outcome.path.to_string_lossy()))],
            ),
        ]
    }

    fn error_lines(screen: &Screen) -> Vec<String> {
        let Screen::Error {
            error, progress, ..
        } = screen
        else {
            return Vec::new();
        };
        let stage = match progress.stage {
            Stage::Profile => t("stage.profile"),
            Stage::Inform => t("stage.inform"),
            Stage::Session => t("stage.session"),
            Stage::Export => t("stage.export"),
        };
        vec![
            i18n::error_message(*error),
            String::new(),
            tf("error.code", &[("code", error.code())]),
            tf(
                "error.details",
                &[("stage", stage), ("rpc", &format!("{:?}", progress.rpc))],
            ),
        ]
    }

    fn styled_lines(&self, mut lines: Vec<String>) -> Vec<Line<'static>> {
        let mut output = Vec::new();
        for (index, line) in lines.drain(..).enumerate() {
            for part in line.split('\n') {
                let style = if index == 0 {
                    match &self.screen {
                        Screen::Result { fresh: true, .. } => theme::success(),
                        Screen::Error {
                            error: Error::Cancelled,
                            ..
                        } => theme::warning(),
                        Screen::Error { .. } => theme::error(),
                        Screen::ConnectNotice(_)
                        | Screen::ReplaceSaved(_)
                        | Screen::DeleteConfirm { .. } => theme::warning(),
                        _ => theme::title(),
                    }
                } else {
                    Style::default()
                };
                output.push(Line::styled(part.to_owned(), style));
            }
        }
        output
    }

    pub(super) fn detail_lines(&self) -> Vec<Line<'static>> {
        let lines = match &self.screen {
            Screen::Device(profile) => vec![
                tf("device.title", &[("serial", profile.serial.as_ref())]),
                tf("form.mac", &[("value", &profile.router_mac.to_string())]),
                self.export_modified.map_or_else(
                    || t("device.not_received").into(),
                    |date| tf("device.saved", &[("date", &saved_date(date))]),
                ),
            ],
            Screen::DeleteConfirm { serial, .. } => vec![
                t("device.delete.title").into(),
                String::new(),
                tf("device.delete.body", &[("serial", &safe(serial))]),
            ],
            Screen::ReplaceSaved(_) => vec![
                t("device.replace.title").into(),
                String::new(),
                t("device.replace.body").into(),
            ],
            Screen::ConnectNotice(profile) => vec![
                t("connect.title").into(),
                String::new(),
                tf(
                    "connect.body",
                    &[("serial", &safe(profile.serial.as_ref()))],
                ),
            ],
            Screen::Running {
                progress,
                cancelling,
                started,
                ..
            } => {
                let step = match progress.stage {
                    Stage::Profile | Stage::Inform => 0,
                    Stage::Session => 1,
                    Stage::Export => 2,
                };
                let labels = [
                    t("running.connect"),
                    t("running.receive"),
                    t("running.save"),
                ];
                let spinner = ['|', '/', '-', '\\'][self.spinner % 4];
                let mut lines = vec![t("running.title").into(), String::new()];
                for (index, label) in labels.iter().enumerate() {
                    let marker = if index < step {
                        "[x]".into()
                    } else if index == step {
                        format!("[>] {spinner}")
                    } else {
                        "[ ]".into()
                    };
                    lines.push(format!("{marker} {label}"));
                }
                let seconds = started.elapsed().as_secs();
                lines.extend([
                    String::new(),
                    tf(
                        "running.elapsed",
                        &[("time", &format!("{}:{:02}", seconds / 60, seconds % 60))],
                    ),
                    t("running.wait").into(),
                ]);
                if *cancelling {
                    lines.push(t("running.cancelling").into());
                }
                lines
            }
            Screen::Result {
                outcome,
                reveal,
                fresh,
                ..
            } => self.result_lines(outcome, *reveal, *fresh),
            Screen::Error { .. } => Self::error_lines(&self.screen),
            Screen::Profiles | Screen::Create | Screen::ChangeKey(_) => Vec::new(),
        };
        self.styled_lines(lines)
    }
}
