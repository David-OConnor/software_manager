//! Running work off the UI thread, and the check, download, and install steps an application goes
//! through to update itself from its About popup.

use std::{
    any::Any,
    fmt,
    panic::{self, AssertUnwindSafe},
    sync::{Arc, Mutex, PoisonError},
    thread,
};

use crate::{
    Application, Error, Release, check_for_update,
    install::{Progress, StagedUpdate, stage_update},
};

/// Work running on a background thread, so the UI stays responsive. Check it with [`Task::poll`]
/// each frame.
pub struct Task<T> {
    result: Arc<Mutex<Option<Result<T, Error>>>>,
}

impl<T: Send + 'static> Task<T> {
    /// Run `work` on a new thread. `on_done` runs after it finishes, on that thread: pass something
    /// that wakes the UI, e.g. `move || ctx.request_repaint()`, so the result is noticed without
    /// waiting for input.
    pub fn spawn(
        work: impl FnOnce() -> Result<T, Error> + Send + 'static,
        on_done: impl FnOnce() + Send + 'static,
    ) -> Self {
        let result = Arc::new(Mutex::new(None));
        let result_thread = Arc::clone(&result);

        thread::spawn(move || {
            // Caught, so that a panic shows as an error rather than as work that never finishes.
            let value = panic::catch_unwind(AssertUnwindSafe(work))
                .unwrap_or_else(|payload| Err(Error::Internal(panic_message(&*payload))));

            *result_thread.lock().unwrap_or_else(PoisonError::into_inner) = Some(value);
            on_done();
        });

        Self { result }
    }

    /// The result, once the work is done. It's returned once; after that, this returns `None`.
    pub fn poll(&mut self) -> Option<Result<T, Error>> {
        self.result
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }
}

impl<T> fmt::Debug for Task<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Task")
    }
}

fn panic_message(payload: &(dyn Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        s.to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "a background task panicked".to_owned()
    }
}

/// Where an [`Updater`] is in checking for, downloading, and installing an update.
#[derive(Debug, Default)]
pub enum UpdateStatus {
    /// Not checked yet.
    #[default]
    Idle,
    Checking(Task<Option<Release>>),
    UpToDate,
    Available(Release),
    Downloading {
        release: Release,
        progress: Arc<Progress>,
        task: Task<StagedUpdate>,
    },
    /// Downloaded, and ready to install once the application closes.
    Ready(StagedUpdate),
    /// Installing once the application closes has started, so it should now close.
    Installing,
    Failed(Error),
}

/// Checks for, downloads, and installs a newer release of the running application, e.g. from its
/// About popup.
///
/// Keep one in the UI state. Each frame it's shown, call [`Updater::poll`], then draw from
/// [`Updater::status`]: offer [`Updater::check`] when idle, [`Updater::download`] when an update is
/// available, and [`Updater::install_on_exit`] once it's downloaded.
#[derive(Debug, Default)]
pub struct Updater {
    status: UpdateStatus,
}

impl Updater {
    pub fn status(&self) -> &UpdateStatus {
        &self.status
    }

    /// Whether a check or download is underway. While it is, request repaints, e.g. so the
    /// download progress updates.
    pub fn busy(&self) -> bool {
        matches!(
            self.status,
            UpdateStatus::Checking(_) | UpdateStatus::Downloading { .. }
        )
    }

    /// Start checking for a release newer than `current_version`: normally the application's
    /// `env!("CARGO_PKG_VERSION")`. See [`Task::spawn`] for `on_done`.
    pub fn check(
        &mut self,
        app: Application,
        current_version: &str,
        on_done: impl FnOnce() + Send + 'static,
    ) {
        let current_version = current_version.to_owned();

        let task = Task::spawn(move || check_for_update(app, &current_version), on_done);
        self.status = UpdateStatus::Checking(task);
    }

    /// Start downloading the available update. Does nothing unless one is available.
    pub fn download(&mut self, on_done: impl FnOnce() + Send + 'static) {
        let UpdateStatus::Available(release) = &self.status else {
            return;
        };

        let release = release.clone();
        let progress = Arc::new(Progress::default());

        let task = {
            let release = release.clone();
            let progress = Arc::clone(&progress);
            Task::spawn(move || stage_update(&release, &progress), on_done)
        };

        self.status = UpdateStatus::Downloading {
            release,
            progress,
            task,
        };
    }

    /// Advance to the next status, once background work finishes.
    pub fn poll(&mut self) {
        let next = match &mut self.status {
            UpdateStatus::Checking(task) => match task.poll() {
                Some(Ok(Some(release))) => UpdateStatus::Available(release),
                Some(Ok(None)) => UpdateStatus::UpToDate,
                Some(Err(e)) => UpdateStatus::Failed(e),
                None => return,
            },
            UpdateStatus::Downloading { task, .. } => match task.poll() {
                Some(Ok(staged)) => UpdateStatus::Ready(staged),
                Some(Err(e)) => UpdateStatus::Failed(e),
                None => return,
            },
            _ => return,
        };

        self.status = next;
    }

    /// Start installing the downloaded update once this process exits; the application then
    /// reopens. Returns `true` if that started, in which case close the application right away.
    /// Otherwise, the status is now [`UpdateStatus::Failed`].
    pub fn install_on_exit(&mut self) -> bool {
        let UpdateStatus::Ready(staged) = &self.status else {
            return false;
        };

        match staged.install_on_exit() {
            Ok(()) => {
                self.status = UpdateStatus::Installing;
                true
            }
            Err(e) => {
                self.status = UpdateStatus::Failed(e);
                false
            }
        }
    }
}
