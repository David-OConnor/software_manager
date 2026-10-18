//! Finding which version of an application is installed, and whether it's running.
//!
//! On Windows, the version is read from the executable: `winresource`, which each application's
//! `build.rs` uses to embed its icon, also embeds the version from Cargo.toml. Linux executables
//! have no equivalent, and nor does Shell's on Windows; for those, we record the version when we
//! install a release, along with a fingerprint of the executable we installed. The record is only
//! trusted while the installed executable still matches it, so an install made some other way,
//! e.g. by running a setup script by hand, shows as an unknown version rather than a wrong one.

use std::{
    fs::{self, File},
    io::{self, Read},
    path::{Path, PathBuf},
};

use crate::{Application, OS, Os, Version};

/// An installed application.
#[derive(Clone, Debug)]
pub struct Installation {
    pub exe: PathBuf,
    /// `None` if we can't tell which version is installed. See the module docs.
    pub version: Option<Version>,
}

/// Find the installed copy of an application, and its version. `None` if it isn't installed.
///
/// This can read the whole executable, so run it off the UI thread.
pub fn installed(app: Application) -> Option<Installation> {
    let exe = app.exe_path()?;
    if !exe.is_file() {
        return None;
    }

    let version = embedded_version(&exe).or_else(|| recorded_version(app, &exe));

    Some(Installation { exe, version })
}

/// Whether the installed copy of an application is running. Its files can't be replaced while it
/// is: the setup scripts refuse to.
pub fn is_running(app: Application) -> bool {
    match app.exe_path() {
        Some(exe) if exe.is_file() => exe_running(&exe),
        _ => false,
    }
}

#[cfg(windows)]
fn exe_running(exe: &Path) -> bool {
    use std::fs::OpenOptions;

    // Windows doesn't allow a running executable to be opened for writing. Opening it, without
    // writing anything, is a test that needs no process list.
    const ERROR_SHARING_VIOLATION: i32 = 32;

    match OpenOptions::new().write(true).open(exe) {
        Ok(_) => false,
        Err(e) => e.raw_os_error() == Some(ERROR_SHARING_VIOLATION),
    }
}

#[cfg(target_os = "linux")]
fn exe_running(exe: &Path) -> bool {
    let Ok(exe) = fs::canonicalize(exe) else {
        return false;
    };
    let Ok(entries) = fs::read_dir("/proc") else {
        return false;
    };

    // Each process has a numbered entry, whose `exe` links to its executable. Other users'
    // processes' links can't be read, which is fine: we only install for this user.
    entries.flatten().any(|entry| {
        let is_process = entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.bytes().all(|b| b.is_ascii_digit()));

        is_process && fs::read_link(entry.path().join("exe")).is_ok_and(|target| target == exe)
    })
}

#[cfg(not(any(windows, target_os = "linux")))]
fn exe_running(_exe: &Path) -> bool {
    false
}

/// The version embedded in a Windows executable's resources.
#[cfg(windows)]
fn embedded_version(exe: &Path) -> Option<Version> {
    use std::{ffi::c_void, os::windows::ffi::OsStrExt, ptr};

    use windows_sys::Win32::Storage::FileSystem::{
        GetFileVersionInfoSizeW, GetFileVersionInfoW, VS_FIXEDFILEINFO, VerQueryValueW,
    };

    let path: Vec<u16> = exe.as_os_str().encode_wide().chain([0]).collect();
    // The fixed-size block at the root of the version resource.
    let root: Vec<u16> = "\\".encode_utf16().chain([0]).collect();

    // SAFETY: Both strings are null-terminated. `data` is the size the first call asks for, and
    // `VerQueryValueW` gives a pointer into it, checked to be large enough before it's read.
    unsafe {
        let size = GetFileVersionInfoSizeW(path.as_ptr(), ptr::null_mut());
        if size == 0 {
            return None; // E.g. Shell's executable, which has no resources.
        }

        let mut data = vec![0u8; size as usize];
        if GetFileVersionInfoW(path.as_ptr(), 0, size, data.as_mut_ptr().cast()) == 0 {
            return None;
        }

        let mut info: *mut c_void = ptr::null_mut();
        let mut info_len = 0;

        let found = VerQueryValueW(data.as_ptr().cast(), root.as_ptr(), &mut info, &mut info_len);
        if found == 0 || info.is_null() || (info_len as usize) < size_of::<VS_FIXEDFILEINFO>() {
            return None;
        }

        let info = ptr::read_unaligned(info as *const VS_FIXEDFILEINFO);

        let version = Version::new(
            info.dwFileVersionMS >> 16,
            info.dwFileVersionMS & 0xffff,
            info.dwFileVersionLS >> 16,
        );

        // A resource without a version set reads as 0.0.0.
        (version != Version::default()).then_some(version)
    }
}

#[cfg(not(windows))]
fn embedded_version(_exe: &Path) -> Option<Version> {
    None
}

/// Where we keep records of what we've installed, and nothing else.
fn data_dir() -> Option<PathBuf> {
    let base = match OS {
        Os::Windows => PathBuf::from(std::env::var_os("LOCALAPPDATA")?),
        _ => match std::env::var_os("XDG_DATA_HOME") {
            Some(dir) if !dir.is_empty() => PathBuf::from(dir),
            _ => PathBuf::from(std::env::var_os("HOME")?)
                .join(".local")
                .join("share"),
        },
    };

    Some(base.join("software_manager"))
}

fn record_path(app: Application) -> Option<PathBuf> {
    let file_name = format!("{}.txt", app.release_file_name());
    Some(data_dir()?.join("installed").join(file_name))
}

/// Record that `exe`, which the setup script copies into place unchanged, is `version`.
/// Best effort: if this fails, the version only shows as unknown.
pub(crate) fn record_version(app: Application, exe: &Path, version: Version) {
    let write = || -> io::Result<()> {
        let path = record_path(app).ok_or(io::ErrorKind::NotFound)?;
        let (size, hash) = fingerprint(exe)?;

        fs::create_dir_all(path.parent().unwrap())?;
        fs::write(path, format!("{version}\n{size}\n{hash:016x}\n"))
    };

    if let Err(e) = write() {
        eprintln!("Unable to record the installed version of {app}: {e}");
    }
}

pub(crate) fn forget_version(app: Application) {
    if let Some(path) = record_path(app) {
        let _ = fs::remove_file(path);
    }
}

/// The version we recorded installing, if the installed executable is still the one we installed.
fn recorded_version(app: Application, exe: &Path) -> Option<Version> {
    let record = fs::read_to_string(record_path(app)?).ok()?;
    let mut lines = record.lines();

    let version = Version::parse(lines.next()?)?;
    let size: u64 = lines.next()?.parse().ok()?;
    let hash = u64::from_str_radix(lines.next()?, 16).ok()?;

    // The size alone rules out most replacements, without reading the file.
    if fs::metadata(exe).ok()?.len() != size {
        return None;
    }

    (fingerprint(exe).ok()? == (size, hash)).then_some(version)
}

/// The file's size, and its FNV-1a hash. FNV is used since it's stable across builds and Rust
/// versions, unlike `DefaultHasher`: the record may be written by one program and read by another.
/// It isn't cryptographic; it only has to notice that the file was replaced.
fn fingerprint(path: &Path) -> io::Result<(u64, u64)> {
    let mut file = File::open(path)?;
    let mut buf = vec![0; 64 * 1024];

    let mut size = 0;
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;

    loop {
        let n = match file.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };

        for &byte in &buf[..n] {
            hash ^= byte as u64;
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
        size += n as u64;
    }

    Ok((size, hash))
}
