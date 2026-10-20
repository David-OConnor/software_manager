//! Copying an application's latest development build, from its project folder, over the installed
//! executable. Only the executable is copied; nothing is built.

use std::{
    fs,
    path::{Path, PathBuf},
    time::SystemTime,
};

use crate::{
    Application, Error, Version,
    detect::{embedded_version, forget_version, is_running, record_version},
    install::replace_exe,
};

/// A release build in an application's project folder.
#[derive(Clone, Debug)]
pub struct ProjectBuild {
    pub app: Application,
    pub exe: PathBuf,
    /// Read from the executable on Windows. Otherwise, e.g. on Linux, from the project's
    /// Cargo.toml, which may have changed since the build.
    pub version: Option<Version>,
    /// When it was built.
    pub modified: Option<SystemTime>,
}

/// The release build in an application's project folder, `target/release/<exe>`, if there is one
/// for this OS. Debug builds are ignored. See [`Application::project_dir`].
pub fn project_build(app: Application) -> Option<ProjectBuild> {
    // Nowhere to copy it to, e.g. on macOS.
    app.install_dir()?;

    let dir = app.project_dir()?;
    let exe = dir.join("target").join("release").join(app.exe_name());

    let metadata = fs::metadata(&exe).ok()?;
    if !metadata.is_file() {
        return None;
    }

    let version = embedded_version(&exe).or_else(|| cargo_version(&dir));

    Some(ProjectBuild {
        app,
        exe,
        version,
        modified: metadata.modified().ok(),
    })
}

/// The `[package] version` in a project's Cargo.toml.
fn cargo_version(project_dir: &Path) -> Option<Version> {
    let manifest = fs::read_to_string(project_dir.join("Cargo.toml")).ok()?;

    let mut in_package = false;

    for line in manifest.lines() {
        let line = line.trim();

        if line.starts_with('[') {
            in_package = line == "[package]";
        } else if in_package
            && let Some(rest) = line.strip_prefix("version")
            && rest.trim_start().starts_with('=')
        {
            // E.g. `version = "0.4.3"`. Not `version.workspace = true`, which has no version here.
            return Version::parse(rest.split('"').nth(1)?);
        }
    }

    None
}

/// Copy a project's release build over the installed executable, or install it there if there
/// isn't one. Only the executable is copied: not setup scripts, libraries, or Start menu entries.
///
/// Executables can be large, so run this off the UI thread, e.g. with [`crate::Task`].
pub fn install_from_project(build: &ProjectBuild) -> Result<(), Error> {
    let app = build.app;

    // Shell, and this program, can be replaced while running; see `replace_exe`. The others'
    // setup scripts refuse to, so this does too.
    let replace_running = matches!(app, Application::Shell | Application::SoftwareManager);

    if !replace_running && is_running(app) {
        return Err(Error::Running(app));
    }

    let exe = app.exe_path().ok_or(Error::UnsupportedOs)?;
    fs::create_dir_all(exe.parent().unwrap())?;

    // Copied beside the target first, so an interrupted copy never leaves a partial executable in
    // its place, and moving it into place is a rename.
    let new = exe.with_file_name(format!("{}.new", app.exe_name()));
    fs::copy(&build.exe, &new)?;

    replace_exe(&new, &exe)?;

    // An earlier record no longer matches the executable, so would be ignored anyway.
    match build.version {
        Some(version) => record_version(app, &exe, version),
        None => forget_version(app),
    }

    Ok(())
}
