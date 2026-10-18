// Disables the terminal window. Use this for releases, and disable when debugging.
#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

use std::{
    path::PathBuf,
    sync::{Arc, LazyLock},
};

use egui::Context;
use software_manager::{
    Application, Error, Installation, Progress, Release, Task, Updater, install, installed,
    is_running, latest_release, uninstall,
};

use crate::ui::{WINDOW_HEIGHT, WINDOW_WIDTH};

mod ui;

/// This program's version, taken from Cargo.toml's `[package] version`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// What's installed, whether it's running, and the latest release.
struct AppStatus {
    installed: Option<Installation>,
    running: bool,
    latest: Result<Release, Error>,
}

/// An install, update, or uninstall underway.
enum Operation {
    Install {
        release: Release,
        progress: Arc<Progress>,
        task: Task<()>,
    },
    Uninstall(Task<Option<PathBuf>>),
}

/// One application's row in the list.
struct AppRow {
    app: Application,
    /// `None` until the first check finishes.
    status: Option<AppStatus>,
    refresh: Option<Task<AppStatus>>,
    operation: Option<Operation>,
    /// How the last operation went: what happened, or what went wrong.
    outcome: Option<Result<String, Error>>,
    /// The Uninstall button was clicked, and is waiting for confirmation.
    confirm_uninstall: bool,
}

impl AppRow {
    fn new(app: Application) -> Self {
        Self {
            app,
            status: None,
            refresh: None,
            operation: None,
            outcome: None,
            confirm_uninstall: false,
        }
    }

    fn busy(&self) -> bool {
        self.refresh.is_some() || self.operation.is_some()
    }

    fn refresh(&mut self, ctx: &Context) {
        let app = self.app;

        let work = move || {
            Ok(AppStatus {
                installed: installed(app),
                running: is_running(app),
                latest: latest_release(app),
            })
        };

        self.refresh = Some(Task::spawn(work, repaint(ctx)));
    }

    fn install(&mut self, release: Release, ctx: &Context) {
        let progress = Arc::new(Progress::default());

        let task = {
            let release = release.clone();
            let progress = Arc::clone(&progress);
            Task::spawn(move || install(&release, &progress), repaint(ctx))
        };

        self.outcome = None;
        self.operation = Some(Operation::Install {
            release,
            progress,
            task,
        });
    }

    fn uninstall(&mut self, ctx: &Context) {
        let app = self.app;

        self.outcome = None;
        self.confirm_uninstall = false;
        self.operation = Some(Operation::Uninstall(Task::spawn(
            move || uninstall(app),
            repaint(ctx),
        )));
    }

    /// Pick up the results of background work.
    fn poll(&mut self, ctx: &Context) {
        if let Some(task) = &mut self.refresh
            && let Some(result) = task.poll()
        {
            self.refresh = None;

            match result {
                Ok(status) => self.status = Some(status),
                Err(e) => self.outcome = Some(Err(e)),
            }
        }

        let outcome = match &mut self.operation {
            Some(Operation::Install { release, task, .. }) => task.poll().map(|result| {
                result.map(|()| format!("Installed {}.", release.version))
            }),
            Some(Operation::Uninstall(task)) => task.poll().map(|result| {
                result.map(|kept| match kept {
                    Some(dir) => format!(
                        "Uninstalled. Its data was kept, in {}.",
                        dir.display()
                    ),
                    None => "Uninstalled.".to_owned(),
                })
            }),
            None => None,
        };

        if let Some(outcome) = outcome {
            self.operation = None;
            self.outcome = Some(outcome);

            // What's installed has changed.
            self.refresh(ctx);
        }
    }
}

pub struct State {
    /// Every application except this one.
    apps: Vec<AppRow>,
    /// This one, which can't replace its own files while running, so updates the way the other
    /// applications do from their About popups: the update installs once it closes.
    self_update: Updater,
}

impl State {
    fn new() -> Self {
        let apps = Application::ALL
            .into_iter()
            .filter(|app| *app != Application::SoftwareManager)
            .map(AppRow::new)
            .collect();

        Self {
            apps,
            self_update: Updater::default(),
        }
    }

    fn busy(&self) -> bool {
        self.apps.iter().any(AppRow::busy) || self.self_update.busy()
    }

    /// Check what's installed, and for new releases. GitHub allows 60 of these checks an hour,
    /// without signing in, so this runs on startup and when asked; not periodically.
    fn refresh(&mut self, ctx: &Context) {
        for row in &mut self.apps {
            if !row.busy() {
                row.refresh(ctx);
            }
        }

        if !self.self_update.busy() {
            self.self_update
                .check(Application::SoftwareManager, VERSION, repaint(ctx));
        }
    }

    fn poll(&mut self, ctx: &Context) {
        for row in &mut self.apps {
            row.poll(ctx);
        }
        self.self_update.poll();
    }
}

/// For background tasks to wake the UI when they finish.
fn repaint(ctx: &Context) -> impl FnOnce() + Send + 'static {
    let ctx = ctx.clone();
    move || ctx.request_repaint()
}

impl eframe::App for State {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        ui::draw(self, ui);
    }
}

fn app_icon() -> Arc<egui::IconData> {
    static ICON: LazyLock<Arc<egui::IconData>> = LazyLock::new(|| {
        let png = include_bytes!("../resources/icon.png");
        Arc::new(eframe::icon_data::from_png_bytes(png).expect("resources/icon.png is a valid PNG"))
    });
    Arc::clone(&ICON)
}

fn main() {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([WINDOW_WIDTH, WINDOW_HEIGHT])
            .with_icon(app_icon()),
        ..Default::default()
    };

    eframe::run_native(
        "Software Manager",
        options,
        Box::new(|cc| {
            let mut state = State::new();
            state.refresh(&cc.egui_ctx);

            Ok(Box::new(state))
        }),
    )
    .unwrap();
}
