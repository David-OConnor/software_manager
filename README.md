# Software manager

For installing, updating, viewing status of applications I use and build.

This has two components: A GUI application used to manage application installation and view statuses, and a library which these applications call to update themselves. (And this application uses to update them).

Works on Linux and Windows; chooses the appropriate setup scripts (`sh` vs `ps1`) based on OS.


## The GUI

Lists each application, the installed version, and the latest release, with buttons to install, update, or uninstall as appropriate. Applications are installed for the current user, without administrator rights, by the setup script in their release. Uninstalling removes what the setup script installed, and its Start menu entry or desktop file, but keeps preferences and other data in the application's folder.

Release it with `make_release.ps1` (Windows; also builds the Linux release in WSL) or `make_release.sh`, as with the other applications.


## The library

Applications depend on it without the GUI:

```toml
software_manager = { path = "../software_manager", default-features = false }
```

`default-features = false` matters: the `gui` feature embeds this program's icon and version resource in the Windows executable, which would collide with the application's own.

In an About popup, keep an `Updater` in the UI state, and each frame:

```rust
updater.poll();

match updater.status() {
    UpdateStatus::Idle => { /* "Check for updates" button: updater.check(Application::Molchanica, VERSION, repaint) */ }
    UpdateStatus::Available(release) => { /* "Download" button: updater.download(repaint) */ }
    UpdateStatus::Ready(_) => { /* "Restart and install" button: if updater.install_on_exit() { close the app } */ }
    // ...
}
```

`repaint` is called from a background thread when work finishes; e.g. `move || ctx.request_repaint()`. Installing waits for the application to close, runs the release's setup script, then reopens it.


## Release conventions

Releases are read from GitHub, from `github.com/David-OConnor/<name>/releases/latest`. Each release has a zip per OS, as the `make_release` scripts produce: `<name>_<major>_<minor>_<patch>_<win|linux>.zip`, containing the executable and `setup_<name>.ps1` (Windows) or `setup_<name>.sh` (Linux). Shell's releases are the bare executable, versioned by the release tag.

The installed version is read from the executable on Windows, where `winresource` (in each application's `build.rs`) embeds it. Linux executables have no equivalent, so it's recorded when installing through this library; an application installed some other way shows its version as unknown.
