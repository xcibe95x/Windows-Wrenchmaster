//! Read-only machine checks for the Diagnose page.
//!
//! Every check here only *reads*. Nothing repairs, schedules, deletes or
//! reboots: the page used to be called a benchmark and did nothing but
//! re-print numbers the Overview had already polled, and the fix for that is
//! more things looked at, not the first thing touched. That rules out
//! `sfc /scannow` and `DISM /RestoreHealth`, both of which rewrite the
//! component store and can run for a quarter of an hour.
//!
//! Each check runs on the same worker thread, one after another, behind its
//! own timeout, and reports its own outcome. A check that fails, times out or
//! needs an administrator token says so in its own row; it never takes the
//! rest of the scan down with it.

use std::process::Command;
use std::time::Duration;

use crate::util::{human_duration, plural};

#[derive(Clone, Debug)]
pub struct EventFinding {
    pub source: String,
    pub event_id: u32,
    /// Local time formatted by Windows as `yyyy-MM-dd HH:mm:ss`.
    pub time: String,
    pub message: String,
}

#[derive(Clone, Debug)]
pub enum ScanStatus {
    Complete,
    Partial,
    #[allow(dead_code)] // Constructed by the non-Windows implementation.
    Unsupported,
    Error(String),
}

#[derive(Clone, Debug)]
pub struct ScanResult {
    pub status: ScanStatus,
    pub findings: Vec<EventFinding>,
    /// Log-specific failures, retained when the other log could be queried.
    pub log_errors: Vec<String>,
}

impl ScanResult {
    fn failed(status: ScanStatus) -> Self {
        Self {
            status,
            findings: Vec::new(),
            log_errors: Vec::new(),
        }
    }
}

/// Query recent Critical and Error entries from System and Application.
pub fn scan_recent_events() -> ScanResult {
    #[cfg(not(windows))]
    {
        return ScanResult::failed(ScanStatus::Unsupported);
    }

    #[cfg(windows)]
    {
        const SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$since = (Get-Date).AddHours(-24)
foreach ($log in @('System', 'Application')) {
  try {
    $logInfo = Get-WinEvent -ListLog $log -ErrorAction Stop
    if (-not $logInfo.IsEnabled) { throw 'This event log is disabled.' }
    $events = @(Get-WinEvent -FilterHashtable @{ LogName = $log; Level = @(1, 2); StartTime = $since } -MaxEvents 100 -ErrorAction SilentlyContinue)
    foreach ($event in $events) {
      $message = [string]$event.Message
      if ($message.Length -gt 1800) { $message = $message.Substring(0, 1800) + '…' }
      $source = $log + ' / ' + [string]$event.ProviderName + ' / ' + [string]$event.LevelDisplayName
      $values = @($source, [string]$event.Id, $event.TimeCreated.ToString('yyyy-MM-dd HH:mm:ss'), $message)
      $encoded = foreach ($value in $values) { [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($value)) }
      [Console]::Out.WriteLine('EVENT' + "`t" + ($encoded -join "`t"))
    }
    [Console]::Out.WriteLine('LOGOK' + "`t" + $log)
  } catch {
    $detail = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($_.Exception.Message))
    [Console]::Out.WriteLine('LOGERR' + "`t" + $log + "`t" + $detail)
  }
}
"#;
        // The spawn/drain/deadline dance moved into `run_bounded` once the
        // other checks needed exactly the same thing; this call is what is
        // left of the copy that used to live here.
        let output = match run_powershell(SCRIPT, Duration::from_secs(45)) {
            Ok(output) => output,
            Err(error) => return ScanResult::failed(ScanStatus::Error(error)),
        };
        if !output.ok {
            return ScanResult::failed(ScanStatus::Error(output.failure_detail()));
        }

        let mut findings = Vec::new();
        let mut log_errors = Vec::new();
        let mut successful_logs = 0;
        for line in output.stdout.lines() {
            let mut fields = line.split('\t');
            match fields.next() {
                Some("EVENT") => {
                    let values: Vec<_> = fields.map(decode_base64_utf8).collect();
                    if values.len() == 4 {
                        if let Ok(event_id) = values[1].parse() {
                            findings.push(EventFinding {
                                source: values[0].clone(),
                                event_id,
                                time: values[2].clone(),
                                message: values[3].clone(),
                            });
                        }
                    }
                }
                Some("LOGOK") => successful_logs += 1,
                Some("LOGERR") => {
                    let log = fields.next().unwrap_or("Unknown log");
                    let message = fields
                        .next()
                        .map(decode_base64_utf8)
                        .unwrap_or_else(|| "Unknown query error".into());
                    log_errors.push(format!("{log}: {message}"));
                }
                _ => {}
            }
        }
        findings.sort_by(|a, b| b.time.cmp(&a.time));
        let status = if successful_logs == 2 {
            ScanStatus::Complete
        } else if successful_logs > 0 {
            ScanStatus::Partial
        } else {
            let detail = log_errors.join("; ");
            return ScanResult::failed(ScanStatus::Error(if detail.is_empty() {
                "Neither Windows event log could be queried.".into()
            } else {
                detail
            }));
        };
        ScanResult {
            status,
            findings,
            log_errors,
        }
    }
}

#[cfg(windows)]
fn read_pipe<R: std::io::Read>(pipe: Option<R>) -> Vec<u8> {
    let mut bytes = Vec::new();
    if let Some(mut pipe) = pipe {
        let _ = pipe.read_to_end(&mut bytes);
    }
    bytes
}

/// Decode one base64 field produced by the PowerShell checks.
///
/// Hand-rolled rather than pulled in as a dependency: this is the only base64
/// in the program and it only ever decodes output we ourselves encoded, so the
/// decoder is allowed to be forgiving. Anything unrecognised contributes zero
/// bits instead of panicking, and the result goes through `from_utf8_lossy`.
fn decode_base64_utf8(value: &str) -> String {
    let decoded = value
        .as_bytes()
        .chunks(4)
        .filter(|chunk| chunk.len() == 4)
        .flat_map(|chunk| {
            let digit = |byte: u8| -> Option<u8> {
                match byte {
                    b'A'..=b'Z' => Some(byte - b'A'),
                    b'a'..=b'z' => Some(byte - b'a' + 26),
                    b'0'..=b'9' => Some(byte - b'0' + 52),
                    b'+' => Some(62),
                    b'/' => Some(63),
                    _ => None,
                }
            };
            let a = digit(chunk[0]).unwrap_or(0) as u32;
            let b = digit(chunk[1]).unwrap_or(0) as u32;
            let c = if chunk[2] == b'=' {
                0
            } else {
                digit(chunk[2]).unwrap_or(0) as u32
            };
            let d = if chunk[3] == b'=' {
                0
            } else {
                digit(chunk[3]).unwrap_or(0) as u32
            };
            let packed = (a << 18) | (b << 12) | (c << 6) | d;
            let mut out = vec![(packed >> 16) as u8];
            if chunk[2] != b'=' {
                out.push((packed >> 8) as u8);
            }
            if chunk[3] != b'=' {
                out.push(packed as u8);
            }
            out
        })
        .collect::<Vec<_>>();
    String::from_utf8_lossy(&decoded).into_owned()
}

/// One of the independent machine checks the Diagnose page runs.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CheckId {
    Uptime,
    PendingReboot,
    PowerPlan,
    MemoryPressure,
    StartupLoad,
    StorageSpace,
    SearchIndexScope,
    PageFile,
    WindowsUpdate,
    EventLog,
    CrashDumps,
    DeviceProblems,
    DriverAge,
    SecurityPosture,
    MemoryDiagnostic,
    DiskHealth,
    VolumeDirtyBit,
    SystemFiles,
}

impl CheckId {
    /// Every check, in the order the worker runs them.
    ///
    /// Cheap and always-available checks come first on purpose. DISM is last
    /// because it is the one that can plausibly take a minute, and a checklist
    /// that sits empty for a minute before anything appears reads as a hang.
    pub const ALL: [CheckId; 18] = [
        CheckId::Uptime,
        CheckId::PendingReboot,
        CheckId::PowerPlan,
        CheckId::MemoryPressure,
        CheckId::StartupLoad,
        CheckId::StorageSpace,
        CheckId::SearchIndexScope,
        CheckId::PageFile,
        CheckId::WindowsUpdate,
        CheckId::EventLog,
        CheckId::CrashDumps,
        CheckId::DeviceProblems,
        CheckId::DriverAge,
        CheckId::SecurityPosture,
        CheckId::MemoryDiagnostic,
        CheckId::DiskHealth,
        CheckId::VolumeDirtyBit,
        CheckId::SystemFiles,
    ];

    /// The row title in the checklist.
    pub fn label(self) -> &'static str {
        match self {
            CheckId::Uptime => "Uptime",
            CheckId::PendingReboot => "Pending restart",
            CheckId::PowerPlan => "Power plan",
            CheckId::MemoryPressure => "Memory pressure",
            CheckId::DriverAge => "Driver age",
            CheckId::StartupLoad => "Startup programs",
            CheckId::StorageSpace => "Free disk space",
            CheckId::SearchIndexScope => "Search index scope",
            CheckId::PageFile => "Virtual memory",
            CheckId::WindowsUpdate => "Windows update age",
            CheckId::CrashDumps => "Recent crashes",
            CheckId::EventLog => "Windows event log",
            CheckId::DeviceProblems => "Device problems",
            CheckId::SecurityPosture => "Virus protection",
            CheckId::MemoryDiagnostic => "Memory diagnostic history",
            CheckId::DiskHealth => "Drive hardware health",
            CheckId::VolumeDirtyBit => "Filesystem dirty flag",
            CheckId::SystemFiles => "Windows component store",
        }
    }

    /// What the check looks at, in the user's terms rather than the cmdlet's.
    /// Shown before the check has run and again under its details, because a
    /// verdict with no stated question behind it is not worth much.
    pub fn explanation(self) -> &'static str {
        match self {
            CheckId::Uptime => {
                "How long since the last restart. Past a week, pending updates and leaked driver memory start to show."
            }
            CheckId::PendingReboot => {
                "Whether Windows has already staged an update or a file rename that only takes effect after a restart."
            }
            CheckId::PowerPlan => {
                "Which Windows power plan is active. Power saver caps the processor, and a laptop on battery does the same thing again on top of it."
            }
            CheckId::MemoryPressure => {
                "How close memory is to its commit limit, and whether Windows is fetching pages back from disk. This is what a machine that feels slow with a quiet processor usually turns out to be doing."
            }
            CheckId::DriverAge => {
                "How old the third-party drivers are for the devices that cause stutter when their driver misbehaves: network, audio, storage and display."
            }
            CheckId::StartupLoad => {
                "How many programs Windows launches with you. Each one costs boot time and memory for as long as the session lasts."
            }
            CheckId::StorageSpace => {
                "Free space on every fixed drive. Below a few percent Windows cannot grow the page file or write an update, and everything slows at once."
            }
            CheckId::SearchIndexScope => {
                "Which drives Windows Search indexes. A spinning drive in the index is woken and kept spinning by a background crawl the user never asked for."
            }
            CheckId::PageFile => {
                "Whether a page file exists and how large it is. A disabled page file turns a memory spike into a closed program or a stop error."
            }
            CheckId::WindowsUpdate => {
                "When an update was last installed. Months of silence usually means updates are failing rather than that none were offered."
            }
            CheckId::CrashDumps => {
                "Stop errors -- blue screens -- recorded in the last 30 days, from both the crash dump folder and the bugcheck event."
            }
            CheckId::EventLog => {
                "Critical and Error entries recorded by the System and Application logs in the last 24 hours."
            }
            CheckId::DeviceProblems => {
                "Devices that are present but that Windows has flagged with an error, the same list Device Manager marks."
            }
            CheckId::SecurityPosture => {
                "Whether Microsoft Defender is on, real-time protection is running, and the signatures are recent."
            }
            CheckId::MemoryDiagnostic => {
                "Whether the Windows Memory Diagnostic has ever reported a fault. It reports only runs you started yourself."
            }
            CheckId::DiskHealth => {
                "What each physical drive reports about itself: health, operational state, and wear where the drive exposes it."
            }
            CheckId::VolumeDirtyBit => {
                "Whether a fixed volume is marked dirty, meaning Windows wants to check it at the next restart. Queried only, never set."
            }
            CheckId::SystemFiles => {
                "Whether DISM reports corruption in the Windows component store. Read-only: this does not repair anything."
            }
        }
    }

    /// Whether this check belongs to the silent health probe at startup.
    ///
    /// Six cheap reads that between them decide most of the score: how full
    /// the Windows drive is, how close memory is to its limit, whether
    /// Windows logged anything serious, whether a device is faulted, whether
    /// a restart is pending, and how long the machine has been up. Everything
    /// slower or narrower belongs to the diagnostic the user asks for.
    pub fn in_health_probe(self) -> bool {
        matches!(
            self,
            CheckId::Uptime
                | CheckId::PendingReboot
                | CheckId::StorageSpace
                | CheckId::MemoryPressure
                | CheckId::EventLog
                | CheckId::DeviceProblems
        )
    }

    /// The Windows tool that answers this check in more depth, as a button
    /// label and a shell target.
    ///
    /// Only read-only consoles and Settings pages are listed. A check that has
    /// no honest counterpart returns `None` rather than the nearest thing:
    /// uptime is answered by restarting, and the component store is repaired
    /// by DISM, and offering either from a page whose whole promise is that it
    /// only reads would be the page breaking its own rule. Those live on the
    /// Repair tab, behind its confirmation, where they belong.
    pub fn tool(self) -> Option<(&'static str, &'static str)> {
        match self {
            CheckId::PendingReboot | CheckId::WindowsUpdate => {
                Some(("Open Windows Update", "ms-settings:windowsupdate"))
            }
            CheckId::PowerPlan => Some(("Open Power options", "ms-settings:powersleep")),
            CheckId::StartupLoad => Some(("Open Startup apps", "ms-settings:startupapps")),
            CheckId::MemoryPressure => Some(("Open Resource Monitor", "resmon.exe")),
            CheckId::DriverAge => Some(("Open Device Manager", "devmgmt.msc")),
            CheckId::StorageSpace => Some(("Open Storage settings", "ms-settings:storagesense")),
            CheckId::SearchIndexScope => {
                Some(("Open Search settings", "ms-settings:cortana-windowssearch"))
            }
            CheckId::PageFile => Some((
                "Open Performance options",
                "SystemPropertiesPerformance.exe",
            )),
            // Reliability Monitor would be the friendlier window, but it is
            // `perfmon /rel` and the shell launcher takes a single token --
            // quoting the switch into it opens nothing at all.
            CheckId::CrashDumps => Some(("Open Event Viewer", "eventvwr.msc")),
            CheckId::EventLog => Some(("Open Event Viewer", "eventvwr.msc")),
            CheckId::DeviceProblems => Some(("Open Device Manager", "devmgmt.msc")),
            CheckId::SecurityPosture => Some(("Open Windows Security", "windowsdefender:")),
            CheckId::MemoryDiagnostic => Some(("Open Memory Diagnostic", "mdsched.exe")),
            CheckId::DiskHealth | CheckId::VolumeDirtyBit => {
                Some(("Open Disk Management", "diskmgmt.msc"))
            }
            CheckId::Uptime | CheckId::SystemFiles => None,
        }
    }
}

