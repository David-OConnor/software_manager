//! The library for checking the application version, and downloading / installing the latest versions.
//!
//! It's used two ways: by the Software Manager GUI (this crate's binary), which installs, updates,
//! and uninstalls each application; and by the applications themselves, which use [`Updater`] in
//! their About popups to check for, download, and install a newer release of themselves.
//!
//! Releases come from GitHub. Each is a zip of the executable and the setup scripts that the
//! application's `make_release` scripts package: `setup_<name>.ps1` on Windows, and
//! `setup_<name>.sh` on Linux. Installing runs that script, as a user would after extracting the
//! zip, so the result is the same either way. Shell is the exception: its releases are the bare
//! executable.

mod detect;
mod install;
mod updater;

use std::{
    fmt::{self, Display},
    io,
    path::PathBuf,
    time::Duration,
};

use serde::Deserialize;

pub use crate::{
    detect::{Installation, installed, is_running},
    install::{Progress, StagedUpdate, install, launch, stage_update, uninstall},
    updater::{Task, UpdateStatus, Updater},
};

/// The GitHub account the applications' repositories are under.
const GITHUB_OWNER: &str = "David-OConnor";

/// For the GitHub API request. Downloads have no overall limit, since releases can be large.
const API_TIMEOUT: Duration = Duration::from_secs(30);

#[cfg(target_os = "windows")]
pub const OS: Os = Os::Windows;

#[cfg(target_os = "macos")]
pub const OS: Os = Os::Mac;

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub const OS: Os = Os::Linux;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Os {
    Windows,
    Linux,
    /// We don't publish releases for Mac; this is so that applications using the library still
    /// build there.
    Mac,
}

impl Os {
    /// The OS part of a release archive's name, e.g. `win` in `molchanica_0_4_2_win.zip`. `None`
    /// if we don't publish releases for this OS.
    fn release_name(self) -> Option<&'static str> {
        match self {
            Self::Windows => Some("win"),
            Self::Linux => Some("linux"),
            Self::Mac => None,
        }
    }
}

impl Display for Os {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let v = match self {
            Self::Windows => "Windows",
            Self::Linux => "Linux",
            Self::Mac => "macOS",
        };

        write!(f, "{v}")
    }
}

/// How an application's releases are packaged.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Packaging {
    /// A zip of the executable, its setup scripts, and anything else it needs, named e.g.
    /// `molchanica_0_4_2_win.zip`.
    Zip,
    /// The executable alone, named e.g. `shell.exe`. The version comes from the release's tag.
    Binary,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Application {
    Molchanica,
    PlasCad,
    Peregrine,
    Shell,
    SoftwareManager,
}

impl Application {
    pub const ALL: [Self; 5] = [
        Self::Molchanica,
        Self::PlasCad,
        Self::Peregrine,
        Self::Shell,
        Self::SoftwareManager,
    ];

