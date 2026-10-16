//! The Troubleshoot / Repair / Tools tabs.
//!
//! The old build shipped a folder of `.diagcab` files and a `Process.Start`
//! per button. The diagcabs are gone (Microsoft retired MSDT), so this uses
//! the in-box Settings troubleshooters and the command-line repair tools,
//! and still picks up local `.diagcab` files if you keep any next to the exe.

use std::path::PathBuf;

use crate::util::{console, console_exe, powershell_path, shell_open, spawn};

#[derive(Clone)]
pub enum Run {
    /// Hand to the shell: `ms-settings:` URIs, `.msc` consoles, applets.
    Shell(String),
    /// Run with a visible console window kept open afterwards.
    Console(String),
    /// Run an executable in its own visible console with separate arguments.
    ConsoleExe(String, Vec<String>),
    /// Launch an executable directly, no console flash.
    Exe(String, Vec<String>),
}

#[derive(Clone)]
pub struct Action {
    pub name: String,
    pub what: String,
    pub needs_admin: bool,
    /// Whether this action changes the machine in a way that is awkward or
    /// slow to undo, and so has to be confirmed before it runs.
    ///
    /// Needing administrator rights is not the same question. Plenty of
    /// read-only consoles need elevation, and resetting the network stack
    /// needs it too but can also drop a remote or VPN session with no way
    /// back -- that is what this flag is for.
    pub destructive: bool,
    pub run: Run,
}

impl Action {
    pub fn launch(&self) -> std::io::Result<()> {
        match &self.run {
            Run::Shell(target) => shell_open(target),
            Run::Console(cmd) => console(cmd),
            Run::ConsoleExe(exe, args) => {
                let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
                console_exe(exe, &refs)
            }
            Run::Exe(exe, args) => {
                let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
                spawn(exe, &refs)
            }
        }
    }
}

fn action(name: &str, what: &str, needs_admin: bool, run: Run) -> Action {
    Action {
        name: name.to_string(),
        what: what.to_string(),
        needs_admin,
        destructive: false,
        run,
    }
}

/// An elevated console command that has to be confirmed first.
fn admin_console_destructive(name: &str, what: &str, cmd: &str) -> Action {
    Action {
        destructive: true,
        ..admin_console(name, what, cmd)
    }
}

fn shell(name: &str, what: &str, target: &str) -> Action {
    action(name, what, false, Run::Shell(target.to_string()))
}

fn admin_console(name: &str, what: &str, cmd: &str) -> Action {
    action(name, what, true, Run::Console(cmd.to_string()))
}

/// Settings-based troubleshooters, which is where Windows 10/11 moved the
/// wizards the 2017 build bundled as diagcabs.
pub fn troubleshooters() -> Vec<Action> {
    vec![
        shell(
            "All troubleshooters",
            "The full Settings list: audio, printer, Bluetooth, camera and the rest.",
            "ms-settings:troubleshoot",
        ),
        shell(
            "Windows Update",
            "Resets the update pipeline when a patch keeps failing.",
            "ms-settings:troubleshoot",
        ),
        shell(
            "Network and internet",
            "The network diagnostic that replaced network10.diagcab.",
            "ms-settings:network-status",
        ),
        shell(
            "Audio playback",
            "Sound troubleshooting and per-device output settings.",
            "ms-settings:sound",
        ),
        shell(
            "Printers",
            "Printer queue, drivers and the printing troubleshooter.",
            "ms-settings:printers",
        ),
        shell(
            "Bluetooth and devices",
            "Pairing problems and device removal.",
            "ms-settings:bluetooth",
        ),
        shell(
            "Power and sleep",
            "Sleep, hibernate and the power plan that broke your battery life.",
            "ms-settings:powersleep",
        ),
        shell(
            "Recovery options",
            "Reset this PC, advanced startup, go back to a previous build.",
            "ms-settings:recovery",
        ),
        action(
            "Program compatibility",
            "The classic compatibility wizard, still shipped in box.",
            false,
            Run::Exe(
                "msdt.exe".into(),
                vec!["-id".into(), "PCWDiagnostic".into()],
            ),
        ),
    ]
}

/// Defender scans plus links to the in-box Windows Security surfaces.
pub fn security_actions() -> Vec<Action> {
    let powershell = powershell_path().to_string_lossy().into_owned();
    vec![
        action(
            "Microsoft Defender quick scan",
            "Checks common locations for known threats using Defender's current security intelligence. Requires administrator rights.",
            true,
            Run::ConsoleExe(
                powershell.clone(),
                vec![
                    "-NoProfile".into(),
                    "-NoExit".into(),
                    "-Command".into(),
                    "Start-MpScan -ScanType QuickScan".into(),
                ],
            ),
        ),
        action(
            "Microsoft Defender full scan",
            "Scans files and running programs across the computer. It can take a long time. Requires administrator rights.",
            true,
            Run::ConsoleExe(
                powershell,
                vec![
                    "-NoProfile".into(),
                    "-NoExit".into(),
                    "-Command".into(),
                    "Start-MpScan -ScanType FullScan".into(),
                ],
            ),
        ),
        shell(
            "Open Windows Security",
            "Run an Offline scan and review Protection history or scan status.",
            "windowsdefender:",
        ),
        shell(
            "Virus & threat protection settings",
            "Review Microsoft Defender settings and the active security provider.",
            "ms-settings:windowsdefender",
        ),
    ]
}

