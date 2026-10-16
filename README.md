# Wrenchmaster Legacy

A cleaned, open-source source copy of the 2017 Visual Basic Windows Forms utility. The original interface and useful legacy resources are retained, with several unfinished controls replaced by small, understandable Windows maintenance helpers.

## Features

- Quick access to Task Manager, Startup Apps, Storage, and Power settings.
- Windows settings shortcuts for display, network, sound, and notifications.
- Troubleshooting shortcuts for Event Viewer, Reliability Monitor, System Information, and Windows Update.
- A local, read-only system summary.
- A preview-first temporary-file cleaner for `.tmp` and `.log` files older than 24 hours in the current Windows user's Temp folder. It skips inaccessible items and linked folders, and asks before deleting.
- A manual Windows System Restore point action.
- A simple accent color setting.
- The Windows-logo startup screen and the original credits.

The utility does not upload system information. Most features open built-in Windows tools. The temporary-file cleaner only operates inside the current user's Temp folder; it does not clear browser data, registry entries, or Windows directories.

## Build

Open `Windows Wrenchmaster.sln` in Visual Studio with the .NET Framework 4.8 Developer Pack installed, then build the `Release` configuration. Generated binaries and intermediate files are intentionally excluded from source control.

The application requests administrator approval at startup, which Windows may require for the System Restore point feature.

## Credits

See [CREDITS.md](CREDITS.md) for the original project and component credits.