    /// The lower-case name used for the executable, release archives, and setup scripts.
    pub fn release_file_name(self) -> &'static str {
        match self {
            Self::Molchanica => "molchanica",
            Self::PlasCad => "plascad",
            Self::Peregrine => "peregrine",
            Self::Shell => "shell",
            Self::SoftwareManager => "software_manager",
        }
    }

    pub fn repo_name(self) -> &'static str {
        self.release_file_name() // Until one of these is different.
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Molchanica => "Molecule editing, visualization, and dynamics",
            Self::PlasCad => "Plasmid and primer design, PCR, and related tools",
            Self::Peregrine => "An integrated terminal and file browser",
            Self::Shell => "A terminal with bookmarks, history, and autocomplete",
            Self::SoftwareManager => "Installs and updates these applications",
        }
    }

    pub fn repo_url(self) -> String {
        format!("https://github.com/{GITHUB_OWNER}/{}", self.repo_name())
    }

    /// The executable's file name, e.g. `molchanica.exe` on Windows.
    pub fn exe_name(self) -> String {
        match OS {
            Os::Windows => format!("{}.exe", self.release_file_name()),
            _ => self.release_file_name().to_owned(),
        }
    }

    /// Whether the Software Manager offers to launch it. Shell is left out, since it runs inside a
    /// terminal rather than opening its own window; and the Software Manager is already running.
    pub fn launch_from_ui(self) -> bool {
        matches!(self, Self::Molchanica | Self::PlasCad | Self::Peregrine)
    }

    fn packaging(self) -> Packaging {
        match self {
            Self::Shell => Packaging::Binary,
            _ => Packaging::Zip,
        }
    }

    /// The folder the application is installed in. This must match its setup script:
    /// `%LOCALAPPDATA%\Programs\<Name>` on Windows, and `~/<name>` on Linux. Shell, which has no
    /// setup script, goes in `~/.local/bin` on Linux, which is normally on the `PATH`.
    pub fn install_dir(self) -> Option<PathBuf> {
        let parent = install_parent()?;

        match OS {
            Os::Windows => Some(parent.join(self.to_string())),
            Os::Linux => match self {
                Self::Shell => Some(parent.join(".local").join("bin")),
                _ => Some(parent.join(self.release_file_name())),
            },
            Os::Mac => None,
        }
    }

    /// Where the installed executable is, or would be.
    pub fn exe_path(self) -> Option<PathBuf> {
        Some(self.install_dir()?.join(self.exe_name()))
    }

    /// The version in a release's file name, if it's this application's release for `os`.
    fn asset_version(self, asset_name: &str, os: &str, tag: &str) -> Option<Version> {
        match self.packaging() {
            Packaging::Zip => {
                // E.g. `molchanica_0_4_2_win.zip`. This doesn't match variants such as
                // `molchanica_0_4_2_win_nocuda.zip`, which leave out optional libraries.
                let version = asset_name
                    .strip_prefix(self.release_file_name())?
                    .strip_prefix('_')?
                    .strip_suffix(".zip")?
                    .strip_suffix(os)?
                    .strip_suffix('_')?;

                Version::parse_underscored(version)
            }
            Packaging::Binary => {
                if asset_name == self.exe_name() {
                    Version::parse(tag)
                } else {
                    None
                }
            }
        }
    }
}

/// The folder the applications are installed in, each in a folder of its own:
/// `%LOCALAPPDATA%\Programs` on Windows, and the home folder on Linux. See
/// [`Application::install_dir`].
pub fn install_parent() -> Option<PathBuf> {
    match OS {
        Os::Windows => {
            let local = PathBuf::from(std::env::var_os("LOCALAPPDATA")?);
            Some(local.join("Programs"))
        }
        Os::Linux => Some(PathBuf::from(std::env::var_os("HOME")?)),
        Os::Mac => None,
    }
}

impl Display for Application {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Also the Windows install folder and Start menu entry name; see `install_dir`.
        let v = match self {
            Self::Molchanica => "Molchanica",
            Self::PlasCad => "PlasCAD",
            Self::Peregrine => "Peregrine",
            Self::Shell => "Shell",
            Self::SoftwareManager => "Software Manager",
        };

        write!(f, "{v}")
    }
}

/// A semantic version, e.g. 0.4.2.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl Version {
    pub const fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }

    /// Parse e.g. `0.4.2`, or `v0.4.2`, as a git tag might be written.
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        Self::parse_with(s.strip_prefix('v').unwrap_or(s), '.')
    }

    /// As used in release archive names, e.g. `0_4_2`.
    fn parse_underscored(s: &str) -> Option<Self> {
        Self::parse_with(s, '_')
    }

    fn parse_with(s: &str, separator: char) -> Option<Self> {
        let mut parts = s.split(separator).map(|part| part.parse().ok());

        let result = Self {
            major: parts.next()??,
            minor: parts.next()??,
            patch: parts.next()??,
        };

        parts.next().is_none().then_some(result)
    }

    /// As used in release archive names, e.g. `0_4_2`.
    fn underscored(self) -> String {
        format!("{}_{}_{}", self.major, self.minor, self.patch)
    }
}

