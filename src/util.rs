//! Small helpers shared by the whole app: byte formatting, path expansion,
//! elevation checks and "launch something and forget about it" plumbing.

use std::path::PathBuf;
use std::process::Command;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
#[cfg(windows)]
const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;

/// Held for the life of the process once this instance has claimed the name.
///
/// The handle has to outlive the call or Windows frees the mutex and the next
/// launch believes it is the first. A `OnceLock` holding the raw value is the
/// smallest thing that does that; it is never read back, only kept.
#[cfg(windows)]
static INSTANCE_MUTEX: std::sync::OnceLock<usize> = std::sync::OnceLock::new();

/// Claim the one-instance-per-session name. `false` means somebody else has it.
///
/// Two copies of this app in one session is not a feature. They poll the same
/// counters twice, they write `settings.ini` over each other's changes --
/// whichever closes last wins, silently -- and a cleaner or backup running in
/// each is two processes walking the same folders. The second launch should
/// put the window that already exists in front of the user instead, which is
/// what every well-behaved Windows tool does.
///
/// A kernel mutex rather than a lock file: it disappears when the process
/// does, however it dies, so there is no stale lock to recover from after a
/// crash.
///
/// `ERROR_ACCESS_DENIED` counts as "taken". The running copy will usually be
/// elevated, and an unelevated process cannot open a high-integrity object for
/// full access -- being refused the handle is itself proof that the name is
/// held.
pub fn claim_single_instance() -> bool {
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::{ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, GetLastError};
        use windows::Win32::System::Threading::CreateMutexW;
        use windows::core::w;

        // Local, not Global: two different users signed in at once are two
        // machines as far as this app is concerned, and each may have one.
        let handle = unsafe { CreateMutexW(None, true, w!("Local\\WindowsWrenchmasterInstance")) };
        let last_error = unsafe { GetLastError() };
        match handle {
            Ok(handle) => {
                if last_error == ERROR_ALREADY_EXISTS {
                    return false;
                }
                let _ = INSTANCE_MUTEX.set(handle.0 as usize);
                true
            }
            Err(_) => last_error != ERROR_ACCESS_DENIED,
        }
    }

    #[cfg(not(windows))]
    {
        true
    }
}

/// Bring an already-running window to the front, reporting whether it found
/// one, so the second launch looks like it opened the app rather than like it
/// did nothing at all.
///
/// Searched by window title, which is the one way to ask the question *before*
/// the elevation handoff: the running copy is normally elevated, and a second
/// launch must not raise a UAC prompt only to discover a window was open all
/// along and close again.
///
/// Raising is best effort. An unelevated process is not always allowed to put
/// an elevated window in front, and that failure is not worth a message -- the
/// window is there either way, and a dialog explaining why a click did nothing
/// is worse than the click doing nothing.
pub fn focus_existing_window() -> bool {
    #[cfg(windows)]
    unsafe {
        use windows::Win32::UI::WindowsAndMessaging::{
            FindWindowW, SW_RESTORE, SetForegroundWindow, ShowWindow,
        };
        use windows::core::w;

        if let Ok(window) = FindWindowW(None, w!("Windows Wrenchmaster")) {
            if !window.is_invalid() {
                let _ = ShowWindow(window, SW_RESTORE);
                let _ = SetForegroundWindow(window);
                return true;
            }
        }
        false
    }

    #[cfg(not(windows))]
    {
        false
    }
}

/// A count with its noun, pluralised properly.
///
/// `"1 device(s)"` appeared on nearly every check summary this app prints.
/// The parenthesised plural is the sort of thing that is invisible while
/// writing the format string and impossible to unsee in the finished window,
/// and there were twenty-nine of them.
///
/// Regular nouns only: everything this app counts takes a plain `s` (devices,
/// drives, programs, events, drivers, records). A word that does not belong
/// here should be written out at the call site rather than taught to this
/// function.
pub fn plural(count: usize, noun: &str) -> String {
    if count == 1 {
        format!("1 {noun}")
    } else {
        format!("{count} {noun}s")
    }
}

/// A span of time in the largest unit that still reads as a whole number:
/// minutes, then hours, then days. "0.2 days" is a figure nobody thinks in.
pub fn human_duration(hours: f64) -> String {
    let minutes = (hours * 60.0).round().max(0.0) as u64;
    if minutes < 60 {
        return plural(minutes as usize, "minute");
    }
    if minutes < 48 * 60 {
        let whole = minutes / 60;
        let rest = minutes % 60;
        return if rest == 0 {
            plural(whole as usize, "hour")
        } else {
            format!(
                "{} {}",
                plural(whole as usize, "hour"),
                plural(rest as usize, "minute")
            )
        };
    }
    let days = minutes / (24 * 60);
    let rest = (minutes % (24 * 60)) / 60;
    if rest == 0 {
        plural(days as usize, "day")
    } else {
        format!(
            "{} {}",
            plural(days as usize, "day"),
            plural(rest as usize, "hour")
        )
    }
}

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
    // A visible console, and not `-NonInteractive`. MAS is a menu the user
    // has to answer; run hidden and non-interactive, as it was, the script
    // reaches its first prompt, finds no console to read from and dies. The
    // app then reported that it had started it and nothing ever happened.
    cmd.args([
        "-NoProfile",
        "-ExecutionPolicy",
        "Bypass",
        "-Command",
        "irm 'https://get.activated.win' | iex".into(),
    ]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NEW_CONSOLE);
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