/// The verdict of one check.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CheckStatus {
    Pass,
    Warning,
    Problem,
    Skipped,
    NeedsAdmin,
    Error,
}

impl CheckStatus {
    /// The pill text.
    pub fn label(self) -> &'static str {
        match self {
            CheckStatus::Pass => "Pass",
            CheckStatus::Warning => "Warning",
            CheckStatus::Problem => "Problem",
            CheckStatus::Skipped => "Skipped",
            CheckStatus::NeedsAdmin => "Needs administrator",
            CheckStatus::Error => "Could not check",
        }
    }

    /// Whether this verdict is something the user has to act on.
    ///
    /// The old `severity()` that mapped onto the Overview's vocabulary is
    /// gone: it had to return `None` for a check that could not run, which is
    /// how a check that blew up ended up painted the same grey as one that was
    /// never meant to apply. Colour now comes from `check_color` in the app,
    /// which has a word for that case; what remains here is the one question
    /// the module itself can answer.
    pub fn is_finding(self) -> bool {
        matches!(
            self,
            CheckStatus::Warning | CheckStatus::NeedsAdmin | CheckStatus::Problem
        )
    }
}

/// What one finished check has to say.
#[derive(Clone, Debug)]
pub struct CheckOutcome {
    pub id: CheckId,
    pub status: CheckStatus,
    /// The single line shown on the collapsed row.
    pub summary: String,
    /// The expanded body. One entry per drawn line; each is pre-truncated,
    /// because these carry device names and command output straight from
    /// Windows and the Diagnose column is not wide.
    pub details: Vec<String>,
}

/// How much of the suite to run.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Scope {
    /// Everything. The diagnostic the user asked for.
    Full,
    /// The six the score leans on, for the silent startup pass.
    HealthProbe,
}

/// Progress from the scan worker.
///
/// `Started` and `Finished` exist so the checklist can show which check is
/// running and which are done, rather than one spinner for the whole sweep —
/// the same reason `treesize` grew its `Partial` message.
#[derive(Clone, Debug)]
pub enum CheckMessage {
    Started(CheckId),
    /// The event log check hands its full result over as well as its verdict,
    /// because the page still draws the per-event list and the Overview still
    /// scores on the finding count.
    Events(ScanResult),
    /// The verdict, and how long that check took on the worker thread.
    Finished(CheckOutcome, Duration),
    AllDone,
}

/// Run every machine check on one background thread.
///
/// One thread rather than one per check: these mostly shell out to PowerShell,
/// and nine simultaneous `powershell.exe` processes cost more than the scan
/// saves on a machine that is already being diagnosed for being slow.
pub fn start_system_scan(scope: Scope) -> std::sync::mpsc::Receiver<CheckMessage> {
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for id in CheckId::ALL {
            let wanted = match scope {
                Scope::Full => true,
                Scope::HealthProbe => id.in_health_probe(),
            };
            if !wanted {
                continue;
            }
            // A closed receiver means the window went away. Stop rather than
            // keep spawning child processes nobody will read the output of.
            if sender.send(CheckMessage::Started(id)).is_err() {
                return;
            }
            // Timed here rather than on the UI thread. The window drains this
            // channel on a 200ms poll and can take a whole batch in one frame,
            // so a clock read at the receiving end measures when the message
            // was collected, not how long the check ran.
            let started = std::time::Instant::now();
            let outcome = run_check(id, &sender);
            if sender
                .send(CheckMessage::Finished(outcome, started.elapsed()))
                .is_err()
            {
                return;
            }
        }
        let _ = sender.send(CheckMessage::AllDone);
    });
    receiver
}

fn run_check(id: CheckId, sender: &std::sync::mpsc::Sender<CheckMessage>) -> CheckOutcome {
    #[cfg(not(windows))]
    {
        let _ = sender;
        return CheckOutcome {
            id,
            status: CheckStatus::Skipped,
            summary: "This check only runs on Windows.".to_owned(),
            details: Vec::new(),
        };
    }

    #[cfg(windows)]
    {
        match id {
            CheckId::EventLog => {
                let result = scan_recent_events();
                let outcome = summarise_events(&result);
                let _ = sender.send(CheckMessage::Events(result));
                outcome
            }
            CheckId::Uptime => check_uptime(),
            CheckId::PendingReboot => check_pending_reboot(),
            CheckId::PowerPlan => check_power_plan(),
            CheckId::MemoryPressure => check_memory_pressure(),
            CheckId::DriverAge => check_driver_age(),
            CheckId::StartupLoad => check_startup_load(),
            CheckId::StorageSpace => check_storage_space(),
            CheckId::SearchIndexScope => check_search_index_scope(),
            CheckId::PageFile => check_page_file(),
            CheckId::WindowsUpdate => check_windows_update(),
            CheckId::CrashDumps => check_crash_dumps(),
            CheckId::DeviceProblems => check_device_problems(),
            CheckId::SecurityPosture => check_security_posture(),
            CheckId::MemoryDiagnostic => check_memory_diagnostic(),
            CheckId::DiskHealth => check_disk_health(),
            CheckId::VolumeDirtyBit => check_volume_dirty_bit(),
            CheckId::SystemFiles => check_system_files(),
        }
    }
}

/// Turn the event log result into a checklist row.
///
/// Eight is the same count at which `health::assess` stops calling recent
/// reliability events a warning and starts calling them bad. Two numbers for
/// one judgement is how the disk thresholds drifted apart, so this one is
/// borrowed rather than chosen again.
fn summarise_events(result: &ScanResult) -> CheckOutcome {
    let id = CheckId::EventLog;
    let count = result.findings.len();
    let mut details: Vec<String> = result
        .findings
        .iter()
        .take(8)
        .map(|finding| {
            format!(
                "{} · Event {} · {}",
                truncate(&finding.source, 70),
                finding.event_id,
                finding.time
            )
        })
        .collect();
    for error in &result.log_errors {
        details.push(truncate(error, 200));
    }
    match &result.status {
        ScanStatus::Error(error) => CheckOutcome {
            id,
            status: CheckStatus::Error,
            summary: truncate(error, 240),
            details,
        },
        ScanStatus::Unsupported => CheckOutcome {
            id,
            status: CheckStatus::Skipped,
            summary: "Windows event logs are only readable on Windows.".to_owned(),
            details,
        },
        ScanStatus::Complete | ScanStatus::Partial if count == 0 => CheckOutcome {
            id,
            status: CheckStatus::Pass,
            summary: "No Critical or Error events in the last 24 hours.".to_owned(),
            details,
        },
        ScanStatus::Complete | ScanStatus::Partial => CheckOutcome {
            id,
            status: if count >= 8 {
                CheckStatus::Problem
            } else {
                CheckStatus::Warning
            },
            summary: format!(
                "{} in the last 24 hours.",
                plural(count, "Critical or Error event")
            ),
            details,
        },
    }
}

// ---------------------------------------------------------------------------
// Shared process plumbing
// ---------------------------------------------------------------------------

/// Everything a finished child process left behind.
#[cfg(windows)]
struct CommandOutput {
    ok: bool,
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

#[cfg(windows)]
impl CommandOutput {
    /// The best sentence available for a non-zero exit. PowerShell usually
    /// writes something to stderr; `fsutil` and `Dism` sometimes do not, and
    /// "exited with code 740" at least names the number to search for.
    fn failure_detail(&self) -> String {
        let detail = self.stderr.trim();
        if !detail.is_empty() {
            return truncate(detail, 400);
        }
        match self.code {
            Some(code) => format!("The command exited with code {code}."),
            None => "The command was stopped before it finished.".to_owned(),
        }
    }
}

/// Run a child process with a deadline, capturing both pipes.
///
/// The deadline is the point of this function. `Get-PhysicalDisk` against a
/// drive that has stopped answering, and `Dism` against a damaged component
/// store, can both sit there indefinitely, and a scan is not allowed to be one
/// of them: on expiry the child is killed and the check reports a timeout.
///
/// Both pipes are drained on their own threads because a child that fills the
/// stdout buffer while we are blocked reading stderr never exits at all.
#[cfg(windows)]
fn run_bounded(
    program: &std::path::Path,
    args: &[&str],
    limit: Duration,
) -> Result<CommandOutput, String> {
    use std::os::windows::process::CommandExt;
    // Without this every check would flash a console window across whatever
    // the user is doing, nine times per scan.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let mut child = Command::new(program)
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|error| format!("Could not start {}: {error}", program.display()))?;

    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();
    let out_reader = std::thread::spawn(move || read_pipe(stdout_pipe));
    let err_reader = std::thread::spawn(move || read_pipe(stderr_pipe));
    let started = std::time::Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if started.elapsed() < limit => std::thread::sleep(Duration::from_millis(100)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "Could not read the status of {}: {error}",
                    program.display()
                ));
            }
        }
    };
    let stdout = out_reader.join().unwrap_or_default();
    let stderr = err_reader.join().unwrap_or_default();
    let Some(status) = status else {
        return Err(format!(
            "{} did not finish within {} seconds and was stopped.",
            program.display(),
            limit.as_secs()
        ));
    };
    Ok(CommandOutput {
        ok: status.success(),
        code: status.code(),
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
        stderr: String::from_utf8_lossy(&stderr).trim().to_owned(),
    })
}

/// Run a PowerShell script with a deadline.
#[cfg(windows)]
fn run_powershell(script: &str, limit: Duration) -> Result<CommandOutput, String> {
    run_bounded(
        &crate::util::powershell_path(),
        &[
            "-NoProfile",
            "-NonInteractive",
            "-WindowStyle",
            "Hidden",
            "-Command",
            script,
        ],
        limit,
    )
}

/// The front matter every check script shares.
///
/// `Emit` is the whole reason the scripts are readable. Each value is base64'd
/// and the fields are tab-joined, so a device friendly name with a comma in it,
/// or an `fsutil` reply with a line break in it, cannot be mistaken for a field
/// boundary. The event log scan established this shape and the rest follow it.
#[cfg(windows)]
const PS_PRELUDE: &str = r#"
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
function Emit($key, $values) {
  $encoded = foreach ($value in $values) { [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes([string]$value)) }
  [Console]::Out.WriteLine($key + "`t" + ($encoded -join "`t"))
}
"#;

#[cfg(windows)]
fn script(body: &str) -> String {
    format!("{PS_PRELUDE}\n{body}")
}

#[cfg(windows)]
fn failed(id: CheckId, detail: String) -> CheckOutcome {
    CheckOutcome {
        id,
        status: CheckStatus::Error,
        summary: truncate(&detail, 240),
        details: Vec::new(),
    }
}

#[cfg(windows)]
fn needs_admin(id: CheckId, detail: &str) -> CheckOutcome {
    CheckOutcome {
        id,
        status: CheckStatus::NeedsAdmin,
        summary: detail.to_owned(),
        details: vec![
            "Restart Wrenchmaster as administrator to include this check. Nothing is prompted for \
             from here, and the rest of the scan is unaffected."
                .to_owned(),
        ],
    }
}

// ---------------------------------------------------------------------------
// Pure parsers. Every one of these is a function of a &str so it can be tested
// against captured output on any machine, including the ones that have no
// `fsutil` to produce it.
// ---------------------------------------------------------------------------

/// Shorten to `limit` characters on a character boundary.
///
/// Byte slicing was the first attempt and it panicked the moment a device name
/// came back with an em dash in it. Overlong strings are a live bug in this UI,
/// so the truncation happens here rather than being left to whoever draws it.
pub fn truncate(value: &str, limit: usize) -> String {
    let mut out: String = value.chars().take(limit).collect();
    if value.chars().count() > limit {
        out.push('\u{2026}');
    }
    out
}

/// Split one `KEY<tab>base64<tab>base64...` line into its key and fields.
pub fn decode_line(line: &str) -> Option<(&str, Vec<String>)> {
    let mut parts = line.split('\t');
    let key = parts.next()?;
    if key.is_empty() {
        return None;
    }
    Some((key, parts.map(decode_base64_utf8).collect()))
}

/// Read one field without ever indexing. Checks are not allowed to panic on
/// output they did not produce, and a short line is the likeliest way for that
/// to happen, so a missing field reads as empty.
fn field(fields: &[String], index: usize) -> &str {
    fields.get(index).map(String::as_str).unwrap_or("").trim()
}

fn parse_number<T: std::str::FromStr>(value: &str) -> Option<T> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    trimmed.parse().ok()
}

