//! The library for checking the application version, and downloading / installing the latest versions.

use std::{fmt::{self, Display}, io, path::Path, process::Command};
use eframe::App;
use ureq;

#[cfg(target_os = "windows")]
pub const OS: Os =  Os::Windows;

#[cfg(target_os = "unix")]
pub const OS: Os =  Os::Linux;


#[derive(Clone, Copy, PartialEq)]
pub enum Os {
    Windows,
    Linux
}

impl Os {
    fn release_name(self) -> String {
        match self {
            Self::Windows => "win",
            Self::Linux => "linux"
        }.to_string()
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum Application {
    Molchanica,
    PlasCad,
    Peregrine,
    Shell,
    SoftwareManager,
}

impl Application {
    pub fn release_file_name(self) -> String {
        match self {
            Self::Molchanica => "molchanica",
            Self::PlasCad => "plascad",
            Self::Peregrine => "peregrine",
            Self::Shell => "shell",
            Self::SoftwareManager => "software_manager",
        }.to_string()
    }

    pub fn repo_name(self) -> String {
        self.release_file_name() // Until one of these is different.
    }


    /// E.g. from Github releases, but doesn't have to be
    pub fn release_url(self, version: (u8, u8, u8)) -> String {
        let version_dot = format!("{}.{}.{}", version.0, version.1, version.2);
        let version_underscore = format!("{}_{}_{}", version.0, version.1, version.2);

        format!(
            "https://github.com/David-OConnor/{}/releases/download/{version_dot}/{}_{version_underscore}_{}.zip",
            self.repo_name(),
            self.release_file_name(),
            OS.release_name(),
        )
    }
}

impl Display for Application {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let v = match self {
            Self::Molchanica => "Molchanica",
            Self::PlasCad => "PlasCAD",
            Self::Peregrine => "Peregrine",
            Self::Shell => "Shell",
            Self::SoftwareManager => "SoftwareManager",
        };

        write!(f, "{v}")
    }
}

/// Semvar. None if not installed.
pub fn check_version(app: Application) -> Option<(u8, u8, u8)> {
    None
}

pub fn download_latest(app: Application, path: &Path) -> io::Result<()> {
    let version = 0.; // todo: Get the latest version from github. How?

    let resp = ureq::get(app.release_url(), version, os); // todo syntax placeholder

    resp.save(path); // todo syntax placeholder

    Ok(())
}

pub fn install(app: Application, release_archive: &Path) -> io::Result<()> {
    unzip(release_archive)?;

    let setup_script = match OS {
        Os::Windows => {}
        Os::Linux => {}
    }

    Command::new(release_archive) // todo a/r

    Ok(())
}

