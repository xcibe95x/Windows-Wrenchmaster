//! Read-only diagnostics from recent Windows Error and Critical event log entries.
//!
//! The query is limited to the System and Application logs, the previous 24
//! hours, and at most 100 entries per log. It does not change system state.

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

/// Start an automatic, bounded event log scan on a background thread.
pub fn start_scan() -> std::sync::mpsc::Receiver<ScanResult> {
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(scan_recent_events());
    });
    receiver
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
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        const LIMIT: Duration = Duration::from_secs(45);
        let mut child = match Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-WindowStyle",
                "Hidden",
                "-Command",
                SCRIPT,
            ])
            .creation_flags(CREATE_NO_WINDOW)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
        {
            Ok(child) => child,
            Err(error) => {
                return ScanResult::failed(ScanStatus::Error(format!(
                    "Could not start PowerShell: {error}"
                )));
            }
        };

        // Drain both pipes concurrently so large event messages cannot block the child.
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let out_reader = std::thread::spawn(move || read_pipe(stdout));
        let err_reader = std::thread::spawn(move || read_pipe(stderr));
        let started = std::time::Instant::now();
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Some(status),
                Ok(None) if started.elapsed() < LIMIT => {
                    std::thread::sleep(Duration::from_millis(100))
                }
                Ok(None) => {
                    let _ = child.kill();
                    break None;
                }
                Err(error) => {
                    let _ = child.kill();
                    return ScanResult::failed(ScanStatus::Error(format!(
                        "Could not read PowerShell status: {error}"
                    )));
                }
            }
        };
        let output = out_reader.join().unwrap_or_default();
        let error_output = err_reader.join().unwrap_or_default();
        let Some(status) = status else {
            return ScanResult::failed(ScanStatus::Error(
                "Event log scan timed out after 45 seconds.".into(),
            ));
        };
        if !status.success() {
            let detail = String::from_utf8_lossy(&error_output).trim().to_string();
            return ScanResult::failed(ScanStatus::Error(if detail.is_empty() {
                format!("PowerShell exited with {status}.")
            } else {
                detail
            }));
        }

        let mut findings = Vec::new();
        let mut log_errors = Vec::new();
        let mut successful_logs = 0;
        for line in String::from_utf8_lossy(&output).lines() {
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

#[cfg(windows)]
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