/// PowerShell stringifies booleans as `True`/`False`.
fn parse_bool(value: &str) -> Option<bool> {
    match value.trim() {
        v if v.eq_ignore_ascii_case("true") || v == "1" => Some(true),
        v if v.eq_ignore_ascii_case("false") || v == "0" => Some(false),
        _ => None,
    }
}

/// Whether a check script reached its terminating `<KEY>OK` line. An empty
/// finding list means two very different things depending on this: nothing
/// wrong, or the query never ran.
pub fn report_completed(text: &str, key: &str) -> bool {
    text.lines()
        .any(|line| line.split('\t').next() == Some(key))
}

/// Error lines a check script emitted for itself.
pub fn report_errors(text: &str, key: &str) -> Vec<String> {
    text.lines()
        .filter_map(decode_line)
        .filter(|(line_key, _)| *line_key == key)
        .map(|(_, fields)| {
            let detail = field(&fields, 0);
            truncate(
                if detail.is_empty() {
                    "Unknown error"
                } else {
                    detail
                },
                240,
            )
        })
        .collect()
}

/// What `DISM /Online /Cleanup-Image /CheckHealth` concluded.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DismVerdict {
    Clean,
    Repairable,
    NotRepairable,
    Unknown,
}

/// Read the verdict out of DISM's report.
///
/// The sentence is all there is to go on: DISM exits 0 both for a clean store
/// and for a repairable one, so the exit code cannot be used. A localised
/// Windows prints a translated sentence and lands on `Unknown`, which the
/// check surfaces as "could not check" — the alternative, treating an
/// unrecognised report as clean, is the one answer that would be actively
/// misleading.
pub fn parse_dism_check_health(text: &str) -> DismVerdict {
    let lower = text.to_ascii_lowercase();
    if lower.contains("no component store corruption detected") {
        DismVerdict::Clean
    } else if lower.contains("not repairable") {
        DismVerdict::NotRepairable
    } else if lower.contains("component store is repairable")
        || lower.contains("corruption was detected")
    {
        DismVerdict::Repairable
    } else {
        DismVerdict::Unknown
    }
}

/// The lines of a DISM report worth showing, without the banner or the
/// progress bar, which is drawn with `[====` and is pure noise once captured.
pub fn dism_report_lines(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| !line.starts_with('['))
        .filter(|line| !line.starts_with("Deployment Image Servicing"))
        .filter(|line| !line.starts_with("Version:"))
        .filter(|line| !line.starts_with("Image Version:"))
        .map(|line| truncate(line, 200))
        .take(12)
        .collect()
}

/// One physical drive, with the reliability counters attached where Windows
/// exposed them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PhysicalDisk {
    pub name: String,
    pub media_type: String,
    pub health: String,
    pub operational: String,
    /// Percentage of the drive's rated write endurance consumed, for the SSDs
    /// that report it. `Get-StorageReliabilityCounter` does not expose a
    /// reallocated-sector count, so wear is the nearest thing available
    /// without reading raw SMART attributes.
    pub wear_percent: Option<u32>,
    pub temperature_celsius: Option<i32>,
    pub power_on_hours: Option<u64>,
    pub uncorrected_read_errors: Option<u64>,
}

/// Parse the `DISK` lines of the drive health script.
pub fn parse_physical_disks(text: &str) -> Vec<PhysicalDisk> {
    text.lines()
        .filter_map(decode_line)
        .filter(|(key, _)| *key == "DISK")
        .map(|(_, fields)| PhysicalDisk {
            name: truncate(field(&fields, 0), 70),
            media_type: truncate(field(&fields, 1), 24),
            health: truncate(field(&fields, 2), 24),
            operational: truncate(field(&fields, 3), 40),
            wear_percent: parse_number(field(&fields, 4)),
            temperature_celsius: parse_number(field(&fields, 5)),
            power_on_hours: parse_number(field(&fields, 6)),
            uncorrected_read_errors: parse_number(field(&fields, 7)),
        })
        .collect()
}

/// What `fsutil dirty query` said about one volume.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DirtyVerdict {
    Clean,
    Dirty,
    Denied,
    Unknown,
}

/// Read one `fsutil dirty query` reply.
///
/// Denial is tested first: the refusal text contains neither "is dirty" nor
/// "not dirty", and checking it last once meant an unelevated run reported
/// every volume as `Unknown` with no hint as to why. The phrasing is English,
/// so a localised Windows lands on `Unknown` and the check says it could not
/// read the flag rather than guessing at it.
pub fn parse_dirty_query(text: &str) -> DirtyVerdict {
    let lower = text.to_ascii_lowercase();
    if lower.contains("denied") || lower.contains("elevated") {
        DirtyVerdict::Denied
    } else if lower.contains("not dirty") {
        DirtyVerdict::Clean
    } else if lower.contains("is dirty") {
        DirtyVerdict::Dirty
    } else {
        DirtyVerdict::Unknown
    }
}

/// Parse the `DIRTY` lines into `(volume, verdict)` pairs.
pub fn parse_dirty_report(text: &str) -> Vec<(String, DirtyVerdict)> {
    text.lines()
        .filter_map(decode_line)
        .filter(|(key, _)| *key == "DIRTY")
        .map(|(_, fields)| {
            (
                truncate(field(&fields, 0), 16),
                parse_dirty_query(field(&fields, 1)),
            )
        })
        .collect()
}

/// A present device Windows has flagged with an error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceProblem {
    pub name: String,
    pub status: String,
    pub class: String,
    pub instance_id: String,
}

/// Parse the `PNP` lines of the device script.
pub fn parse_device_problems(text: &str) -> Vec<DeviceProblem> {
    text.lines()
        .filter_map(decode_line)
        .filter(|(key, _)| *key == "PNP")
        .map(|(_, fields)| {
            let name = field(&fields, 0);
            DeviceProblem {
                // An unnamed device is normal for exactly the devices that are
                // broken enough to land in this list.
                name: truncate(
                    if name.is_empty() {
                        "Unnamed device"
                    } else {
                        name
                    },
                    70,
                ),
                status: truncate(field(&fields, 1), 24),
                class: truncate(field(&fields, 2), 32),
                instance_id: truncate(field(&fields, 3), 90),
            }
        })
        .collect()
}

/// What `Get-MpComputerStatus` reported.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct DefenderStatus {
    pub antivirus_enabled: Option<bool>,
    pub realtime_enabled: Option<bool>,
    pub antispyware_enabled: Option<bool>,
    pub service_enabled: Option<bool>,
    pub signature_age_days: Option<u32>,
    pub signature_updated: String,
}

/// Parse the single `MP` line of the security script.
pub fn parse_defender_status(text: &str) -> Option<DefenderStatus> {
    text.lines()
        .filter_map(decode_line)
        .find(|(key, _)| *key == "MP")
        .map(|(_, fields)| DefenderStatus {
            antivirus_enabled: parse_bool(field(&fields, 0)),
            realtime_enabled: parse_bool(field(&fields, 1)),
            antispyware_enabled: parse_bool(field(&fields, 2)),
            service_enabled: parse_bool(field(&fields, 3)),
            signature_age_days: parse_number(field(&fields, 4)),
            signature_updated: truncate(field(&fields, 5), 40),
        })
}

/// A `Microsoft-Windows-MemoryDiagnostics-Results` entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoryDiagnosticEvent {
    pub event_id: u32,
    pub level: String,
    pub time: String,
    pub message: String,
}

/// Parse the `MEMDIAG` lines of the memory diagnostic script.
pub fn parse_memory_diagnostics(text: &str) -> Vec<MemoryDiagnosticEvent> {
    text.lines()
        .filter_map(decode_line)
        .filter(|(key, _)| *key == "MEMDIAG")
        .map(|(_, fields)| MemoryDiagnosticEvent {
            event_id: parse_number(field(&fields, 0)).unwrap_or(0),
            level: truncate(field(&fields, 1), 24),
            time: truncate(field(&fields, 2), 24),
            message: truncate(field(&fields, 3), 400),
        })
        .collect()
}

/// Whether any memory diagnostic run ended badly.
///
/// 1201 is the "no errors" result and 1202 is the failure; the level is
/// consulted as well because a future build could add a third id, and an
/// Error-level entry from this provider is not something to pass silently.
pub fn memory_diagnostic_failed(events: &[MemoryDiagnosticEvent]) -> bool {
    events.iter().any(|event| {
        event.event_id == 1202
            || event.level.eq_ignore_ascii_case("Error")
            || event.level.eq_ignore_ascii_case("Critical")
    })
}

/// Parse the single `BOOT` line into the boot timestamp and uptime in hours.
pub fn parse_boot_report(text: &str) -> Option<(String, f64)> {
    text.lines()
        .filter_map(decode_line)
        .find(|(key, _)| *key == "BOOT")
        .and_then(|(_, fields)| {
            let hours: f64 = parse_number(field(&fields, 1))?;
            Some((truncate(field(&fields, 0), 32), hours))
        })
}

/// Whether a failure message is really a missing administrator token.
///
/// Worth distinguishing because "Access is denied" shown as a red error reads
/// as a broken machine, when all it means is that the user declined the UAC
/// prompt at startup.
pub fn looks_like_access_denied(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("access is denied")
        || lower.contains("access denied")
        || lower.contains("unauthorizedaccess")
        || lower.contains("requires elevation")
        || lower.contains("elevated administrator")
}

// ---------------------------------------------------------------------------
// The checks
// ---------------------------------------------------------------------------

#[cfg(windows)]
fn check_uptime() -> CheckOutcome {
    let id = CheckId::Uptime;
    const BODY: &str = r#"
try {
  $os = Get-CimInstance Win32_OperatingSystem -ErrorAction Stop
  $boot = $os.LastBootUpTime
  $hours = ((Get-Date) - $boot).TotalHours
  Emit 'BOOT' @($boot.ToString('yyyy-MM-dd HH:mm:ss'), [string][math]::Round($hours, 2))
} catch { Emit 'BOOTERR' @($_.Exception.Message) }
"#;
    let output = match run_powershell(&script(BODY), Duration::from_secs(20)) {
        Ok(output) => output,
        Err(error) => return failed(id, error),
    };
    let Some((boot, hours)) = parse_boot_report(&output.stdout) else {
        let mut detail = report_errors(&output.stdout, "BOOTERR").join("; ");
        if detail.is_empty() {
            detail = output.failure_detail();
        }
        return failed(id, format!("Could not read the last boot time: {detail}"));
    };
    let days = hours / 24.0;
    let details = vec![format!(
        "Last started {boot} ({} ago)",
        human_duration(hours)
    )];
    // The same two bands health.rs uses for uptime, for the same reason the
    // disk thresholds were unified: one judgement, one pair of numbers.
    if days >= 7.0 {
        CheckOutcome {
            id,
            status: CheckStatus::Warning,
            summary: format!(
                "Up for {}. A restart would apply anything Windows has staged.",
                human_duration(hours)
            ),
            details,
        }
    } else {
        CheckOutcome {
            id,
            status: CheckStatus::Pass,
            summary: format!("Up for {}.", human_duration(hours)),
            details,
        }
    }
}

/// Pending-restart markers, read through `winreg` rather than PowerShell.
///
/// These are three registry lookups and nothing else; spawning a shell to ask
/// about them would cost more than the whole check. All three live under HKLM
/// but are readable without elevation, so this one never needs admin.
#[cfg(windows)]
fn check_pending_reboot() -> CheckOutcome {
    use winreg::RegKey;
    use winreg::enums::{HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_64KEY};

    let id = CheckId::PendingReboot;
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let flags = KEY_READ | KEY_WOW64_64KEY;
    let mut reasons: Vec<String> = Vec::new();
    let mut notes: Vec<String> = Vec::new();

    for (path, reason) in [
        (
            r"SOFTWARE\Microsoft\Windows\CurrentVersion\Component Based Servicing\RebootPending",
            "Component Based Servicing has a servicing operation waiting on a restart.",
        ),
        (
            r"SOFTWARE\Microsoft\Windows\CurrentVersion\WindowsUpdate\Auto Update\RebootRequired",
            "Windows Update is waiting for a restart to finish installing.",
        ),
    ] {
        if hklm.open_subkey_with_flags(path, flags).is_ok() {
            reasons.push(reason.to_owned());
        }
    }

    match hklm.open_subkey_with_flags(r"SYSTEM\CurrentControlSet\Control\Session Manager", flags) {
        Ok(key) => {
            if let Ok(value) = key.get_raw_value("PendingFileRenameOperations") {
                // REG_MULTI_SZ carries its own terminator, so an empty list is
                // still two bytes on disk and the presence of the value alone
                // proves nothing.
                if value.bytes.len() > 2 {
                    reasons.push(
                        "Files are queued to be renamed or deleted at the next restart.".to_owned(),
                    );
                }
            }
        }
        Err(error) => notes.push(truncate(
            &format!("Could not read the Session Manager key: {error}"),
            200,
        )),
    }

    let mut details = reasons.clone();
    details.append(&mut notes);
    if reasons.is_empty() {
        CheckOutcome {
            id,
            status: CheckStatus::Pass,
            summary: "Nothing is waiting for a restart.".to_owned(),
            details,
        }
    } else {
        CheckOutcome {
            id,
            status: CheckStatus::Warning,
            summary: format!("{} set.", plural(reasons.len(), "restart marker")),
            details,
        }
    }
}

