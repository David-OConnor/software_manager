# Software manager

For installing, updating, viewing status of applications I use and build.

This has two components: A GUI application used to manage application installation and view statuses, and a library which these applications call to update themselves. (And this application uses to update them).

Works on Linux and Windows; chooses the appropriate setup scripts (`sh` vs `ps1`) based on OS.

## Where things are installed

Each application is installed in a folder of its own, under:

- **Windows**: `%LOCALAPPDATA%\Programs`, e.g. `C:\Users\<you>\AppData\Local\Programs\Molchanica`.
- **Linux**: your home folder, e.g. `~/molchanica`. The exception is Shell, a single executable,
  which goes in `~/.local/bin`.

Their data, e.g. preferences and downloaded files:

- **Molchanica** and **PlasCAD** keep theirs in their install folders. Uninstalling leaves it there.
- **Shell** and **Peregrine** share a state file in your home folder: `~/shell_state.ss`, or
  `%USERPROFILE%\shell_state.ss` on Windows.
- **Software Manager** records the versions it installed in `%LOCALAPPDATA%\software_manager` on
  Windows, and `~/.local/share/software_manager` on Linux.