/// Repairs that actually change something, so each one says what it does.
pub fn repairs() -> Vec<Action> {
    vec![
        admin_console(
            "System file check (SFC)",
            "Verifies and repairs protected system files. Takes a few minutes.",
            "sfc /scannow",
        ),
        admin_console(
            "Repair component store (DISM)",
            "Fixes the servicing store SFC relies on. Needs no media on modern builds.",
            "dism /Online /Cleanup-Image /RestoreHealth",
        ),
        admin_console(
            "Scan the system drive",
            "Read-only chkdsk scan. Does not lock the volume or require a reboot.",
            "chkdsk %SystemDrive% /scan",
        ),
        admin_console_destructive(
            "Reset Windows Update components",
            "Stops the update services, renames the cache folders, starts them again.",
            "net stop wuauserv & net stop bits & net stop cryptsvc & ren %SystemRoot%\\SoftwareDistribution SoftwareDistribution.old & ren %SystemRoot%\\System32\\catroot2 catroot2.old & net start cryptsvc & net start bits & net start wuauserv & echo Done. Reboot when convenient.",
        ),
        admin_console_destructive(
            "Reset the network stack",
            "Winsock catalog, TCP/IP, DNS cache and adapter leases. Reboot afterwards.",
            "ipconfig /flushdns & ipconfig /registerdns & netsh winsock reset & netsh int ip reset & echo Done. Reboot to finish.",
        ),
        action(
            "Flush the DNS cache",
            "The two-second fix for a site that resolves to the wrong place.",
            false,
            Run::Console("ipconfig /flushdns".into()),
        ),
        admin_console_destructive(
            "Rebuild the icon cache",
            "Clears the icon databases and restarts Explorer.",
            "taskkill /f /im explorer.exe & del /a /q \"%LOCALAPPDATA%\\IconCache.db\" & del /a /f /q \"%LOCALAPPDATA%\\Microsoft\\Windows\\Explorer\\iconcache*\" & start explorer.exe",
        ),
        action(
            "Disk Cleanup (cleanmgr)",
            "The in-box cleaner, for the component-store cleanup this app leaves alone.",
            false,
            Run::Exe("cleanmgr.exe".into(), vec![]),
        ),
        action(
            "Memory diagnostic",
            "Schedules a RAM test on the next boot.",
            false,
            Run::Exe("mdsched.exe".into(), vec![]),
        ),
        action(
            "Battery report",
            "Writes a full battery wear report to your desktop.",
            false,
            Run::Console(
                "powercfg /batteryreport /output \"%USERPROFILE%\\Desktop\\battery-report.html\""
                    .into(),
            ),
        ),
        admin_console(
            "Energy report",
            "60 second trace of what is keeping this machine awake.",
            "powercfg /energy /output \"%USERPROFILE%\\Desktop\\energy-report.html\"",
        ),
        admin_console(
            "Driver inventory",
            "Lists every third-party driver with its date and publisher.",
            "pnputil /enum-drivers",
        ),
    ]
}

/// Read-only places to look. These are the buttons that cannot hurt.
pub fn utilities() -> Vec<Action> {
    vec![
        action(
            "System information",
            "msinfo32, the full hardware and firmware dump.",
            false,
            Run::Exe("msinfo32.exe".into(), vec![]),
        ),
        action(
            "DirectX diagnostics",
            "dxdiag: display, sound and input stack.",
            false,
            Run::Exe("dxdiag.exe".into(), vec![]),
        ),
        action(
            "Resource monitor",
            "Live disk, network and memory per process.",
            false,
            Run::Exe("resmon.exe".into(), vec![]),
        ),
        action(
            "Performance monitor",
            "Counters and data collector sets.",
            false,
            Run::Exe("perfmon.exe".into(), vec![]),
        ),
        action(
            "Reliability monitor",
            "The timeline of crashes and failed updates.",
            false,
            Run::Exe("perfmon.exe".into(), vec!["/rel".into()]),
        ),
        shell(
            "Event viewer",
            "System and application logs.",
            "eventvwr.msc",
        ),
        shell(
            "Services",
            "Start, stop and set startup type.",
            "services.msc",
        ),
        shell(
            "Device manager",
            "Drivers and problem devices.",
            "devmgmt.msc",
        ),
        shell(
            "Disk management",
            "Partitions, volumes and drive letters.",
            "diskmgmt.msc",
        ),
        shell(
            "Task scheduler",
            "Everything that runs on a trigger.",
            "taskschd.msc",
        ),
        shell(
            "Registry editor",
            "Back up first. The Backup tab is one click away.",
            "regedit.exe",
        ),
        shell("Installed apps", "The uninstall list.", "appwiz.cpl"),
        shell(
            "Startup apps",
            "What launches when you sign in.",
            "ms-settings:startupapps",
        ),
        shell(
            "Storage sense",
            "Where the disk went, and the automatic cleanup settings.",
            "ms-settings:storagesense",
        ),
        shell(
            "Windows security",
            "Defender, firewall and device security.",
            "windowsdefender:",
        ),
        action(
            "Microsoft Activation Scripts",
            "Downloads get.activated.win and runs it in PowerShell. Third-party script, fetched over the network each time.",
            false,
            Run::Exe(
                powershell_path().to_string_lossy().into_owned(),
                vec![
                    "-NoProfile".into(),
                    "-NoExit".into(),
                    "-Command".into(),
                    "irm 'https://get.activated.win' | iex".into(),
                ],
            ),
        ),
        shell(
            "About Windows",
            "winver, for when you need the exact build.",
            "winver.exe",
        ),
    ]
}