#[cfg(windows)]
fn check_device_problems() -> CheckOutcome {
    let id = CheckId::DeviceProblems;
    const BODY: &str = r#"
try {
  $devices = @(Get-PnpDevice -Status Error -PresentOnly -ErrorAction Stop)
  foreach ($device in $devices) {
    Emit 'PNP' @($device.FriendlyName, $device.Status, $device.Class, $device.InstanceId)
  }
  Emit 'PNPOK' @([string]$devices.Count)
} catch { Emit 'PNPERR' @($_.Exception.Message) }
"#;
    let output = match run_powershell(&script(BODY), Duration::from_secs(45)) {
        Ok(output) => output,
        Err(error) => return failed(id, error),
    };
    let errors = report_errors(&output.stdout, "PNPERR");
    if !report_completed(&output.stdout, "PNPOK") {
        let detail = if errors.is_empty() {
            output.failure_detail()
        } else {
            errors.join("; ")
        };
        return failed(id, format!("Could not list devices: {detail}"));
    }
    let devices = parse_device_problems(&output.stdout);
    if devices.is_empty() {
        return CheckOutcome {
            id,
            status: CheckStatus::Pass,
            summary: "No present device is reporting an error.".to_owned(),
            details: errors,
        };
    }
    let mut details: Vec<String> = devices
        .iter()
        .take(20)
        .map(|device| {
            format!(
                "{} · {} · class {} · {}",
                device.name, device.status, device.class, device.instance_id
            )
        })
        .collect();
    details.extend(errors);
    CheckOutcome {
        id,
        status: CheckStatus::Warning,
        summary: format!("{} reporting an error.", plural(devices.len(), "device")),
        details,
    }
}

#[cfg(windows)]
fn check_security_posture() -> CheckOutcome {
    let id = CheckId::SecurityPosture;
    const BODY: &str = r#"
try {
  $status = Get-MpComputerStatus -ErrorAction Stop
  Emit 'MP' @($status.AntivirusEnabled, $status.RealTimeProtectionEnabled, $status.AntispywareEnabled, $status.AMServiceEnabled, $status.AntivirusSignatureAge, $status.AntivirusSignatureLastUpdated)
} catch { Emit 'MPERR' @($_.Exception.Message) }
"#;
    let output = match run_powershell(&script(BODY), Duration::from_secs(45)) {
        Ok(output) => output,
        Err(error) => return failed(id, error),
    };
    let errors = report_errors(&output.stdout, "MPERR");
    let Some(status) = parse_defender_status(&output.stdout) else {
        let detail = if errors.is_empty() {
            output.failure_detail()
        } else {
            errors.join("; ")
        };
        // A third-party antivirus replaces Defender's cmdlets outright, so a
        // missing `Get-MpComputerStatus` is not by itself bad news.
        if looks_like_access_denied(&detail) {
            return needs_admin(id, "Defender status is only readable by an administrator.");
        }
        return CheckOutcome {
            id,
            status: CheckStatus::Skipped,
            summary:
                "Microsoft Defender did not answer; another antivirus may be managing this PC."
                    .to_owned(),
            details: vec![truncate(&detail, 300)],
        };
    };

    let shown = |value: Option<bool>| match value {
        Some(true) => "on",
        Some(false) => "off",
        None => "unknown",
    };
    let mut details = vec![
        format!(
            "Antivirus {} · real-time protection {} · antispyware {} · service {}",
            shown(status.antivirus_enabled),
            shown(status.realtime_enabled),
            shown(status.antispyware_enabled),
            shown(status.service_enabled)
        ),
        match status.signature_age_days {
            Some(age) => format!(
                "Signatures are {} old, last updated {}",
                plural(age as usize, "day"),
                if status.signature_updated.is_empty() {
                    "at an unreported time"
                } else {
                    status.signature_updated.as_str()
                }
            ),
            None => "Signature age was not reported.".to_owned(),
        },
    ];
    details.extend(errors);

    if status.realtime_enabled == Some(false) || status.antivirus_enabled == Some(false) {
        CheckOutcome {
            id,
            status: CheckStatus::Problem,
            summary: "Defender real-time protection is off.".to_owned(),
            details,
        }
    } else if status.signature_age_days.is_some_and(|age| age >= 7) {
        CheckOutcome {
            id,
            status: CheckStatus::Warning,
            summary: "Defender is on, but its signatures are a week or more out of date."
                .to_owned(),
            details,
        }
    } else {
        CheckOutcome {
            id,
            status: CheckStatus::Pass,
            summary: "Defender is on with recent signatures.".to_owned(),
            details,
        }
    }
}

#[cfg(windows)]
fn check_memory_diagnostic() -> CheckOutcome {
    let id = CheckId::MemoryDiagnostic;
    const BODY: &str = r#"
try {
  $events = @(Get-WinEvent -FilterHashtable @{ LogName = 'System'; ProviderName = 'Microsoft-Windows-MemoryDiagnostics-Results' } -MaxEvents 10 -ErrorAction SilentlyContinue)
  foreach ($event in $events) {
    $message = [string]$event.Message
    if ($message.Length -gt 600) { $message = $message.Substring(0, 600) }
    Emit 'MEMDIAG' @([string]$event.Id, [string]$event.LevelDisplayName, $event.TimeCreated.ToString('yyyy-MM-dd HH:mm:ss'), $message)
  }
  Emit 'MEMDIAGOK' @([string]$events.Count)
} catch { Emit 'MEMDIAGERR' @($_.Exception.Message) }
"#;
    let output = match run_powershell(&script(BODY), Duration::from_secs(45)) {
        Ok(output) => output,
        Err(error) => return failed(id, error),
    };
    let errors = report_errors(&output.stdout, "MEMDIAGERR");
    if !report_completed(&output.stdout, "MEMDIAGOK") {
        let detail = if errors.is_empty() {
            output.failure_detail()
        } else {
            errors.join("; ")
        };
        return failed(
            id,
            format!("Could not read memory diagnostic results: {detail}"),
        );
    }
    let events = parse_memory_diagnostics(&output.stdout);
    let mut details: Vec<String> = events
        .iter()
        .map(|event| {
            format!(
                "{} · {} · event {} · {}",
                event.time, event.level, event.event_id, event.message
            )
        })
        .collect();
    details.extend(errors);
    if events.is_empty() {
        CheckOutcome {
            id,
            status: CheckStatus::Pass,
            summary: "No memory diagnostic has ever reported a result on this PC.".to_owned(),
            details: vec![
                "The Windows Memory Diagnostic only writes a result when you run it, so an empty \
                 history is normal and is not evidence that the RAM is good."
                    .to_owned(),
            ],
        }
    } else if memory_diagnostic_failed(&events) {
        CheckOutcome {
            id,
            status: CheckStatus::Problem,
            summary: "A memory diagnostic run reported a hardware fault.".to_owned(),
            details,
        }
    } else {
        CheckOutcome {
            id,
            status: CheckStatus::Pass,
            summary: format!("{}, none failed.", events.len()),
            details,
        }
    }
}

#[cfg(windows)]
fn check_disk_health() -> CheckOutcome {
    let id = CheckId::DiskHealth;
    // The reliability counters are fetched inside their own try: they need an
    // administrator token and are missing entirely on USB bridges and some RAID
    // controllers, and neither case should cost us the drive list itself.
    const BODY: &str = r#"
try {
  $disks = @(Get-PhysicalDisk -ErrorAction Stop)
  foreach ($disk in $disks) {
    $wear = ''
    $temperature = ''
    $hours = ''
    $readErrors = ''
    try {
      $counter = $disk | Get-StorageReliabilityCounter -ErrorAction Stop
      if ($counter) {
        $wear = [string]$counter.Wear
        $temperature = [string]$counter.Temperature
        $hours = [string]$counter.PowerOnHours
        $readErrors = [string]$counter.ReadErrorsUncorrected
      }
    } catch { }
    Emit 'DISK' @($disk.FriendlyName, $disk.MediaType, $disk.HealthStatus, $disk.OperationalStatus, $wear, $temperature, $hours, $readErrors)
  }
  Emit 'DISKOK' @([string]$disks.Count)
} catch { Emit 'DISKERR' @($_.Exception.Message) }
"#;
    let output = match run_powershell(&script(BODY), Duration::from_secs(60)) {
        Ok(output) => output,
        Err(error) => return failed(id, error),
    };
    let errors = report_errors(&output.stdout, "DISKERR");
    if !report_completed(&output.stdout, "DISKOK") {
        let detail = if errors.is_empty() {
            output.failure_detail()
        } else {
            errors.join("; ")
        };
        if looks_like_access_denied(&detail) {
            return needs_admin(
                id,
                "The storage subsystem answers drive queries only to an administrator.",
            );
        }
        return failed(id, format!("Could not list physical drives: {detail}"));
    }
    let disks = parse_physical_disks(&output.stdout);
    if disks.is_empty() {
        return CheckOutcome {
            id,
            status: CheckStatus::Skipped,
            summary: "Windows did not report any physical drive.".to_owned(),
            details: errors,
        };
    }

    let mut status = CheckStatus::Pass;
    let mut details: Vec<String> = Vec::new();
    let mut unhealthy = 0usize;
    for disk in &disks {
        let mut line = format!(
            "{} · {} · health {} · {}",
            if disk.name.is_empty() {
                "Unnamed drive"
            } else {
                disk.name.as_str()
            },
            if disk.media_type.is_empty() {
                "unknown media"
            } else {
                disk.media_type.as_str()
            },
            if disk.health.is_empty() {
                "unreported"
            } else {
                disk.health.as_str()
            },
            if disk.operational.is_empty() {
                "unreported"
            } else {
                disk.operational.as_str()
            }
        );
        if let Some(wear) = disk.wear_percent {
            line.push_str(&format!(" · wear {wear}%"));
        }
        if let Some(temperature) = disk.temperature_celsius {
            line.push_str(&format!(" · {temperature} \u{b0}C"));
        }
        if let Some(hours) = disk.power_on_hours {
            line.push_str(&format!(" · {hours} h powered"));
        }
        if let Some(read_errors) = disk.uncorrected_read_errors {
            line.push_str(&format!(
                " · {}",
                plural(read_errors as usize, "uncorrected read error")
            ));
        }
        details.push(truncate(&line, 300));

        let healthy = disk.health.eq_ignore_ascii_case("Healthy");
        let warning = disk.health.eq_ignore_ascii_case("Warning");
        if !healthy {
            unhealthy += 1;
        }
        if healthy {
            // Degraded endurance is a warning long before the drive admits to
            // being anything other than Healthy, which is the whole reason the
            // reliability counters are read at all.
            // Wear is a prediction and drives routinely outlive their rated
            // endurance, so it stays a warning however high it climbs. An
            // uncorrected read error is not a prediction: the drive was asked
            // for data and could not return it, which has already cost the
            // user something.
            if disk
                .uncorrected_read_errors
                .is_some_and(|errors| errors > 0)
            {
                status = worse(status, CheckStatus::Problem);
            } else if disk.wear_percent.is_some_and(|wear| wear >= 80) {
                status = worse(status, CheckStatus::Warning);
            }
        } else if warning || disk.health.is_empty() {
            status = worse(status, CheckStatus::Warning);
        } else {
            status = worse(status, CheckStatus::Problem);
        }
    }
    details.extend(errors);
    let summary = if unhealthy == 0 && status == CheckStatus::Pass {
        format!(
            "{} report themselves healthy.",
            plural(disks.len(), "drive")
        )
    } else if unhealthy == 0 {
        format!(
            "{} report healthy, but the wear or error counters are worth a look.",
            disks.len()
        )
    } else {
        format!(
            "{unhealthy} of {} not reporting a healthy state.",
            disks.len()
        )
    };
    CheckOutcome {
        id,
        status,
        summary,
        details,
    }
}

// ---------------------------------------------------------------------------
// The facets a slow or unstable machine usually turns out to be hiding in.
//
// These were added because the first nine answered "is Windows damaged" well
// and "why is this machine unpleasant to use" not at all. A capped processor,
// forty startup programs, a drive with 2 GB left, no page file, a year of
// failed updates and a weekly stop error are all common, all cheap to read,
// and none of them show up in a component-store scan.
// ---------------------------------------------------------------------------

/// Which power plan is active, and whether the machine is on battery.
///
/// Power saver on a desktop is nearly always an accident -- an OEM tool or a
/// game launcher set it and nothing set it back -- and it caps the processor
/// hard enough to be the whole complaint on its own.
#[cfg(windows)]
fn check_power_plan() -> CheckOutcome {
    let id = CheckId::PowerPlan;
    // `powercfg`, not `Win32_PowerPlan`. The CIM class lives in
    // root\cimv2\power and reading it unelevated fails outright -- the UAT on
    // an ordinary session got "only administrators have permission", which
    // would have put a "Could not check" row on this page for every user who
    // runs the app the way it is meant to be run. `powercfg
    // /getactivescheme` answers the same question with no token at all.
    const BODY: &str = r#"
try {
  $active = (& powercfg.exe /getactivescheme 2>&1 | Out-String).Trim()
  if ($LASTEXITCODE -ne 0 -or -not $active) { throw 'powercfg did not report an active scheme.' }
  $battery = @(Get-CimInstance -ClassName Win32_Battery -ErrorAction SilentlyContinue)
  $onBattery = $false
  if ($battery.Count -gt 0) { $onBattery = ($battery[0].BatteryStatus -eq 1) }
  Emit 'PLAN' @($active, [string]$onBattery, [string]($battery.Count -gt 0))
  Emit 'PLANOK' @('1')
} catch { Emit 'PLANERR' @($_.Exception.Message) }
"#;
    let output = match run_powershell(&script(BODY), Duration::from_secs(30)) {
        Ok(output) => output,
        Err(error) => return failed(id, error),
    };
    let errors = report_errors(&output.stdout, "PLANERR");
    if !report_completed(&output.stdout, "PLANOK") {
        let detail = if errors.is_empty() {
            output.failure_detail()
        } else {
            errors.join("; ")
        };
        return failed(id, format!("Could not read the power plan: {detail}"));
    }
    let Some(fields) = output
        .stdout
        .lines()
        .filter_map(decode_line)
        .find(|(key, _)| *key == "PLAN")
        .map(|(_, fields)| fields)
    else {
        return failed(id, "Windows named no active power plan.".to_owned());
    };
    let reported = field(&fields, 0);
    let on_battery = field(&fields, 1).eq_ignore_ascii_case("true");
    let has_battery = field(&fields, 2).eq_ignore_ascii_case("true");
    let (guid, name) = parse_active_scheme(reported);
    // By GUID, never by name. `powercfg` prints the scheme name in the
    // display language -- this was first written to match on "saver" and the
    // machine it was tested on answered in Italian, which is the whole
    // argument against matching on words.
    let saver = guid.eq_ignore_ascii_case(POWER_SAVER_GUID);

    let mut details = vec![format!("Active plan: {}", truncate(&name, 120))];
    if has_battery {
        details.push(
            if on_battery {
                "Running on battery, where Windows applies its own additional limits."
            } else {
                "Plugged in."
            }
            .to_owned(),
        );
    }
    details.extend(errors);

    // Balanced is not reported as a finding. It is the Windows default, it
    // boosts on demand, and calling it a problem would make this check cry
    // wolf on the majority of healthy machines.
    let (status, summary) = if saver {
        (
            CheckStatus::Warning,
            format!(
                "Power saver is active ({}). The processor is capped.",
                truncate(&name, 60)
            ),
        )
    } else {
        (
            CheckStatus::Pass,
            format!("{} is active.", truncate(&name, 80)),
        )
    };
    CheckOutcome {
        id,
        status,
        summary,
        details,
    }
}

