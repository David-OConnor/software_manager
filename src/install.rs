//! Downloading releases, and installing and uninstalling them.
//!
//! Zip releases are installed by their own setup script, as a user would after extracting one, so
//! the install is the same either way. A running application's files can't be replaced, so to
//! update the application doing the updating, [`stage_update`] downloads the release, and
//! [`StagedUpdate::install_on_exit`] leaves a process behind to run the setup script once the
//! application has closed, then reopen it.

use std::{
    env,
    fs::{self, File},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
};

use crate::{
    Application, Error, OS, Os, Packaging, Release, agent,
    detect::{forget_version, is_running, record_version},
};

/// Hides the console window a console program, e.g. PowerShell, would otherwise open when started
/// from a GUI.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Download progress, shared between the thread downloading and the UI showing it.
#[derive(Debug, Default)]
pub struct Progress {
    downloaded: AtomicU64,
    total: AtomicU64,
}

impl Progress {
    /// Bytes downloaded so far, and the total; 0 until it's known.
    pub fn bytes(&self) -> (u64, u64) {
        (
            self.downloaded.load(Ordering::Relaxed),
            self.total.load(Ordering::Relaxed),
        )
    }

    /// From 0 to 1.
    pub fn fraction(&self) -> f32 {
        match self.bytes() {
            (_, 0) => 0.,
            (downloaded, total) => (downloaded as f64 / total as f64).min(1.) as f32,
        }
    }

    /// Whether the download has finished, and installing has started.
    pub fn downloaded(&self) -> bool {
        let (downloaded, total) = self.bytes();
        total > 0 && downloaded >= total
    }
}

/// Where a release is downloaded and extracted to, before installing it.
fn staging_dir(release: &Release) -> PathBuf {
    let name = format!(
        "{}_{}",
        release.app.release_file_name(),
        release.version.underscored()
    );

    env::temp_dir().join("software_manager").join(name)
}

/// Download a release's file to `dest`.
fn download(release: &Release, dest: &Path, progress: &Progress) -> Result<(), Error> {
    let mut resp = agent(None).get(&release.download_url).call()?;

    let total = resp.body().content_length().unwrap_or(release.size);
    progress.total.store(total, Ordering::Relaxed);
    progress.downloaded.store(0, Ordering::Relaxed);

    // Under a temporary name, so an interrupted download is never mistaken for a complete one.
    let partial = dest.with_file_name(format!("{}.part", release.asset_name));
    let mut file = File::create(&partial)?;

    let mut reader = resp.body_mut().as_reader();
    let mut buf = vec![0; 64 * 1024];

    loop {
        let n = match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(Error::Network(e.to_string())),
        };

        file.write_all(&buf[..n])?;
        progress.downloaded.fetch_add(n as u64, Ordering::Relaxed);
    }

    file.sync_all()?;
    drop(file);

    fs::rename(&partial, dest)?;
    Ok(())
}

/// Download a zip release, and extract it into a fresh folder. Returns that folder.
fn download_and_extract(release: &Release, progress: &Progress) -> Result<PathBuf, Error> {
    let dir = staging_dir(release);

    // Start clean: files left from an earlier attempt would otherwise be installed with this one.
    if dir.exists() {
        fs::remove_dir_all(&dir)?;
    }
    fs::create_dir_all(&dir)?;

    let archive = dir.join(&release.asset_name);
    download(release, &archive, progress)?;

    let contents = dir.join("contents");

    let mut zip = zip::ZipArchive::new(File::open(&archive)?).map_err(io::Error::other)?;
    zip.extract(&contents).map_err(io::Error::other)?;

    let _ = fs::remove_file(&archive);

    Ok(contents)
}

