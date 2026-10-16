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
    EventLog,
    DeviceProblems,
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
    pub const ALL: [CheckId; 9] = [
        CheckId::Uptime,
        CheckId::PendingReboot,
        CheckId::EventLog,
        CheckId::DeviceProblems,
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

    /// Map onto the Overview's severity vocabulary so a colour means the same
    /// thing on both pages. `None` is deliberate for the two statuses that are
    /// not findings at all: a check that did not run has nothing to say about
    /// the machine, and painting it green or amber would claim it does.
    pub fn severity(self) -> Option<crate::health::Severity> {
        match self {
            CheckStatus::Pass => Some(crate::health::Severity::Info),
            CheckStatus::Warning | CheckStatus::NeedsAdmin => Some(crate::health::Severity::Warn),
            CheckStatus::Problem => Some(crate::health::Severity::Bad),
            CheckStatus::Skipped | CheckStatus::Error => None,
        }
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
    Finished(CheckOutcome),
    AllDone,
}

/// Run every machine check on one background thread.
///
/// One thread rather than one per check: these mostly shell out to PowerShell,
/// and nine simultaneous `powershell.exe` processes cost more than the scan
/// saves on a machine that is already being diagnosed for being slow.
pub fn start_system_scan() -> std::sync::mpsc::Receiver<CheckMessage> {
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for id in CheckId::ALL {
            // A closed receiver means the window went away. Stop rather than
            // keep spawning child processes nobody will read the output of.
            if sender.send(CheckMessage::Started(id)).is_err() {
                return;
            }
            let outcome = run_check(id, &sender);
            if sender.send(CheckMessage::Finished(outcome)).is_err() {
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
            summary: format!("{count} Critical or Error event(s) in the last 24 hours."),
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
    let details = vec![format!("Last started {boot} ({hours:.1} hours ago)")];
    // The same two bands health.rs uses for uptime, for the same reason the
    // disk thresholds were unified: one judgement, one pair of numbers.
    if days >= 7.0 {
        CheckOutcome {
            id,
            status: CheckStatus::Warning,
            summary: format!(
                "Up for {days:.0} days. A restart would apply anything Windows has staged."
            ),
            details,
        }
    } else {
        CheckOutcome {
            id,
            status: CheckStatus::Pass,
            summary: format!("Up for {days:.1} days."),
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
            summary: format!("{} restart marker(s) are set.", reasons.len()),
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
        summary: format!("{} device(s) are reporting an error.", devices.len()),
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
                "Signatures are {age} day(s) old, last updated {}",
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
            summary: format!(
                "{} past memory diagnostic run(s), none failed.",
                events.len()
            ),
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
            line.push_str(&format!(" · {read_errors} uncorrected read error(s)"));
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
        format!("{} drive(s) report themselves healthy.", disks.len())
    } else if unhealthy == 0 {
        format!(
            "{} drive(s) report healthy, but the wear or error counters are worth a look.",
            disks.len()
        )
    } else {
        format!(
            "{unhealthy} of {} drive(s) are not reporting a healthy state.",
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
        format!("{} fixed volume(s) are clean.", volumes.len())
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
    fn statuses_map_onto_the_health_vocabulary() {
        assert!(CheckStatus::Pass.severity().is_some());
        assert!(CheckStatus::Problem.severity().is_some());
        // Neither of these is a statement about the machine, so neither gets a
        // colour that would read as one.
        assert!(CheckStatus::Skipped.severity().is_none());
        assert!(CheckStatus::Error.severity().is_none());
        assert_eq!(CheckId::ALL.len(), 9);
    }
}