/// What Windows launches at sign-in.
///
/// Counted from `Win32_StartupCommand`, which covers the Run keys and the
/// Startup folders. It does not cover scheduled tasks or services, so the
/// summary says "programs" rather than claiming to be the whole boot cost.
#[cfg(windows)]
fn check_startup_load() -> CheckOutcome {
    let id = CheckId::StartupLoad;
    const BODY: &str = r#"
try {
  $items = @(Get-CimInstance -ClassName Win32_StartupCommand -ErrorAction Stop)
  foreach ($item in $items) { Emit 'RUN' @($item.Name, $item.Location, $item.Command) }
  Emit 'RUNOK' @([string]$items.Count)
} catch { Emit 'RUNERR' @($_.Exception.Message) }
"#;
    let output = match run_powershell(&script(BODY), Duration::from_secs(45)) {
        Ok(output) => output,
        Err(error) => return failed(id, error),
    };
    let errors = report_errors(&output.stdout, "RUNERR");
    if !report_completed(&output.stdout, "RUNOK") {
        let detail = if errors.is_empty() {
            output.failure_detail()
        } else {
            errors.join("; ")
        };
        return failed(id, format!("Could not list startup programs: {detail}"));
    }
    let entries: Vec<(String, String)> = output
        .stdout
        .lines()
        .filter_map(decode_line)
        .filter(|(key, _)| *key == "RUN")
        .map(|(_, fields)| {
            (
                truncate(field(&fields, 0), 70),
                truncate(field(&fields, 2), 120),
            )
        })
        .collect();

    let mut details: Vec<String> = entries
        .iter()
        .take(20)
        .map(|(name, command)| format!("{name} · {command}"))
        .collect();
    details.extend(errors);

    // Twelve is where this starts being worth saying. A clean Windows install
    // with a vendor audio panel and a cloud drive lands around five; past a
    // dozen the sign-in is visibly slower, and the list is long enough that
    // the user has stopped noticing what is in it.
    let (status, summary) = match entries.len() {
        0 => (
            CheckStatus::Pass,
            "Nothing starts with Windows beyond the system's own services.".to_owned(),
        ),
        count if count <= 12 => (
            CheckStatus::Pass,
            format!("{} start with Windows.", plural(count, "program")),
        ),
        count => (
            CheckStatus::Warning,
            format!(
                "{} start with Windows, which is enough to slow the sign-in.",
                plural(count, "program")
            ),
        ),
    };
    CheckOutcome {
        id,
        status,
        summary,
        details,
    }
}

/// Free space on every fixed drive.
///
/// Both a percentage and an absolute floor, because neither alone is right:
/// 5% of a 4 TB drive is 200 GB and fine, while 5 GB free on any drive is not,
/// and 10% of a 128 GB laptop drive is already too tight for a feature update.
#[cfg(windows)]
fn check_storage_space() -> CheckOutcome {
    let id = CheckId::StorageSpace;
    const BODY: &str = r#"
try {
  $system = ($env:SystemDrive).TrimEnd(':')
  $media = @{}
  try {
    foreach ($disk in @(Get-PhysicalDisk -ErrorAction Stop)) {
      foreach ($part in @(Get-Partition -DiskNumber $disk.DeviceId -ErrorAction SilentlyContinue)) {
        if ($part.DriveLetter) { $media[[string]$part.DriveLetter] = [string]$disk.MediaType }
      }
    }
  } catch { }
  $volumes = @(Get-Volume -ErrorAction Stop | Where-Object { $_.DriveType -eq 'Fixed' -and $_.Size -gt 0 })
  foreach ($volume in $volumes) {
    $letter = [string]$volume.DriveLetter
    $kind = ''
    if ($letter -and $media.ContainsKey($letter)) { $kind = $media[$letter] }
    Emit 'VOL' @($volume.DriveLetter, $volume.FileSystemLabel, [string]$volume.Size, [string]$volume.SizeRemaining, [string]($letter -eq $system), $kind)
  }
  Emit 'VOLOK' @([string]$volumes.Count)
} catch { Emit 'VOLERR' @($_.Exception.Message) }
"#;
    let output = match run_powershell(&script(BODY), Duration::from_secs(45)) {
        Ok(output) => output,
        Err(error) => return failed(id, error),
    };
    let errors = report_errors(&output.stdout, "VOLERR");
    if !report_completed(&output.stdout, "VOLOK") {
        let detail = if errors.is_empty() {
            output.failure_detail()
        } else {
            errors.join("; ")
        };
        return failed(id, format!("Could not read drive free space: {detail}"));
    }

    // Two scales, because one alone is wrong in both directions. The UAT
    // machine has 130 GB free on a 2 TB drive -- 6.5%, which a percentage
    // rule calls low and which is in fact plenty -- and a 974 MB recovery
    // partition, which an absolute rule calls critical on every machine ever
    // made. So a drive is only short of space when it is short by both, and
    // the floors are set where Windows itself starts to struggle: a feature
    // update wants about 20 GB, and under 5 GB the page file cannot grow.
    const CRITICAL_BYTES: u64 = 5 * 1024 * 1024 * 1024;
    const TIGHT_BYTES: u64 = 20 * 1024 * 1024 * 1024;
    // A second drive with nothing left on it is a storage problem, not a
    // performance one. Windows pages, updates, writes its temp files and
    // builds its search index on the system drive; a full games or archive
    // drive inconveniences the user and slows nothing. Saying otherwise
    // teaches people to ignore this row, which costs them the one case --
    // the system drive -- where it is urgent. So the floors are only applied
    // in full to the system volume, and a secondary one has to be genuinely
    // out of room before it is mentioned at all.
    // Where Windows itself stops calling a drive comfortable: Explorer turns
    // the capacity bar red at 90% used, and a solid-state drive wants about
    // this much free to keep garbage collection ahead of writes.
    const SYSTEM_HEADROOM_FRACTION: f64 = 0.10;
    // The band above "out of headroom", where it is worth saying but not
    // worth acting on yet.
    const TIGHT_FRACTION: f64 = 0.20;
    const SECONDARY_BYTES: u64 = 2 * 1024 * 1024 * 1024;
    const SECONDARY_FRACTION: f64 = 0.01;

    let mut status = CheckStatus::Pass;
    let mut details = Vec::new();
    let mut tight = Vec::new();
    let mut full_secondary = Vec::new();
    let mut counted = 0usize;
    for (_, fields) in output
        .stdout
        .lines()
        .filter_map(decode_line)
        .filter(|(key, _)| *key == "VOL")
    {
        let letter = field(&fields, 0);
        let label = field(&fields, 1);
        // No drive letter means a recovery or EFI partition: small, always
        // nearly full by design, and nothing the user can or should clear.
        if letter.is_empty() {
            continue;
        }
        let Ok(size) = field(&fields, 2).parse::<u64>() else {
            continue;
        };
        let Ok(free) = field(&fields, 3).parse::<u64>() else {
            continue;
        };
        if size == 0 {
            continue;
        }
        counted += 1;
        let fraction = free as f64 / size as f64;
        let is_system = field(&fields, 4).eq_ignore_ascii_case("true");
        let media = field(&fields, 5);
        let name = if label.is_empty() {
            format!("{letter}:")
        } else {
            format!("{letter}: {}", truncate(label, 30))
        };
        let kind = match media.to_ascii_uppercase().as_str() {
            "HDD" => " · spinning disk",
            "SSD" => " · SSD",
            _ => "",
        };
        details.push(format!(
            "{name}{}{kind} · {} free of {} ({:.0}%)",
            if is_system { " · Windows drive" } else { "" },
            crate::util::human_bytes(free),
            crate::util::human_bytes(size),
            fraction * 100.0
        ));
        if is_system {
            // Proportion and gigabytes are different questions: bytes say
            // whether an update can stage, percentage whether the drive can
            // still manage its own writes. Past 90% full the Windows drive
            // fails the second one, and that is a problem, not a note --
            // Explorer turns its own bar red at the same point.
            if fraction < SYSTEM_HEADROOM_FRACTION || free < CRITICAL_BYTES {
                status = worse(status, CheckStatus::Problem);
                tight.push(format!("{name} (Windows)"));
            } else if fraction < TIGHT_FRACTION || free < TIGHT_BYTES {
                status = worse(status, CheckStatus::Warning);
                tight.push(format!("{name} (Windows)"));
            }
        } else if fraction < SECONDARY_FRACTION || free < SECONDARY_BYTES {
            status = worse(status, CheckStatus::Warning);
            full_secondary.push(name);
        }
    }
    details.extend(errors);

    if counted == 0 {
        return CheckOutcome {
            id,
            status: CheckStatus::Error,
            summary: "No lettered fixed drive reported its size.".to_owned(),
            details,
        };
    }
    let summary = match (tight.is_empty(), full_secondary.is_empty()) {
        (true, true) => format!("All {} have room to work.", plural(counted, "fixed drive")),
        (false, _) => format!(
            "Little room left on {}, which is where Windows pages, updates and              manages its own writes.",
            tight.join(", ")
        ),
        // Said plainly, including the part that makes it not urgent. The
        // alternative is a user clearing a games drive to fix a slowdown it
        // was never causing.
        (true, false) => format!(
            "{} is out of room. It does not slow Windows, but nothing more can be written to it.",
            full_secondary.join(", ")
        ),
    };
    CheckOutcome {
        id,
        status,
        summary,
        details,
    }
}