/// The setup script in an extracted release.
fn setup_script(app: Application, contents: &Path) -> Result<PathBuf, Error> {
    let ext = match OS {
        Os::Windows => "ps1",
        Os::Linux => "sh",
        Os::Mac => return Err(Error::UnsupportedOs),
    };

    let script = contents.join(format!("setup_{}.{ext}", app.release_file_name()));

    if !script.is_file() {
        return Err(Error::Setup(format!(
            "The release doesn't contain {}.",
            script.file_name().unwrap().to_string_lossy()
        )));
    }

    Ok(script)
}

/// The command that runs a setup script.
fn setup_command(script: &Path) -> Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;

        // As the release's `setup_<name>.bat` runs it. Without `Bypass`, the execution policy
        // blocks scripts that were extracted from a downloaded zip.
        let mut cmd = Command::new("powershell.exe");
        cmd.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
            .arg(script)
            .creation_flags(CREATE_NO_WINDOW);
        cmd
    }

    #[cfg(not(windows))]
    {
        let mut cmd = Command::new("bash");
        cmd.arg(script);
        cmd
    }
}

/// Run an extracted release's setup script, and wait for it.
fn run_setup(app: Application, contents: &Path) -> Result<(), Error> {
    let script = setup_script(app, contents)?;

    // Some setup scripts find the files to copy relative to the working directory.
    let output = setup_command(&script)
        .current_dir(contents)
        .stdin(Stdio::null())
        .output()?;

    if !output.status.success() {
        let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
        text.push_str(&String::from_utf8_lossy(&output.stderr));

        return Err(Error::Setup(text.trim().to_owned()));
    }

    Ok(())
}

/// Install a release, or update to it: download it, then run its setup script, or for Shell, put
/// the executable in place.
///
/// This blocks until done, so run it off the UI thread, e.g. with [`crate::Task`]. To update the
/// running application, use [`stage_update`] instead.
pub fn install(release: &Release, progress: &Progress) -> Result<(), Error> {
    let app = release.app;

    match app.packaging() {
        Packaging::Zip => {
            if is_running(app) {
                return Err(Error::Running(app));
            }

            let contents = download_and_extract(release, progress)?;
            run_setup(app, &contents)?;

            record_version(app, &contents.join(app.exe_name()), release.version);

            let _ = fs::remove_dir_all(staging_dir(release));
        }
        Packaging::Binary => {
            let exe = app.exe_path().ok_or(Error::UnsupportedOs)?;
            fs::create_dir_all(exe.parent().unwrap())?;

            // Downloaded beside the target rather than to the temporary folder, so moving it
            // into place is a rename on the same drive.
            let new = exe.with_file_name(format!("{}.new", app.exe_name()));
            download(release, &new, progress)?;

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&new, fs::Permissions::from_mode(0o755))?;
            }

            replace_exe(&new, &exe)?;

            record_version(app, &exe, release.version);
        }
    }

    Ok(())
}

/// Move a new executable into place. On Windows, a running executable can't be overwritten, but it
/// can be renamed, so the old one is moved aside first, and removed if it isn't running. (On
/// Linux, the rename alone works: a running program keeps the old file open.) This lets Shell be
/// updated from inside itself.
fn replace_exe(new: &Path, exe: &Path) -> io::Result<()> {
    let old = exe.with_extension("old");

    if cfg!(windows) && exe.exists() {
        // From an earlier update. This fails, harmlessly, if that copy is still running.
        let _ = fs::remove_file(&old);
        fs::rename(exe, &old)?;
    }

    fs::rename(new, exe)?;
    let _ = fs::remove_file(&old);

    Ok(())
}

/// The files and folders an application's setup script installs, relative to its install folder.
/// Anything else in that folder is the application's own data, which uninstalling keeps.
fn installed_files(app: Application) -> Vec<String> {
    let mut files = vec![app.exe_name()];

    let extra: &[&str] = match (app, OS) {
        (Application::Molchanica, Os::Windows) => {
            &["convert_mpnn_weights.py", "cufft64_12.dll", "gemmi"]
        }
        (Application::Molchanica, _) => &["convert_mpnn_weights.py", "libcufft.so.12", "icon.png"],
        // Left over from an update; see `replace_exe`.
        (Application::Shell, _) => &["shell.old"],
        (_, Os::Windows) => &[],
        (_, _) => &["icon.png"],
    };

    files.extend(extra.iter().map(|f| f.to_string()));
    files
}