/// Local `.diagcab` packages, if the user keeps any next to the
/// binary the way the original release shipped one.
pub fn local_diagcabs() -> Vec<PathBuf> {
    let mut found = Vec::new();
    let Ok(exe) = std::env::current_exe() else {
        return found;
    };
    let Some(dir) = exe.parent() else {
        return found;
    };
    for candidate in [dir.join("Fixit"), dir.to_path_buf()] {
        if let Ok(entries) = std::fs::read_dir(&candidate) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path
                    .extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| e.eq_ignore_ascii_case("diagcab"))
                {
                    found.push(path);
                }
            }
        }
    }
    found.sort();
    found
}

/// Registry export plus a restore point: the "Backup" button from the old
/// build, which used to shell out to a `wscript.vbs` that was never shipped.
pub fn export_registry(dest: &std::path::Path) -> Result<Vec<PathBuf>, String> {
    let stamp = timestamp();
    let folder = dest.join(format!("WrenchmasterRegistryBackup-{stamp}"));
    std::fs::create_dir_all(&folder).map_err(|e| format!("Cannot create {folder:?}: {e}"))?;

    let hives = [
        ("HKLM", "HKEY_LOCAL_MACHINE"),
        ("HKCU", "HKEY_CURRENT_USER"),
        ("HKCR", "HKEY_CLASSES_ROOT"),
        ("HKU", "HKEY_USERS"),
        ("HKCC", "HKEY_CURRENT_CONFIG"),
    ];

    let mut written = Vec::new();
    let mut failures = Vec::new();
    for (short, full) in hives {
        let file = folder.join(format!("{short}.reg"));
        let mut command = std::process::Command::new("reg");
        command.args(["export", full, &file.to_string_lossy(), "/y"]);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let status = command.status();
        match status {
            Ok(s) if s.success() => written.push(file),
            _ => failures.push(short),
        }
    }

    if written.is_empty() {
        return Err(format!(
            "No hive could be exported ({}). Try running as administrator.",
            failures.join(", ")
        ));
    }
    Ok(written)
}

/// A System Restore point, which is the backup that actually gets you out of
/// a bad tweak. Requires System Protection to be enabled on the system drive.
///
/// Windows refuses to create a second restore point within 24 hours of the
/// last one, and returns success while doing nothing about it. The `restore.vbs`
/// that shipped in the 2017 resources folder worked around this by zeroing
/// `SystemRestorePointCreationFrequency` first and deleting the value after;
/// that trick is ported here, including putting the old value back.
const RESTORE_POINT_SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
$key = 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\SystemRestore'
$name = 'SystemRestorePointCreationFrequency'
if (-not (Test-Path $key)) { New-Item -Path $key -Force | Out-Null }
$previous = (Get-ItemProperty -Path $key -Name $name -ErrorAction SilentlyContinue).$name
try {
    Set-ItemProperty -Path $key -Name $name -Value 0 -Type DWord
    Checkpoint-Computer -Description 'Wrenchmaster' -RestorePointType MODIFY_SETTINGS
} catch {
    Write-Output $_.Exception.Message
    exit 1
} finally {
    if ($null -eq $previous) {
        Remove-ItemProperty -Path $key -Name $name -ErrorAction SilentlyContinue
    } else {
        Set-ItemProperty -Path $key -Name $name -Value $previous -Type DWord
    }
}
exit 0
"#;

pub fn create_restore_point() -> Result<(), String> {
    let status = crate::util::powershell_wait(RESTORE_POINT_SCRIPT)
        .map_err(|e| format!("Could not start PowerShell: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err("Restore point failed. Enable System Protection on the system drive, and run this as administrator.".into())
    }
}

pub fn timestamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Plain civil-date maths: no chrono, no time zone database, still offline.
    let days = secs / 86_400;
    let tod = secs % 86_400;
    let (y, m, d) = civil_from_days(days as i64);
    format!(
        "{y:04}{m:02}{d:02}-{:02}{:02}{:02}",
        tod / 3600,
        (tod % 3600) / 60,
        tod % 60
    )
}

/// Howard Hinnant's days-from-civil, inverted. Exact, branch-light, no deps.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::civil_from_days;

    #[test]
    fn converts_epoch_days_to_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_000), (2022, 1, 8));
        // 2024 was a leap year: day 60 of that year is 29 February.
        assert_eq!(civil_from_days(19_782), (2024, 2, 29));
    }
}