/// Whether a page file exists at all.
///
/// The check is deliberately about existence rather than size. "Disable the
/// page file, you have 32 GB" is advice that circulates widely and turns a
/// memory spike into a closed program or a stop error; Windows managing its
/// own size is the normal, correct state and is reported as a pass.
#[cfg(windows)]
fn check_page_file() -> CheckOutcome {
    let id = CheckId::PageFile;
    const BODY: &str = r#"
try {
  $system = Get-CimInstance -ClassName Win32_ComputerSystem -ErrorAction Stop
  $files = @(Get-CimInstance -ClassName Win32_PageFileUsage -ErrorAction SilentlyContinue)
  foreach ($file in $files) { Emit 'PF' @($file.Name, [string]$file.AllocatedBaseSize, [string]$file.CurrentUsage, [string]$file.PeakUsage) }
  Emit 'PFSYS' @([string]$system.AutomaticManagedPagefile, [string]$files.Count)
  Emit 'PFOK' @('1')
} catch { Emit 'PFERR' @($_.Exception.Message) }
"#;
    let output = match run_powershell(&script(BODY), Duration::from_secs(30)) {
        Ok(output) => output,
        Err(error) => return failed(id, error),
    };
    let errors = report_errors(&output.stdout, "PFERR");
    if !report_completed(&output.stdout, "PFOK") {
        let detail = if errors.is_empty() {
            output.failure_detail()
        } else {
            errors.join("; ")
        };
        return failed(
            id,
            format!("Could not read the page file settings: {detail}"),
        );
    }
    let lines: Vec<(String, Vec<String>)> = output
        .stdout
        .lines()
        .filter_map(decode_line)
        .map(|(key, fields)| (key.to_owned(), fields))
        .collect();
    let automatic = lines
        .iter()
        .find(|(key, _)| key == "PFSYS")
        .map(|(_, fields)| field(fields, 0).eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    let mut details: Vec<String> = lines
        .iter()
        .filter(|(key, _)| key == "PF")
        .map(|(_, fields)| {
            format!(
                "{} · {} MB allocated · peak use {} MB",
                truncate(field(fields, 0), 80),
                field(fields, 1),
                field(fields, 3)
            )
        })
        .collect();
    let count = details.len();
    if automatic {
        details.push("Windows is managing the size itself.".to_owned());
    }
    details.extend(errors);

    let (status, summary) = if count == 0 && !automatic {
        (
            CheckStatus::Warning,
            "No page file is configured. A memory spike has nowhere to go.".to_owned(),
        )
    } else if count == 0 {
        (
            CheckStatus::Pass,
            "Windows is managing the page file itself.".to_owned(),
        )
    } else {
        (
            CheckStatus::Pass,
            format!("{} configured.", plural(count, "page file")),
        )
    };
    CheckOutcome {
        id,
        status,
        summary,
        details,
    }
}

/// How long since an update actually installed.
///
/// Read from the installed-update history rather than from the update service,
/// because the question is whether updates are landing, not whether the
/// service says it is willing to look. `Get-HotFix` misses the updates that
/// arrive as a component servicing operation, so a long gap is reported as
/// something to look at rather than as a failure.
#[cfg(windows)]
fn check_windows_update() -> CheckOutcome {
    let id = CheckId::WindowsUpdate;
    const BODY: &str = r#"
try {
  $hotfixes = @(Get-HotFix -ErrorAction Stop | Where-Object { $_.InstalledOn } | Sort-Object InstalledOn -Descending)
  foreach ($fix in ($hotfixes | Select-Object -First 5)) {
    Emit 'FIX' @($fix.HotFixID, $fix.Description, $fix.InstalledOn.ToString('yyyy-MM-dd'), [string]([math]::Floor(((Get-Date) - $fix.InstalledOn).TotalDays)))
  }
  Emit 'FIXOK' @([string]$hotfixes.Count)
} catch { Emit 'FIXERR' @($_.Exception.Message) }
"#;
    let output = match run_powershell(&script(BODY), Duration::from_secs(60)) {
        Ok(output) => output,
        Err(error) => return failed(id, error),
    };
    let errors = report_errors(&output.stdout, "FIXERR");
    if !report_completed(&output.stdout, "FIXOK") {
        let detail = if errors.is_empty() {
            output.failure_detail()
        } else {
            errors.join("; ")
        };
        return failed(id, format!("Could not read the update history: {detail}"));
    }
    let fixes: Vec<Vec<String>> = output
        .stdout
        .lines()
        .filter_map(decode_line)
        .filter(|(key, _)| *key == "FIX")
        .map(|(_, fields)| fields)
        .collect();
    let mut details: Vec<String> = fixes
        .iter()
        .map(|fields| {
            format!(
                "{} · {} · installed {}",
                truncate(field(fields, 0), 30),
                truncate(field(fields, 1), 40),
                field(fields, 2)
            )
        })
        .collect();
    details.extend(errors);

    let Some(age) = fixes
        .first()
        .and_then(|fields| field(fields, 3).parse::<i64>().ok())
    else {
        return CheckOutcome {
            id,
            status: CheckStatus::Skipped,
            summary: "Windows reported no dated update history to read.".to_owned(),
            details,
        };
    };

    // Ninety days, not thirty. Monthly updates are the rhythm, but a machine
    // that was off for a holiday is not a finding, and a check that warns
    // every fifth week teaches the user to ignore it.
    let (status, summary) = if age > 180 {
        (
            CheckStatus::Warning,
            format!("The last update installed {age} days ago. Updates are probably failing."),
        )
    } else if age > 90 {
        (
            CheckStatus::Warning,
            format!("The last update installed {age} days ago."),
        )
    } else {
        (
            CheckStatus::Pass,
            format!(
                "An update installed {} ago.",
                plural(age.max(0) as usize, "day")
            ),
        )
    };
    CheckOutcome {
        id,
        status,
        summary,
        details,
    }
}

/// Stop errors in the last 30 days, from the dump folder and the event.
///
/// Both sources, because either can be empty on a machine that is definitely
/// crashing: dump writing can be turned off, and the bugcheck event is missed
/// if the machine lost power rather than stopping. Agreement is not required
/// -- whichever found something is reported.
#[cfg(windows)]
fn check_crash_dumps() -> CheckOutcome {
    let id = CheckId::CrashDumps;
    const BODY: &str = r#"
try {
  $since = (Get-Date).AddDays(-30)
  $dumps = @()
  $folder = Join-Path $env:SystemRoot 'Minidump'
  if (Test-Path $folder) {
    try {
      $dumps = @(Get-ChildItem -Path $folder -Filter '*.dmp' -ErrorAction Stop | Where-Object { $_.LastWriteTime -ge $since } | Sort-Object LastWriteTime -Descending)
    } catch { Emit 'DUMPDENIED' @($_.Exception.Message) }
  }
  foreach ($dump in ($dumps | Select-Object -First 10)) {
    Emit 'DUMP' @($dump.Name, $dump.LastWriteTime.ToString('yyyy-MM-dd HH:mm'), [string]([math]::Floor(((Get-Date) - $dump.LastWriteTime).TotalDays)))
  }
  $events = @()
  try {
    $events = @(Get-WinEvent -FilterHashtable @{ LogName = 'System'; Id = 1001; StartTime = $since } -ErrorAction SilentlyContinue | Where-Object { $_.ProviderName -like '*BugCheck*' })
  } catch { }
  foreach ($event in ($events | Select-Object -First 10)) {
    Emit 'BUG' @($event.TimeCreated.ToString('yyyy-MM-dd HH:mm'), [string]$event.Message)
  }
  Emit 'CRASHOK' @([string]$dumps.Count, [string]$events.Count)
} catch { Emit 'CRASHERR' @($_.Exception.Message) }
"#;
    let output = match run_powershell(&script(BODY), Duration::from_secs(60)) {
        Ok(output) => output,
        Err(error) => return failed(id, error),
    };
    let errors = report_errors(&output.stdout, "CRASHERR");
    if !report_completed(&output.stdout, "CRASHOK") {
        let detail = if errors.is_empty() {
            output.failure_detail()
        } else {
            errors.join("; ")
        };
        return failed(id, format!("Could not read the crash history: {detail}"));
    }
    let decoded: Vec<(String, Vec<String>)> = output
        .stdout
        .lines()
        .filter_map(decode_line)
        .map(|(key, fields)| (key.to_owned(), fields))
        .collect();
    let dumps: Vec<&Vec<String>> = decoded
        .iter()
        .filter(|(key, _)| key == "DUMP")
        .map(|(_, fields)| fields)
        .collect();
    let bugchecks: Vec<&Vec<String>> = decoded
        .iter()
        .filter(|(key, _)| key == "BUG")
        .map(|(_, fields)| fields)
        .collect();

    let mut details: Vec<String> = dumps
        .iter()
        .map(|fields| {
            format!(
                "Crash dump {} · {}",
                truncate(field(fields, 0), 40),
                field(fields, 1)
            )
        })
        .collect();
    details.extend(bugchecks.iter().map(|fields| {
        format!(
            "Stop error logged {} · {}",
            field(fields, 0),
            truncate(field(fields, 1), 160)
        )
    }));
    details.extend(errors);

    let recent = dumps
        .iter()
        .filter_map(|fields| field(fields, 2).parse::<i64>().ok())
        .min();
    let count = dumps.len().max(bugchecks.len());
    // Silence from a folder we were not allowed to open is not the same as
    // silence from an empty one, and reporting "no crashes" on the strength
    // of a permission error is the kind of false all-clear this page exists
    // to avoid. The bugcheck events are still worth whatever they found.
    let denied = decoded.iter().any(|(key, _)| key == "DUMPDENIED");
    let (status, summary) = if count == 0 && denied {
        (
            CheckStatus::NeedsAdmin,
            "The crash dump folder could not be read; only the event log was checked.".to_owned(),
        )
    } else if count == 0 {
        (
            CheckStatus::Pass,
            "No stop error was recorded in the last 30 days.".to_owned(),
        )
    } else if recent.is_some_and(|days| days <= 7) {
        (
            CheckStatus::Problem,
            format!(
                "{} in the last 30 days, the most recent within a week.",
                plural(count, "stop error")
            ),
        )
    } else {
        (
            CheckStatus::Warning,
            format!("{} in the last 30 days.", plural(count, "stop error")),
        )
    };
    CheckOutcome {
        id,
        status,
        summary,
        details,
    }
}

/// Which drives Windows Search crawls.
///
/// This is the read-only half of a real complaint: a spinning drive kept in
/// the search index is woken by the crawler on its own schedule, so a machine
/// that should be quiet clatters, and a drive the user touches once a month
/// never gets to rest. On an SSD the same setting costs nothing noticeable,
/// which is why the verdict turns on the media type rather than on the
/// setting alone -- warning about an indexed SSD would be noise.
///
/// The scope comes from the crawl-scope registry rules rather than from the
/// search WMI provider: `root\Microsoft\Windows\Search` does not exist on
/// every edition -- the UAT machine answers "invalid namespace" -- while the
/// rules under `CrawlScopeManager` are there whenever the feature is.
///
/// Nothing here changes a scope. Removing a drive from the index is a real
/// change with a real cost (that drive stops appearing in Start menu results),
/// so this names the drive and the setting, and the user decides.
#[cfg(windows)]
fn check_search_index_scope() -> CheckOutcome {
    let id = CheckId::SearchIndexScope;
    const BODY: &str = r#"
try {
  $system = ($env:SystemDrive).TrimEnd(':')
  $service = 'Unknown'
  $found = Get-Service WSearch -ErrorAction SilentlyContinue
  if ($found) { $service = [string]$found.Status }
  $media = @{}
  try {
    foreach ($disk in @(Get-PhysicalDisk -ErrorAction Stop)) {
      foreach ($part in @(Get-Partition -DiskNumber $disk.DeviceId -ErrorAction SilentlyContinue)) {
        if ($part.DriveLetter) { $media[[string]$part.DriveLetter] = [string]$disk.MediaType }
      }
    }
  } catch { }
  $counts = @{}
  foreach ($set in @('WorkingSetRules', 'DefaultRules')) {
    $base = 'HKLM:\SOFTWARE\Microsoft\Windows Search\CrawlScopeManager\Windows\SystemIndex\' + $set
    foreach ($rule in @(Get-ChildItem $base -ErrorAction SilentlyContinue)) {
      $values = Get-ItemProperty $rule.PSPath -ErrorAction SilentlyContinue
      if (-not $values -or $values.Include -ne 1) { continue }
      $match = [regex]::Match([string]$values.URL, '^file:///([A-Za-z]):')
      if (-not $match.Success) { continue }
      $letter = $match.Groups[1].Value.ToUpper()
      if ($counts.ContainsKey($letter)) { $counts[$letter] = $counts[$letter] + 1 }
      else { $counts[$letter] = 1 }
    }
  }
  foreach ($letter in $counts.Keys) {
    $kind = ''
    if ($media.ContainsKey($letter)) { $kind = $media[$letter] }
    Emit 'IDX' @($letter, $kind, [string]$counts[$letter], [string]($letter -eq $system.ToUpper()))
  }
  Emit 'IDXOK' @($service)
} catch { Emit 'IDXERR' @($_.Exception.Message) }
"#;
    let output = match run_powershell(&script(BODY), Duration::from_secs(60)) {
        Ok(output) => output,
        Err(error) => return failed(id, error),
    };
    let errors = report_errors(&output.stdout, "IDXERR");
    if !report_completed(&output.stdout, "IDXOK") {
        let detail = if errors.is_empty() {
            output.failure_detail()
        } else {
            errors.join("; ")
        };
        return failed(
            id,
            format!("Could not read the search index scope: {detail}"),
        );
    }
    let decoded: Vec<(String, Vec<String>)> = output
        .stdout
        .lines()
        .filter_map(decode_line)
        .map(|(key, fields)| (key.to_owned(), fields))
        .collect();
    let service = decoded
        .iter()
        .find(|(key, _)| key == "IDXOK")
        .map(|(_, fields)| field(fields, 0).to_owned())
        .unwrap_or_default();

    let mut details = Vec::new();
    let mut spinning = Vec::new();
    let mut other_drives = 0usize;
    for (_, fields) in decoded.iter().filter(|(key, _)| key == "IDX") {
        let letter = field(fields, 0);
        let media = field(fields, 1).to_ascii_uppercase();
        let rules = field(fields, 2);
        let is_system = field(fields, 3).eq_ignore_ascii_case("true");
        let kind = match media.as_str() {
            "HDD" => "spinning disk",
            "SSD" => "SSD",
            _ => "unknown drive type",
        };
        details.push(format!(
            "{letter}: · {kind}{} · {} indexed",
            if is_system { " · Windows drive" } else { "" },
            plural(rules.parse().unwrap_or(0), "location")
        ));
        if is_system {
            continue;
        }
        other_drives += 1;
        if media == "HDD" {
            spinning.push(letter.to_owned());
        }
    }
    // Indexing the Windows drive is how search is meant to work and is not
    // reported either way: the finding is only ever about the drives that did
    // not have to be in there.
    details.push(format!("Windows Search service: {service}."));
    details.extend(errors);

    let running = service.eq_ignore_ascii_case("Running");
    let (status, summary) = if !running {
        (
            CheckStatus::Pass,
            format!("Windows Search is not running ({service}); nothing is being crawled."),
        )
    } else if !spinning.is_empty() {
        (
            CheckStatus::Warning,
            format!(
                "{} is a spinning drive and is in the search index, so the crawler wakes it on its own schedule.",
                spinning.join(", ")
            ),
        )
    } else if other_drives > 0 {
        (
            CheckStatus::Pass,
            format!(
                "{} beyond Windows are indexed, none of them spinning.",
                plural(other_drives, "drive")
            ),
        )
    } else {
        (
            CheckStatus::Pass,
            "Only the Windows drive is indexed.".to_owned(),
        )
    };
    CheckOutcome {
        id,
        status,
        summary,
        details,
    }
}

/// Memory pressure: how close the machine is to its commit limit, and whether
/// it is reading pages back off the disk to stay there.
///
/// This is the gap the other checks left. A machine with 8 GB and a browser
/// open has a quiet processor, a healthy drive, no device errors and nothing
/// in the event log, and is still unpleasant to use, because every switch
/// between windows is a trip to the page file. Nothing else here would see it.
///
/// Read through `Win32_PerfFormattedData_PerfOS_Memory` rather than
/// `Get-Counter`. Counter *paths* are localised -- `\Memory\Available MBytes`
/// does not resolve on a Windows installed in Italian, which is the machine
/// this was written on -- while the CIM class name is the same everywhere.
#[cfg(windows)]
fn check_memory_pressure() -> CheckOutcome {
    let id = CheckId::MemoryPressure;
    // Three samples rather than one. Hard faults are spiky by nature and a
    // single instant catches either a burst or a lull; a second and a half of
    // them is still cheap and is at least an average of something.
    const BODY: &str = r#"
try {
  $available = @(); $committed = @(); $faults = @()
  foreach ($pass in 1..3) {
    $memory = Get-CimInstance -ClassName Win32_PerfFormattedData_PerfOS_Memory -ErrorAction Stop
    $available += [double]$memory.AvailableMBytes
    $committed += [double]$memory.PercentCommittedBytesInUse
    $faults += [double]$memory.PagesInputPerSec
    if ($pass -lt 3) { Start-Sleep -Milliseconds 500 }
  }
  Emit 'MEM' @(
    [string]([math]::Round(($available | Measure-Object -Average).Average)),
    [string]([math]::Round(($committed | Measure-Object -Average).Average)),
    [string]([math]::Round(($faults | Measure-Object -Average).Average))
  )
  Emit 'MEMOK' @('1')
} catch { Emit 'MEMERR' @($_.Exception.Message) }
"#;
    let output = match run_powershell(&script(BODY), Duration::from_secs(45)) {
        Ok(output) => output,
        Err(error) => return failed(id, error),
    };
    let errors = report_errors(&output.stdout, "MEMERR");
    if !report_completed(&output.stdout, "MEMOK") {
        let detail = if errors.is_empty() {
            output.failure_detail()
        } else {
            errors.join("; ")
        };
        return failed(id, format!("Could not read the memory counters: {detail}"));
    }
    let Some(fields) = output
        .stdout
        .lines()
        .filter_map(decode_line)
        .find(|(key, _)| *key == "MEM")
        .map(|(_, fields)| fields)
    else {
        return failed(id, "The memory counters returned nothing.".to_owned());
    };
    let available = field(&fields, 0).parse::<f64>().unwrap_or(-1.0);
    let committed = field(&fields, 1).parse::<f64>().unwrap_or(-1.0);
    let faults = field(&fields, 2).parse::<f64>().unwrap_or(0.0);
    if available < 0.0 || committed < 0.0 {
        return failed(
            id,
            "The memory counters returned no usable figures.".to_owned(),
        );
    }

    let mut details = vec![
        format!("{available:.0} MB available to hand out without paging."),
        format!("{committed:.0}% of the commit limit is in use."),
        format!("{faults:.0} pages per second read back from disk, averaged over 1.5 seconds."),
        "Taken at the moment of the scan. A machine that is only slow while a particular program \
         runs will read normally here if that program is closed."
            .to_owned(),
    ];
    details.extend(errors);

    // 90% committed is where Windows starts refusing allocations outright and
    // 512 MB available is where it is already paging to keep going. The
    // warning band below each is deliberately wide: this check is a pointer
    // at a cause, not a verdict, and a near-miss is worth saying.
    let (status, summary) = if committed >= 95.0 || available < 512.0 {
        (
            CheckStatus::Problem,
            format!(
                "Memory is nearly exhausted: {committed:.0}% committed, {available:.0} MB free."
            ),
        )
    } else if committed >= 85.0 || available < 1024.0 {
        (
            CheckStatus::Warning,
            format!("Memory is tight: {committed:.0}% committed, {available:.0} MB free."),
        )
    } else if faults >= 200.0 {
        (
            CheckStatus::Warning,
            format!(
                "Memory is not full, but {faults:.0} pages a second are coming back from disk."
            ),
        )
    } else {
        (
            CheckStatus::Pass,
            format!("{available:.0} MB available, {committed:.0}% of the commit limit in use."),
        )
    };
    CheckOutcome {
        id,
        status,
        summary,
        details,
    }
}

/// How old the third-party drivers are on the devices that cause stutter.
///
/// Microsoft's own drivers are excluded, and this is the whole subtlety of the
/// check: every in-box driver carries the date 21 June 2006 regardless of how
/// current it is -- the UAT machine reports a 2006 date against version
/// 10.0.26100, shipped this year. Judging those by date would flag every
/// Windows installation in existence. A vendor driver's date is real.
///
/// Classes are limited to the four subsystems where an old driver is a
/// plausible cause of the symptom rather than a tidiness complaint: network,
/// audio, storage and display.
#[cfg(windows)]
fn check_driver_age() -> CheckOutcome {
    let id = CheckId::DriverAge;
    const BODY: &str = r#"
try {
  $classes = @('NET', 'MEDIA', 'SCSIADAPTER', 'HDC', 'DISPLAY')
  $drivers = @(Get-CimInstance -ClassName Win32_PnPSignedDriver -ErrorAction Stop |
    Where-Object { $classes -contains $_.DeviceClass -and $_.DriverDate -and $_.DriverProviderName -notlike 'Microsoft*' })
  foreach ($driver in ($drivers | Sort-Object DriverDate | Select-Object -First 12)) {
    Emit 'DRV' @(
      $driver.DeviceName,
      $driver.DriverProviderName,
      $driver.DriverDate.ToString('yyyy-MM-dd'),
      [string]([math]::Floor(((Get-Date) - $driver.DriverDate).TotalDays)),
      $driver.DeviceClass
    )
  }
  Emit 'DRVOK' @([string]$drivers.Count)
} catch { Emit 'DRVERR' @($_.Exception.Message) }
"#;
    let output = match run_powershell(&script(BODY), Duration::from_secs(60)) {
        Ok(output) => output,
        Err(error) => return failed(id, error),
    };
    let errors = report_errors(&output.stdout, "DRVERR");
    if !report_completed(&output.stdout, "DRVOK") {
        let detail = if errors.is_empty() {
            output.failure_detail()
        } else {
            errors.join("; ")
        };
        return failed(id, format!("Could not list drivers: {detail}"));
    }
    let drivers: Vec<Vec<String>> = output
        .stdout
        .lines()
        .filter_map(decode_line)
        .filter(|(key, _)| *key == "DRV")
        .map(|(_, fields)| fields)
        .collect();

    // Five years. Vendors do stop shipping updates for working hardware, so
    // this is phrased as something to look at rather than as a fault: the
    // check cannot tell a neglected driver from a finished one.
    const OLD_DAYS: i64 = 1825;
    let mut details: Vec<String> = drivers
        .iter()
        .map(|fields| {
            let age = field(fields, 3).parse::<i64>().unwrap_or(0);
            format!(
                "{} · {} · {} · {}",
                truncate(field(fields, 0), 60),
                truncate(field(fields, 1), 30),
                field(fields, 2),
                human_duration(age.max(0) as f64 * 24.0)
            )
        })
        .collect();
    details.push(
        "Microsoft's own drivers are left out: Windows dates all of them 2006 whatever version \
         they are, so their age says nothing."
            .to_owned(),
    );
    details.extend(errors);

    let old: Vec<&Vec<String>> = drivers
        .iter()
        .filter(|fields| field(fields, 3).parse::<i64>().unwrap_or(0) > OLD_DAYS)
        .collect();
    let (status, summary) = if drivers.is_empty() {
        (
            CheckStatus::Pass,
            "No third-party driver was found on the network, audio, storage or display devices."
                .to_owned(),
        )
    } else if old.is_empty() {
        (
            CheckStatus::Pass,
            format!(
                "Every vendor driver is under five years old ({}).",
                plural(drivers.len(), "driver")
            ),
        )
    } else {
        (
            CheckStatus::Warning,
            format!(
                "{} over five years old, including {}.",
                plural(old.len(), "vendor driver"),
                truncate(field(old[0], 0), 50)
            ),
        )
    };
    CheckOutcome {
        id,
        status,
        summary,
        details,
    }
}

/// The GUID Windows gives the Power saver scheme. Fixed across every edition
/// and every display language, which is the point: the scheme *name* is
/// localised and matching on it fails on exactly the machines this check is
/// most useful on.
pub const POWER_SAVER_GUID: &str = "a1841308-3541-4fab-bc81-f71556f20b4a";

/// Pull the GUID and the display name out of `powercfg /getactivescheme`.
///
/// The line reads, in English, `Power Scheme GUID: <guid>  (Balanced)`. Both
/// halves are localised except the GUID, and an OEM scheme can carry any name
/// at all, so the name is taken as whatever sits in the final parentheses and
/// the GUID is found by shape rather than by position.
pub fn parse_active_scheme(reported: &str) -> (String, String) {
    let guid = reported
        .split(|c: char| !c.is_ascii_hexdigit() && c != '-')
        .find(|token| token.len() == 36 && token.matches('-').count() == 4)
        .unwrap_or_default()
        .to_owned();
    let name = reported
        .rfind('(')
        .and_then(|open| {
            let rest = &reported[open + 1..];
            rest.rfind(')').map(|close| rest[..close].trim().to_owned())
        })
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| {
            if guid.is_empty() {
                reported.trim().to_owned()
            } else {
                format!("Scheme {guid}")
            }
        });
    (guid, name)
}

