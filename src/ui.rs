use std::time::Duration;

use egui::{
    Align, Button, CentralPanel, Color32, Context, Grid, Label, Layout, ProgressBar, RichText, Ui,
    ViewportCommand,
};
use software_manager::{Application, Error, Progress, UpdateStatus, Updater};

use crate::{AppRow, Operation, State, VERSION, repaint};

pub const WINDOW_WIDTH: f32 = 800.;
pub const WINDOW_HEIGHT: f32 = 600.;

const COLOR_OK: Color32 = Color32::from_rgb(120, 200, 120);
const COLOR_WARNING: Color32 = Color32::ORANGE;

/// Keeps notes and errors from widening the last column past the window.
const ACTIONS_WIDTH: f32 = 400.;

/// How often to repaint while background work runs, so progress bars move.
const PROGRESS_REPAINT: Duration = Duration::from_millis(100);

/// The most lines of a failed setup script's output to show.
const MAX_ERROR_LINES: usize = 8;

pub fn draw(state: &mut State, ui: &mut Ui) {
    let ctx = ui.ctx().clone();

    state.poll(&ctx);
    if state.busy() {
        ctx.request_repaint_after(PROGRESS_REPAINT);
    }

    CentralPanel::default().show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.heading(RichText::new("Software applications"));

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let refresh = ui
                    .add_enabled(!state.busy(), Button::new("Refresh"))
                    .on_hover_text("Check what's installed, and for new releases.");

                if refresh.clicked() {
                    state.refresh(&ctx);
                }
            });
        });

        ui.add_space(20.);

        Grid::new("apps")
            .num_columns(4)
            .spacing([24., 14.])
            .striped(true)
            .show(ui, |ui| {
                ui.label(RichText::new("Application").strong());
                ui.label(RichText::new("Installed").strong());
                ui.label(RichText::new("Latest").strong());
                ui.label("");
                ui.end_row();

                for row in &mut state.apps {
                    app_row(row, &ctx, ui);
                    ui.end_row();
                }

                self_row(&mut state.self_update, &ctx, ui);
                ui.end_row();
            });

        ui.add_space(20.);

        ui.label(
            RichText::new(
                "Applications are installed for the current user, without administrator rights. \
                Uninstalling keeps their preferences and data.",
            )
            .weak()
            .small(),
        );
    });
}

fn name_cell(app: Application, ui: &mut Ui) {
    ui.label(RichText::new(app.to_string()).strong())
        .on_hover_text(app.description());
}

/// The latest release's version, linking to its notes.
fn latest_cell(release: Option<&software_manager::Release>, ui: &mut Ui) {
    match release {
        Some(release) => {
            ui.hyperlink_to(release.version.to_string(), &release.page_url)
                .on_hover_text("Open the release notes in a web browser.");
        }
        None => {
            ui.label(RichText::new("—").weak());
        }
    }
}

fn app_row(row: &mut AppRow, ctx: &Context, ui: &mut Ui) {
    name_cell(row.app, ui);

    let Some(status) = &row.status else {
        // The first check is underway.
        ui.spinner();
        ui.label("");
        ui.label("");
        return;
    };

    match &status.installed {
        Some(installation) => {
            let text = match installation.version {
                Some(version) => RichText::new(version.to_string()),
                None => RichText::new("Unknown version").weak(),
            };

            ui.label(text)
                .on_hover_text(installation.exe.display().to_string());
        }
        None => {
            ui.label(RichText::new("Not installed").weak());
        }
    }

    latest_cell(status.latest.as_ref().ok(), ui);

    let mut install = None;
    let mut uninstall = false;

    ui.vertical(|ui| {
        ui.set_max_width(ACTIONS_WIDTH);

        match &row.operation {
            Some(Operation::Install { progress, .. }) => {
                if progress.downloaded() {
                    busy_label("Installing…", ui);
                } else {
                    download_progress(progress, ui);
                }
                return;
            }
            Some(Operation::Uninstall(_)) => {
                busy_label("Uninstalling…", ui);
                return;
            }
            None => (),
        }

        // Shell can be updated while it's running; see `install::replace_exe`. The others'
        // setup scripts refuse to.
        let blocked = status.running && row.app != Application::Shell;
        let enabled = row.refresh.is_none() && !blocked;

        ui.horizontal(|ui| {
            ui.add_enabled_ui(enabled, |ui| {
                match (&status.latest, &status.installed) {
                    (Ok(release), None) => {
                        let button = ui
                            .button(format!("Install {}", release.version))
                            .on_hover_text(format!("Download {}, and install it.", mb(release.size)));

                        if button.clicked() {
                            install = Some(release.clone());
                        }
                    }
                    (Ok(release), Some(installation)) => match installation.version {
                        Some(version) if version >= release.version => {
                            ui.label(RichText::new("✔ Up to date").color(COLOR_OK));
                        }
                        Some(_) => {
                            let button = ui
                                .button(format!("Update to {}", release.version))
                                .on_hover_text(format!(
                                    "Download {}, and install it.",
                                    mb(release.size)
                                ));

                            if button.clicked() {
                                install = Some(release.clone());
                            }
                        }
                        None => {
                            let button = ui
                                .button(format!("Install {}", release.version))
                                .on_hover_text(
                                    "The installed version is unknown. This installs the latest \
                                    release over it.",
                                );

                            if button.clicked() {
                                install = Some(release.clone());
                            }
                        }
                    },
                    (Err(e), _) => error_label(e, ui),
                }

                if status.installed.is_some()
                    && !row.confirm_uninstall
                    && ui.button("Uninstall").clicked()
                {
                    row.confirm_uninstall = true;
                }
            });
        });

        if blocked {
            ui.label(
                RichText::new(format!("{} is running. Close it to update or uninstall.", row.app))
                    .color(COLOR_WARNING)
                    .small(),
            );
        }

        if row.confirm_uninstall {
            ui.horizontal(|ui| {
                ui.label(format!("Uninstall {}?", row.app));

                let confirm = Button::new(RichText::new("Uninstall").color(Color32::WHITE))
                    .fill(Color32::DARK_RED);

                if ui.add_enabled(enabled, confirm).clicked() {
                    uninstall = true;
                }
                if ui.button("Cancel").clicked() {
                    row.confirm_uninstall = false;
                }
            });
        }

        match &row.outcome {
            Some(Ok(note)) => {
                ui.add(Label::new(RichText::new(note).color(COLOR_OK).small()).wrap());
            }
            Some(Err(e)) => error_label(e, ui),
            None => (),
        }
    });

    if let Some(release) = install {
        row.install(release, ctx);
    }
    if uninstall {
        row.uninstall(ctx);
    }
}

