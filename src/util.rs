//! Small helpers shared by the whole app: byte formatting, path expansion,
//! elevation checks and "launch something and forget about it" plumbing.

use std::path::PathBuf;
use std::process::Command;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
#[cfg(windows)]
const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;

/// Human readable byte count. The original tool only ever spoke in GB/MB/KB
/// of "RAM", this one is used for disks, caches and junk piles alike.
pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.2} {}", UNITS[unit])
    }
}

/// Expand `%VAR%` style placeholders. Returns `None` when a variable is not
/// set, so callers can quietly skip targets that do not exist on this machine.
pub fn expand(raw: &str) -> Option<PathBuf> {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let tail = &rest[start + 1..];
        let end = tail.find('%')?;
        let value = std::env::var(&tail[..end]).ok()?;
        out.push_str(&value);
        rest = &tail[end + 1..];
    }
    out.push_str(rest);
    Some(PathBuf::from(out))
}

/// The flag the elevated child is launched with. It marks a process as "this
/// one came from our own UAC handoff", so a failed elevation cannot loop
/// forever. It is *not* evidence of elevation: anybody can pass an argument.
pub const RESTART_FLAG: &str = "--wrenchmaster-uac-relaunch";

/// Whether this process actually runs with an elevated token. Asks Windows
/// about the process token rather than trusting argv or probing the registry:
/// an elevated process can still be denied on individual protected keys, and
/// `IsUserAnAdmin` only reports administrator-group membership.
pub fn is_elevated() -> bool {
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::{CloseHandle, HANDLE};
        use windows::Win32::Security::{
            GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation,
        };
        use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

        unsafe {
            let mut token = HANDLE::default();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
                return false;
            }
            let mut elevation = TOKEN_ELEVATION::default();
            let mut returned = 0u32;
            let ok = GetTokenInformation(
                token,
                TokenElevation,
                Some((&raw mut elevation).cast()),
                size_of::<TOKEN_ELEVATION>() as u32,
                &mut returned,
            )
            .is_ok();
            let _ = CloseHandle(token);
            ok && elevation.TokenIsElevated != 0
        }
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// True when this build was started by the development launcher. Used only for
/// cosmetics (showing tools that are otherwise hidden on a healthy machine) —
/// never to decide anything about privileges.
pub fn is_development_mode() -> bool {
    std::env::var("WRENCHMASTER_DEV").is_ok_and(|value| value == "1")
        || cfg!(debug_assertions) && std::env::var_os("CARGO").is_some()
}

/// Whether the startup UAC prompt is suppressed. The launchers set this after
/// they have already elevated the process themselves, and it is also the
/// escape hatch for working on the UI without a prompt on every `cargo run`.
pub fn uac_suppressed() -> bool {
    std::env::var("WRENCHMASTER_NO_UAC").is_ok_and(|value| value == "1")
}

/// Resolve the inbox Windows PowerShell executable without relying on PATH.
pub fn powershell_path() -> PathBuf {
    std::env::var_os("SystemRoot")
        .map(|root| PathBuf::from(root).join("System32\\WindowsPowerShell\\v1.0\\powershell.exe"))
        .unwrap_or_else(|| PathBuf::from("powershell.exe"))
}

/// Start a detached process without flashing a console window.
pub fn spawn(program: &str, args: &[&str]) -> std::io::Result<()> {
    let mut cmd = Command::new(program);
    cmd.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.spawn().map(|_| ())
}

/// Open a shell target: a control panel applet, an `ms-settings:` URI, a
/// folder, a `.diagcab`. Everything Explorer knows how to handle.
pub fn shell_open(target: &str) -> std::io::Result<()> {
    spawn("cmd", &["/c", "start", "", target])
}

/// Run something in a visible console and keep the window open, for the
/// long-running repair commands where the output *is* the point.
pub fn console(command: &str) -> std::io::Result<()> {
    Command::new("cmd")
        .args(["/c", "start", "", "cmd", "/k", command])
        .spawn()
        .map(|_| ())
}

/// Run an executable with argument-safe parameters in a new visible console.
/// This leaves stdout and stderr attached to that console so errors can be read.
pub fn console_exe(program: &str, args: &[&str]) -> std::io::Result<()> {
    let mut cmd = Command::new(program);
    cmd.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NEW_CONSOLE);
    }
    cmd.spawn().map(|_| ())
}