/// Start menu entries (Windows) or desktop files (Linux) the setup script creates.
fn launcher_entries(app: Application) -> Vec<PathBuf> {
    let mut result = Vec::new();

    if app.packaging() == Packaging::Binary {
        return result;
    }

    match OS {
        Os::Windows => {
            if let Some(appdata) = env::var_os("APPDATA") {
                let programs = PathBuf::from(appdata).join(r"Microsoft\Windows\Start Menu\Programs");
                result.push(programs.join(format!("{app}.lnk")));
            }
        }
        Os::Linux => {
            let desktop_file = format!("{}.desktop", app.release_file_name());

            // Some setup scripts respect `XDG_DATA_HOME`, and others don't.
            if let Some(dir) = env::var_os("XDG_DATA_HOME").filter(|d| !d.is_empty()) {
                result.push(PathBuf::from(dir).join("applications").join(&desktop_file));
            }
            if let Some(home) = env::var_os("HOME") {
                let dir = PathBuf::from(home).join(".local/share/applications");
                result.push(dir.join(&desktop_file));
            }
        }
        Os::Mac => (),
    }

    result
}

/// Remove an application: the files its setup script installed, and its Start menu entry or
/// desktop file. Anything else in its folder, e.g. preferences and downloaded data, is kept.
/// Returns the folder, if it's still there for that reason.
pub fn uninstall(app: Application) -> Result<Option<PathBuf>, Error> {
    if is_running(app) {
        return Err(Error::Running(app));
    }

    let dir = app.install_dir().ok_or(Error::UnsupportedOs)?;

    for file in installed_files(app) {
        let path = dir.join(file);

        if path.is_dir() {
            fs::remove_dir_all(&path)?;
        } else if path.exists() {
            fs::remove_file(&path)?;
        }
    }

    for entry in launcher_entries(app) {
        if entry.exists() {
            fs::remove_file(&entry)?;
        }
    }

    forget_version(app);

    // On Linux, Shell's folder is `~/.local/bin`, which isn't ours to remove.
    if app == Application::Shell && OS == Os::Linux {
        return Ok(None);
    }

    // Only succeeds if it's empty.
    let _ = fs::remove_dir(&dir);

    Ok(dir.exists().then_some(dir))
}

/// A release of the running application, downloaded and extracted, ready to install once the
/// application closes. See [`stage_update`].
#[derive(Debug)]
pub struct StagedUpdate {
    pub release: Release,
    contents: PathBuf,
}

/// Download and extract a release of the running application. Then call
/// [`StagedUpdate::install_on_exit`], and close the application.
///
/// This blocks until done, so run it off the UI thread, e.g. with [`crate::Task`].
pub fn stage_update(release: &Release, progress: &Progress) -> Result<StagedUpdate, Error> {
    if release.app.packaging() != Packaging::Zip {
        return Err(Error::Internal(format!(
            "{} is updated with `install`, not `stage_update`",
            release.app
        )));
    }

    let contents = download_and_extract(release, progress)?;

    // Fail now, rather than silently after the application has closed.
    setup_script(release.app, &contents)?;

    Ok(StagedUpdate {
        release: release.clone(),
        contents,
    })
}

impl StagedUpdate {
    /// Start a process that waits for this one to exit, runs the release's setup script, then
    /// reopens the application. Close the application right after this succeeds, e.g. with
    /// `ViewportCommand::Close`.
    ///
    /// There's no window to show the setup script's output in, so it's written to
    /// `install_log.txt`, in the folder the release was extracted to.
    pub fn install_on_exit(&self) -> Result<(), Error> {
        let app = self.release.app;

        let script = setup_script(app, &self.contents)?;
        let exe = app.exe_path().ok_or(Error::UnsupportedOs)?;
        let log = self.contents.join("install_log.txt");

        // Nothing of ours runs after the setup script to record this, so it's recorded now. If
        // the setup fails, the record is ignored, since the installed executable won't match it.
        record_version(app, &self.contents.join(app.exe_name()), self.release.version);

        spawn_installer(std::process::id(), &script, &log, &exe, &self.contents)?;
        Ok(())
    }
}