/// Keep the worst verdict seen so far. Needed because the drive check folds
/// several drives into one row and must not let a healthy second drive erase
/// what the first one said.
#[cfg(windows)]
fn worse(current: CheckStatus, candidate: CheckStatus) -> CheckStatus {
    let rank = |status: CheckStatus| match status {
        CheckStatus::Pass => 0,
        CheckStatus::Skipped => 1,
        CheckStatus::NeedsAdmin => 2,
        CheckStatus::Error => 3,
        CheckStatus::Warning => 4,
        CheckStatus::Problem => 5,
    };
    if rank(candidate) > rank(current) {
        candidate
    } else {
        current
    }
}

#[cfg(windows)]
fn check_volume_dirty_bit() -> CheckOutcome {
    let id = CheckId::VolumeDirtyBit;
    if !crate::util::is_elevated() {
        return needs_admin(
            id,
            "Windows reports the dirty flag only to an administrator.",
        );
    }
    // `fsutil dirty query` reads the flag; it is `fsutil dirty set` that would
    // schedule a chkdsk, and that word appears nowhere in this program.
    // fsutil is invoked from inside PowerShell so one child process covers
    // every volume and one deadline covers the whole check.
    const BODY: &str = r#"
try {
  $volumes = @(Get-CimInstance Win32_LogicalDisk -Filter 'DriveType=3' -ErrorAction Stop)
  foreach ($volume in $volumes) {
    $reply = & "$env:SystemRoot\System32\fsutil.exe" dirty query $volume.DeviceID 2>&1 | Out-String
    Emit 'DIRTY' @($volume.DeviceID, $reply)
  }
  Emit 'DIRTYOK' @([string]$volumes.Count)
} catch { Emit 'DIRTYERR' @($_.Exception.Message) }
"#;
    let output = match run_powershell(&script(BODY), Duration::from_secs(45)) {
        Ok(output) => output,
        Err(error) => return failed(id, error),
    };
    let errors = report_errors(&output.stdout, "DIRTYERR");
    if !report_completed(&output.stdout, "DIRTYOK") {
        let detail = if errors.is_empty() {
            output.failure_detail()
        } else {
            errors.join("; ")
        };
        return failed(id, format!("Could not query the dirty flag: {detail}"));
    }
    let volumes = parse_dirty_report(&output.stdout);
    if volumes.is_empty() {
        return CheckOutcome {
            id,
            status: CheckStatus::Skipped,
            summary: "No fixed volume was found to query.".to_owned(),
            details: errors,
        };
    }
    let mut status = CheckStatus::Pass;
    let mut dirty = Vec::new();
    let mut details: Vec<String> = Vec::new();
    for (volume, verdict) in &volumes {
        let word = match verdict {
            DirtyVerdict::Clean => "not dirty",
            DirtyVerdict::Dirty => "DIRTY",
            DirtyVerdict::Denied => "access denied",
            DirtyVerdict::Unknown => "unrecognised reply",
        };
        details.push(format!("{volume} · {word}"));
        match verdict {
            DirtyVerdict::Clean => {}
            DirtyVerdict::Dirty => {
                dirty.push(volume.clone());
                status = worse(status, CheckStatus::Problem);
            }
            DirtyVerdict::Denied => status = worse(status, CheckStatus::NeedsAdmin),
            DirtyVerdict::Unknown => status = worse(status, CheckStatus::Error),
        }
    }
    details.extend(errors);
    let summary = if !dirty.is_empty() {
        format!(
            "{} is marked dirty; Windows wants to check it at the next restart.",
            dirty.join(", ")
        )
    } else if status == CheckStatus::Pass {
        format!("{} clean.", plural(volumes.len(), "fixed volume"))
    } else {
        "The dirty flag could not be read for every volume.".to_owned()
    };
    CheckOutcome {
        id,
        status,
        summary,
        details,
    }
}