impl Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// A published release, and the file in it for this OS.
#[derive(Clone, Debug)]
pub struct Release {
    pub app: Application,
    pub version: Version,
    /// The release's page on GitHub, e.g. for its notes.
    pub page_url: String,
    pub asset_name: String,
    pub download_url: String,
    /// In bytes.
    pub size: u64,
}

#[derive(Debug)]
pub enum Error {
    /// We don't publish releases for this OS.
    UnsupportedOs,
    /// The repository has no published release.
    NoRelease,
    /// The latest release has no file for this OS.
    NoAsset { tag: String },
    /// Reaching GitHub, or downloading from it, failed.
    Network(String),
    /// The release's setup script failed. Contains its output.
    Setup(String),
    /// The application's files can't be replaced or removed while it's running.
    Running(Application),
    Io(io::Error),
    /// Background work panicked.
    Internal(String),
}

impl Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedOs => write!(f, "Releases aren't published for {OS}."),
            Self::NoRelease => write!(f, "No release has been published yet."),
            Self::NoAsset { tag } => write!(f, "Release {tag} has no download for {OS}."),
            Self::Network(e) => write!(f, "Couldn't reach GitHub: {e}"),
            Self::Setup(output) => write!(f, "The setup script failed:\n{output}"),
            Self::Running(app) => write!(f, "{app} is running. Close it, then try again."),
            Self::Io(e) => write!(f, "{e}"),
            Self::Internal(e) => write!(f, "Unexpected error: {e}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<ureq::Error> for Error {
    fn from(e: ureq::Error) -> Self {
        match e {
            ureq::Error::StatusCode(404) => Self::NoRelease,
            // Unauthenticated requests to the API are limited to 60 an hour, per IP address.
            ureq::Error::StatusCode(403 | 429) => {
                Self::Network("its rate limit was reached; try again in an hour.".to_owned())
            }
            e => Self::Network(e.to_string()),
        }
    }
}

/// The parts of GitHub's release JSON we use.
#[derive(Deserialize)]
struct GithubRelease {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    assets: Vec<GithubAsset>,
}

#[derive(Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
    size: u64,
}

/// `timeout` applies to the whole request, including reading the body.
fn agent(timeout: Option<Duration>) -> ureq::Agent {
    ureq::Agent::config_builder()
        // GitHub's API rejects requests without one.
        .user_agent(concat!("software_manager/", env!("CARGO_PKG_VERSION")))
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_recv_response(Some(Duration::from_secs(30)))
        .timeout_global(timeout)
        .build()
        .new_agent()
}

/// Find the latest published release of an application, and its file for this OS. This makes a
/// network request, so run it off the UI thread, e.g. with [`Task`].
pub fn latest_release(app: Application) -> Result<Release, Error> {
    let os = OS.release_name().ok_or(Error::UnsupportedOs)?;

    // Drafts and pre-releases are excluded from "latest".
    let url = format!(
        "https://api.github.com/repos/{GITHUB_OWNER}/{}/releases/latest",
        app.repo_name()
    );

    let mut resp = agent(Some(API_TIMEOUT))
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .call()?;

    let release: GithubRelease = resp.body_mut().read_json()?;

    // The version comes from the file name where it has one, rather than the tag: they've
    // disagreed before (tag 0.7.7, containing plascad_0_7_8_win.zip), and the file name is what
    // the release scripts derive from Cargo.toml.
    for asset in release.assets {
        if let Some(version) = app.asset_version(&asset.name, os, &release.tag_name) {
            return Ok(Release {
                app,
                version,
                page_url: release.html_url,
                asset_name: asset.name,
                download_url: asset.browser_download_url,
                size: asset.size,
            });
        }
    }

    Err(Error::NoAsset {
        tag: release.tag_name,
    })
}

/// The latest release, if it's newer than `current_version`; e.g. `env!("CARGO_PKG_VERSION")`.
pub fn check_for_update(app: Application, current_version: &str) -> Result<Option<Release>, Error> {
    let current = Version::parse(current_version).ok_or_else(|| {
        Error::Internal(format!("{current_version} isn't a valid version"))
    })?;

    let latest = latest_release(app)?;
    Ok((latest.version > current).then_some(latest))
}