/// Quote a path for a PowerShell script.
#[cfg(windows)]
fn ps_quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "''"))
}

/// Start a hidden PowerShell process that waits for process `pid` to exit, runs the setup
/// `script`, then starts `exe`.
#[cfg(windows)]
fn spawn_installer(pid: u32, script: &Path, log: &Path, exe: &Path, dir: &Path) -> io::Result<()> {
    use std::os::windows::process::CommandExt;

    /// So that the process outlives this one if this one is in a job that ends with it, e.g. when
    /// started from some terminals or IDEs.
    const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;

    let exe_dir = exe.parent().unwrap_or(dir);

    // The setup scripts end with `throw` on failure; that's caught here so that the application
    // still reopens, as the previous version, and the error is in the log.
    let helper = format!(
        "Wait-Process -Id {pid} -ErrorAction SilentlyContinue\r\n\
         try {{ & {script} *> {log} }} catch {{ $_ | Out-File -Append -FilePath {log} }}\r\n\
         Start-Process -FilePath {exe} -WorkingDirectory {exe_dir}\r\n",
        script = ps_quote(script),
        log = ps_quote(log),
        exe = ps_quote(exe),
        exe_dir = ps_quote(exe_dir),
    );

    let helper_path = dir.join("install_on_exit.ps1");

    // With a byte order mark, so Windows PowerShell reads it as UTF-8, not the ANSI code page:
    // the paths can contain any character.
    fs::write(&helper_path, format!("\u{feff}{helper}"))?;

    let command = |flags| {
        Command::new("powershell.exe")
            .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
            .arg(&helper_path)
            .current_dir(dir)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(flags)
            .spawn()
    };

    // Breaking away fails if the job doesn't allow it; then the process is in the job, which is
    // usually fine.
    command(CREATE_NO_WINDOW | CREATE_BREAKAWAY_FROM_JOB).or_else(|_| command(CREATE_NO_WINDOW))?;

    Ok(())
}

/// Quote a path for a POSIX shell script.
#[cfg(unix)]
fn sh_quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', r"'\''"))
}

/// Start a background shell process that waits for process `pid` to exit, runs the setup `script`,
/// then starts `exe`.
#[cfg(unix)]
fn spawn_installer(pid: u32, script: &Path, log: &Path, exe: &Path, dir: &Path) -> io::Result<()> {
    use std::os::unix::process::CommandExt;

    let exe_dir = exe.parent().unwrap_or(dir);

    // The application reopens even if the setup fails, as the previous version.
    let helper = format!(
        "while kill -0 {pid} 2>/dev/null; do sleep 1; done\n\
         cd {dir} && bash {script} > {log} 2>&1\n\
         cd {exe_dir} && nohup {exe} > /dev/null 2>&1 &\n",
        dir = sh_quote(dir),
        script = sh_quote(script),
        log = sh_quote(log),
        exe_dir = sh_quote(exe_dir),
        exe = sh_quote(exe),
    );

    let helper_path = dir.join("install_on_exit.sh");
    fs::write(&helper_path, helper)?;

    Command::new("sh")
        .arg(&helper_path)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        // Its own process group, so a Ctrl+C in the terminal the application was started from
        // doesn't stop it too.
        .process_group(0)
        .spawn()?;

    Ok(())
}

#[cfg(not(any(windows, unix)))]
fn spawn_installer(_pid: u32, _script: &Path, _log: &Path, _exe: &Path, _dir: &Path) -> io::Result<()> {
    Err(io::ErrorKind::Unsupported.into())
}
