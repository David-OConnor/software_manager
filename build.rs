// Embeds the application icon in the Windows `.exe`, so Explorer, shortcuts, Start and the
// taskbar show it. This also embeds the version from Cargo.toml, which is how we read the
// installed version of an application on Windows. (See `detect.rs`.)
//
// Only for the GUI: the resource is linked into whatever binary depends on this crate, so for an
// application using the library alone, it would collide with the application's own.

use std::io;

fn main() -> io::Result<()> {
    #[cfg(feature = "gui")]
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        winresource::WindowsResource::new()
            // This path can be absolute, or relative to your crate root.
            .set_icon("resources/icon.ico")
            .compile()?;
    }
    Ok(())
}