/// This program. It can't replace itself while running, so it downloads the update, then installs
/// it after closing, and reopens.
fn self_row(updater: &mut Updater, ctx: &Context, ui: &mut Ui) {
    name_cell(Application::SoftwareManager, ui);

    ui.label(VERSION).on_hover_text("This program.");

    let latest = match updater.status() {
        UpdateStatus::Available(release) | UpdateStatus::Downloading { release, .. } => {
            Some(release)
        }
        UpdateStatus::Ready(staged) => Some(&staged.release),
        _ => None,
    };

    match latest {
        Some(release) => latest_cell(Some(release), ui),
        None if matches!(updater.status(), UpdateStatus::UpToDate) => {
            ui.label(VERSION);
        }
        None => latest_cell(None, ui),
    }

    let mut download = false;
    let mut restart = false;

    ui.vertical(|ui| {
        ui.set_max_width(ACTIONS_WIDTH);

        match updater.status() {
            UpdateStatus::Idle => (),
            UpdateStatus::Checking(_) => {
                ui.spinner();
            }
            UpdateStatus::UpToDate => {
                ui.label(RichText::new("✔ Up to date").color(COLOR_OK));
            }
            UpdateStatus::Available(release) => {
                download = ui
                    .button(format!("Update to {}", release.version))
                    .on_hover_text(format!("Download {}.", mb(release.size)))
                    .clicked();
            }
            UpdateStatus::Downloading { progress, .. } => download_progress(progress, ui),
            UpdateStatus::Ready(_) => {
                restart = ui
                    .button("Restart to update")
                    .on_hover_text("Close Software Manager, install the update, then reopen it.")
                    .clicked();
            }
            UpdateStatus::Installing => busy_label("Installing…", ui),
            UpdateStatus::Failed(e) => error_label(e, ui),
        }
    });

    if download {
        updater.download(repaint(ctx));
    }

    if restart && updater.install_on_exit() {
        ctx.send_viewport_cmd(ViewportCommand::Close);
    }
}

fn busy_label(text: &str, ui: &mut Ui) {
    ui.horizontal(|ui| {
        ui.spinner();
        ui.label(text);
    });
}

fn download_progress(progress: &Progress, ui: &mut Ui) {
    let (downloaded, total) = progress.bytes();

    let text = if total > 0 {
        format!("Downloading… {} of {}", mb(downloaded), mb(total))
    } else {
        "Downloading…".to_owned()
    };

    ui.add(
        ProgressBar::new(progress.fraction())
            .text(text)
            .desired_width(280.),
    );
}

/// Errors that only mean there's nothing to install are shown plainly; others as warnings. A
/// failed setup script's output can be long; the end, where the reason is, is shown.
fn error_label(e: &Error, ui: &mut Ui) {
    let text = match e {
        Error::NoRelease | Error::NoAsset { .. } | Error::UnsupportedOs => {
            ui.label(RichText::new(e.to_string()).weak());
            return;
        }
        Error::Setup(_) => {
            let text = e.to_string();
            let lines: Vec<_> = text.lines().collect();

            if lines.len() > MAX_ERROR_LINES {
                format!("…\n{}", lines[lines.len() - MAX_ERROR_LINES..].join("\n"))
            } else {
                text
            }
        }
        _ => e.to_string(),
    };

    ui.add(
        Label::new(RichText::new(text).color(COLOR_WARNING).small()).wrap(),
    );
}

fn mb(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1_000_000.)
}
