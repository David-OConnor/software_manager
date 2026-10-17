# Software manager

For installing, updating, viewing status of applications I use and build.

This has two components: A GUI application used to manage application installation and view statuses, and a library which these applications call to update themselves. (And this application uses to update them).

Works on Linux and Windows; chooses the appropriate setup scripts (`sh` vs `ps1`) based on OS.