#[cfg(windows)]
fn check_system_files() -> CheckOutcome {
    let id = CheckId::SystemFiles;
    if !crate::util::is_elevated() {
        return needs_admin(
            id,
            "DISM reports component store health only to an administrator.",
        );
    }
    let dism = std::env::var_os("SystemRoot")
        .map(|root| std::path::PathBuf::from(root).join("System32\\Dism.exe"))
        .unwrap_or_else(|| std::path::PathBuf::from("Dism.exe"));
    // /CheckHealth reads the flag the last servicing operation left behind and
    // returns in seconds. /ScanHealth walks the whole store and /RestoreHealth
    // rewrites it; neither belongs in something labelled a scan.
    let output = match run_bounded(
        &dism,
        &["/Online", "/Cleanup-Image", "/CheckHealth"],
        Duration::from_secs(120),
    ) {
        Ok(output) => output,
        Err(error) => return failed(id, error),
    };
    let text = format!("{}\n{}", output.stdout, output.stderr);
    if looks_like_access_denied(&text) {
        return needs_admin(
            id,
            "DISM refused the query: it needs a full administrator token.",
        );
    }
    let mut details = vec![format!(
        "Ran Dism.exe /Online /Cleanup-Image /CheckHealth ({})",
        match output.code {
            Some(code) => format!("exit code {code}"),
            None => "no exit code".to_owned(),
        }
    )];
    details.extend(dism_report_lines(&text));
    match parse_dism_check_health(&text) {
        DismVerdict::Clean => CheckOutcome {
            id,
            status: CheckStatus::Pass,
            summary: "No component store corruption is reported.".to_owned(),
            details,
        },
        DismVerdict::Repairable => CheckOutcome {
            id,
            status: CheckStatus::Problem,
            summary: "Component store corruption was detected and DISM says it is repairable."
                .to_owned(),
            details,
        },
        DismVerdict::NotRepairable => CheckOutcome {
            id,
            status: CheckStatus::Problem,
            summary: "Component store corruption was detected and is not repairable in place."
                .to_owned(),
            details,
        },
        DismVerdict::Unknown => CheckOutcome {
            id,
            status: CheckStatus::Error,
            summary: "DISM did not print a verdict this code recognises.".to_owned(),
            details,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a `KEY<tab>base64...` line the way the PowerShell prelude does,
    /// so the sample fixtures below stay readable.
    fn line(key: &str, values: &[&str]) -> String {
        const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let encode = |value: &str| {
            let bytes = value.as_bytes();
            let mut out = String::new();
            for chunk in bytes.chunks(3) {
                let b0 = chunk.first().copied().unwrap_or(0) as u32;
                let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
                let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
                let packed = (b0 << 16) | (b1 << 8) | b2;
                out.push(ALPHABET[(packed >> 18) as usize & 63] as char);
                out.push(ALPHABET[(packed >> 12) as usize & 63] as char);
                out.push(if chunk.len() > 1 {
                    ALPHABET[(packed >> 6) as usize & 63] as char
                } else {
                    '='
                });
                out.push(if chunk.len() > 2 {
                    ALPHABET[packed as usize & 63] as char
                } else {
                    '='
                });
            }
            out
        };
        let encoded: Vec<String> = values.iter().map(|value| encode(*value)).collect();
        format!("{key}\t{}", encoded.join("\t"))
    }

    #[test]
    fn truncation_stops_on_a_character_boundary() {
        assert_eq!(truncate("short", 10), "short");
        assert_eq!(truncate("abcdef", 3), "abc\u{2026}");
        // The em dash is three bytes; byte slicing here is what used to panic.
        assert_eq!(truncate("a\u{2014}bcd", 2), "a\u{2014}\u{2026}");
    }

    #[test]
    fn decodes_a_field_containing_tabs_and_newlines() {
        let raw = line("DIRTY", &["C:", "Volume - C: is NOT Dirty\r\n"]);
        let (key, fields) = decode_line(&raw).expect("line should decode");
        assert_eq!(key, "DIRTY");
        assert_eq!(fields.len(), 2);
        assert_eq!(fields.first().map(String::as_str), Some("C:"));
        assert_eq!(
            fields.get(1).map(|reply| parse_dirty_query(reply)),
            Some(DirtyVerdict::Clean)
        );
    }

    #[test]
    fn short_and_malformed_lines_are_ignored_rather_than_panicking() {
        assert!(decode_line("").is_none());
        // A line that lost its fields must read as empty, not index past the
        // end: the reason every field goes through `field` rather than `[]`.
        let disks = parse_physical_disks("DISK\nDISK\nnot a line at all\n");
        assert_eq!(disks.len(), 2);
        assert!(disks.iter().all(|disk| disk.name.is_empty()));
        assert!(disks.iter().all(|disk| disk.wear_percent.is_none()));
        let devices = parse_device_problems("PNP");
        assert_eq!(
            devices.first().map(|device| device.name.as_str()),
            Some("Unnamed device")
        );
        assert_eq!(
            devices.first().map(|device| device.instance_id.as_str()),
            Some("")
        );
    }

    #[test]
    fn reads_the_dism_verdicts() {
        const CLEAN: &str = "Deployment Image Servicing and Management tool\r\n\
Version: 10.0.26100.1150\r\n\r\nImage Version: 10.0.26100.4061\r\n\r\n\
[==========================100.0%==========================]\r\n\
No component store corruption detected.\r\n\
The operation completed successfully.\r\n";
        const REPAIRABLE: &str = "Image Version: 10.0.26100.4061\r\n\r\n\
The component store is repairable.\r\n\
The operation completed successfully.\r\n";
        const BROKEN: &str = "Error: 1726\r\n\
The component store corruption was detected but it is not repairable.\r\n";
        assert_eq!(parse_dism_check_health(CLEAN), DismVerdict::Clean);
        assert_eq!(parse_dism_check_health(REPAIRABLE), DismVerdict::Repairable);
        assert_eq!(parse_dism_check_health(BROKEN), DismVerdict::NotRepairable);
        assert_eq!(parse_dism_check_health(""), DismVerdict::Unknown);
        // A translated report must not be read as a clean bill of health.
        assert_eq!(
            parse_dism_check_health("Keine Beschaedigung des Komponentenspeichers."),
            DismVerdict::Unknown
        );
    }

    #[test]
    fn dism_report_drops_the_banner_and_the_progress_bar() {
        const CLEAN: &str = "Deployment Image Servicing and Management tool\r\n\
Version: 10.0.26100.1150\r\n\r\nImage Version: 10.0.26100.4061\r\n\r\n\
[==========================100.0%==========================]\r\n\
No component store corruption detected.\r\n";
        assert_eq!(
            dism_report_lines(CLEAN),
            vec!["No component store corruption detected.".to_owned()]
        );
    }

    #[test]
    fn reads_the_fsutil_replies() {
        assert_eq!(
            parse_dirty_query("Volume - C: is NOT Dirty"),
            DirtyVerdict::Clean
        );
        assert_eq!(
            parse_dirty_query("Volume - D: is Dirty"),
            DirtyVerdict::Dirty
        );
        assert_eq!(parse_dirty_query("Access is denied."), DirtyVerdict::Denied);
        assert_eq!(
            parse_dirty_query("Le volume n'est pas sale"),
            DirtyVerdict::Unknown
        );
    }

    #[test]
    fn parses_physical_disks_with_and_without_reliability_counters() {
        let text = format!(
            "{}\n{}\n{}\n",
            line(
                "DISK",
                &[
                    "Samsung SSD 990 PRO 2TB",
                    "SSD",
                    "Healthy",
                    "OK",
                    "3",
                    "41",
                    "4210",
                    "0"
                ]
            ),
            line(
                "DISK",
                &["ST4000DM004-2CV104", "HDD", "Warning", "OK", "", "", "", ""]
            ),
            line("DISKOK", &["2"]),
        );
        let disks = parse_physical_disks(&text);
        assert_eq!(disks.len(), 2);
        let first = disks.first().expect("first disk");
        assert_eq!(first.name, "Samsung SSD 990 PRO 2TB");
        assert_eq!(first.media_type, "SSD");
        assert_eq!(first.wear_percent, Some(3));
        assert_eq!(first.temperature_celsius, Some(41));
        assert_eq!(first.power_on_hours, Some(4210));
        assert_eq!(first.uncorrected_read_errors, Some(0));
        let second = disks.get(1).expect("second disk");
        assert_eq!(second.health, "Warning");
        assert_eq!(second.wear_percent, None);
        assert!(report_completed(&text, "DISKOK"));
    }

    #[test]
    fn parses_device_problems_and_names_the_nameless() {
        let text = format!(
            "{}\n{}\n{}\n",
            line(
                "PNP",
                &[
                    "Base System Device",
                    "Error",
                    "Unknown",
                    "PCI\\VEN_10EC&DEV_522A&SUBSYS_86301043&REV_01\\4&1A2B3C4D&0&00E4"
                ]
            ),
            line("PNP", &["", "Error", "Unknown", "ROOT\\SYSTEM\\0001"]),
            line("PNPOK", &["2"]),
        );
        let devices = parse_device_problems(&text);
        assert_eq!(devices.len(), 2);
        assert_eq!(
            devices.first().map(|device| device.name.as_str()),
            Some("Base System Device")
        );
        assert_eq!(
            devices.get(1).map(|device| device.name.as_str()),
            Some("Unnamed device")
        );
        assert_eq!(
            devices.get(1).map(|device| device.class.as_str()),
            Some("Unknown")
        );
        assert!(report_completed(&text, "PNPOK"));
    }

    #[test]
    fn parses_defender_status() {
        let text = line(
            "MP",
            &["True", "True", "True", "True", "1", "2026-10-16 03:11:08"],
        );
        let status = parse_defender_status(&text).expect("status should parse");
        assert_eq!(status.antivirus_enabled, Some(true));
        assert_eq!(status.realtime_enabled, Some(true));
        assert_eq!(status.antispyware_enabled, Some(true));
        assert_eq!(status.service_enabled, Some(true));
        assert_eq!(status.signature_age_days, Some(1));
        assert_eq!(status.signature_updated, "2026-10-16 03:11:08");

        let off = line("MP", &["True", "False", "True", "True", "", ""]);
        let status = parse_defender_status(&off).expect("status should parse");
        assert_eq!(status.realtime_enabled, Some(false));
        assert_eq!(status.signature_age_days, None);

        assert_eq!(parse_defender_status("MPERR\tAAAA"), None);
    }

    #[test]
    fn spots_a_failed_memory_diagnostic() {
        let passed = line(
            "MEMDIAG",
            &[
                "1201",
                "Information",
                "2026-02-02 08:14:00",
                "The Windows Memory Diagnostic tested memory and detected no errors.",
            ],
        );
        let failed = line(
            "MEMDIAG",
            &[
                "1202",
                "Error",
                "2026-03-09 21:40:12",
                "The Windows Memory Diagnostic detected hardware problems.",
            ],
        );
        assert!(!memory_diagnostic_failed(&parse_memory_diagnostics(
            &passed
        )));
        assert!(memory_diagnostic_failed(&parse_memory_diagnostics(&failed)));
        let events = parse_memory_diagnostics(&failed);
        assert_eq!(events.first().map(|event| event.event_id), Some(1202));
        assert_eq!(
            events.first().map(|event| event.time.as_str()),
            Some("2026-03-09 21:40:12")
        );
        assert!(
            events
                .first()
                .is_some_and(|event| event.message.contains("hardware problems"))
        );
    }

    #[test]
    fn parses_the_boot_report() {
        let text = line("BOOT", &["2026-10-15 07:22:51", "50.37"]);
        let (boot, hours) = parse_boot_report(&text).expect("boot should parse");
        assert_eq!(boot, "2026-10-15 07:22:51");
        assert!((hours - 50.37).abs() < 0.001);
        // A row with the hours missing is not half an answer.
        assert!(parse_boot_report(&line("BOOT", &["2026-10-15 07:22:51"])).is_none());
        assert!(parse_boot_report("").is_none());
    }

    #[test]
    fn recognises_a_denial_but_not_an_ordinary_failure() {
        assert!(looks_like_access_denied("Access is denied."));
        assert!(looks_like_access_denied(
            "Get-MpComputerStatus : UnauthorizedAccessException"
        ));
        assert!(!looks_like_access_denied("The RPC server is unavailable."));
    }

    #[test]
    fn report_errors_survives_a_field_free_line() {
        assert_eq!(
            report_errors("PNPERR", "PNPERR"),
            vec!["Unknown error".to_owned()]
        );
        assert_eq!(
            report_errors(
                &line("PNPERR", &["The service cannot be started."]),
                "PNPERR"
            ),
            vec!["The service cannot be started.".to_owned()]
        );
    }

    #[test]
    fn event_summary_follows_the_health_bands() {
        let finding = |id: u32| EventFinding {
            source: "System / disk / Error".to_owned(),
            event_id: id,
            time: "2026-10-17 09:00:00".to_owned(),
            message: "The device has a bad block.".to_owned(),
        };
        let clean = ScanResult {
            status: ScanStatus::Complete,
            findings: Vec::new(),
            log_errors: Vec::new(),
        };
        assert_eq!(summarise_events(&clean).status, CheckStatus::Pass);

        let few = ScanResult {
            status: ScanStatus::Complete,
            findings: (0..3).map(finding).collect(),
            log_errors: Vec::new(),
        };
        assert_eq!(summarise_events(&few).status, CheckStatus::Warning);

        let many = ScanResult {
            status: ScanStatus::Complete,
            findings: (0..8).map(finding).collect(),
            log_errors: Vec::new(),
        };
        let outcome = summarise_events(&many);
        assert_eq!(outcome.status, CheckStatus::Problem);
        // Only the first eight events are listed; the full list has its own panel.
        assert_eq!(outcome.details.len(), 8);

        let broken = ScanResult {
            status: ScanStatus::Error("PowerShell is missing.".to_owned()),
            findings: Vec::new(),
            log_errors: Vec::new(),
        };
        assert_eq!(summarise_events(&broken).status, CheckStatus::Error);
    }

    #[test]
    fn reads_the_active_power_scheme_in_any_language() {
        // Captured from a real Italian session during the UAT: the name is
        // translated, the GUID is not, and an OEM scheme carries neither of
        // the names this check used to look for.
        let (guid, name) = parse_active_scheme(
            "GUID combinazione risparmio energia: c99ca36e-be1c-4069-93bb-b53c78210ca2  (Prestazioni eccellenti)",
        );
        assert_eq!(guid, "c99ca36e-be1c-4069-93bb-b53c78210ca2");
        assert_eq!(name, "Prestazioni eccellenti");

        let (guid, name) = parse_active_scheme(
            "Power Scheme GUID: a1841308-3541-4fab-bc81-f71556f20b4a  (Power saver)",
        );
        assert_eq!(guid, POWER_SAVER_GUID);
        assert_eq!(name, "Power saver");

        // Nothing recognisable must not panic and must not claim a GUID.
        let (guid, name) = parse_active_scheme("powercfg said something else entirely");
        assert!(guid.is_empty());
        assert_eq!(name, "powercfg said something else entirely");
    }

    #[test]
    fn statuses_map_onto_the_health_vocabulary() {
        assert!(CheckStatus::Problem.is_finding());
        assert!(CheckStatus::Warning.is_finding());
        assert!(CheckStatus::NeedsAdmin.is_finding());
        // A pass is not a finding, and neither is a check that did not run or
        // could not finish -- those are reported, and coloured, but there is
        // nothing in them for the user to act on.
        assert!(!CheckStatus::Pass.is_finding());
        assert!(!CheckStatus::Skipped.is_finding());
        assert!(!CheckStatus::Error.is_finding());
        // The count is asserted so that adding a variant without adding it to
        // `ALL` -- which would silently stop running that check -- fails here
        // rather than in the field.
        assert_eq!(CheckId::ALL.len(), 18);
        for id in CheckId::ALL {
            assert!(!id.label().is_empty());
            assert!(!id.explanation().is_empty());
        }
    }
}