/// Request elevation once at startup. Returns `false` in the unelevated parent,
/// which should exit after launching its elevated replacement.
///
/// Elevation is attempted in debug builds too: a development run that silently
/// drops to a standard token leaves most of the app greyed out. Set
/// `WRENCHMASTER_NO_UAC=1` to opt out — either because the launcher already
/// elevated us, or because the UI work at hand does not need the token.
pub fn request_elevation_at_startup() -> std::io::Result<bool> {
    #[cfg(windows)]
    {
        if is_elevated() {
            return Ok(true);
        }
        // Already came back from a UAC handoff without a token: the user
        // declined, or policy denied it. Carry on unelevated instead of
        // prompting again in a loop.
        if std::env::args_os().any(|arg| arg == RESTART_FLAG) || uac_suppressed() {
            return Ok(true);
        }

        match relaunch_elevated() {
            Ok(()) => Ok(false),
            // A declined prompt is a choice, not a failure: keep running with
            // the standard token and let the UI mark what it cannot do.
            Err(_) => Ok(true),
        }
    }

    #[cfg(not(windows))]
    {
        Ok(true)
    }
}

/// Launch a fresh, elevated copy of this executable and leave it running. The
/// caller is responsible for shutting itself down afterwards.
pub fn relaunch_elevated() -> std::io::Result<()> {
    #[cfg(windows)]
    {
        let exe = std::env::current_exe()?;
        let escaped_exe = exe.to_string_lossy().replace('\'', "''");
        let working_dir = exe
            .parent()
            .map(|dir| dir.to_string_lossy().replace('\'', "''"))
            .unwrap_or_default();
        let working_dir_arg = if working_dir.is_empty() {
            String::new()
        } else {
            format!(" -WorkingDirectory '{working_dir}'")
        };
        // The child must not inherit the suppression flag, or an in-app
        // "restart as administrator" would hand the token question back to a
        // process that has been told never to ask.
        let script = format!(
            "$ErrorActionPreference = 'Stop'; \
             $env:WRENCHMASTER_NO_UAC = $null; \
             try {{ Start-Process -FilePath '{escaped_exe}'{working_dir_arg} \
             -ArgumentList '{RESTART_FLAG}' -Verb RunAs; exit 0 }} \
             catch {{ [Console]::Error.WriteLine($_.Exception.Message); exit 1 }}"
        );
        let mut cmd = Command::new(powershell_path());
        cmd.args([
            "-NoProfile",
            "-NonInteractive",
            "-WindowStyle",
            "Hidden",
            "-Command",
            &script,
        ]);
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let result = cmd.output()?;
        if !result.status.success() {
            let detail = String::from_utf8_lossy(&result.stderr).trim().to_owned();
            return Err(std::io::Error::other(if detail.is_empty() {
                "Windows did not start Wrenchmaster as administrator.".to_owned()
            } else {
                detail
            }));
        }
        Ok(())
    }

    #[cfg(not(windows))]
    {
        Err(std::io::Error::other(
            "Elevation is only meaningful on Windows.",
        ))
    }
}

/// Download and run the Microsoft Activation Scripts from get.activated.win.
///
/// This is not the Windows activation settings page: it fetches a script over
/// the network and executes it with this process's rights. The name says so
/// because the button that calls it has to say so too.
pub fn run_activation_scripts() -> std::io::Result<()> {
    let mut cmd = Command::new(powershell_path());
    cmd.args([
        "-NoProfile",
        "-NonInteractive",
        "-WindowStyle",
        "Hidden",
        "-Command",
        "irm 'https://get.activated.win' | iex".into(),
    ]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.spawn().map(|_| ())
}
/// Run a hidden PowerShell one-liner and wait for it. Used for the few jobs
/// that have no sane Win32-free equivalent (emptying the Recycle Bin).
pub fn powershell_wait(script: &str) -> std::io::Result<std::process::ExitStatus> {
    let mut cmd = Command::new("powershell");
    cmd.args(["-NoProfile", "-NonInteractive", "-Command", script]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.status()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_bytes() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(1024), "1.00 KB");
        assert_eq!(human_bytes(1024 * 1024 * 3 / 2), "1.50 MB");
    }

    #[test]
    fn expands_known_variables() {
        unsafe { std::env::set_var("WM_TEST_ROOT", r"C:\wm") };
        assert_eq!(
            expand(r"%WM_TEST_ROOT%\logs"),
            Some(std::path::PathBuf::from(r"C:\wm\logs"))
        );
        assert_eq!(
            expand(r"plain\path"),
            Some(std::path::PathBuf::from(r"plain\path"))
        );
        assert_eq!(expand(r"%WM_TEST_MISSING%\x"), None);
    }
}
