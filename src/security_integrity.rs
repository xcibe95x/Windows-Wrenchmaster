//! Read-only integrity inventory for the Windows shell executable.
//!
//! Authenticode status and process metadata are collected from Windows. This
//! module deliberately reports duplicate Explorer processes as inventory only:
//! multiple valid Explorer instances can be normal. It does not download files,
//! execute scanned files, or make remediation decisions.

#[cfg(windows)]
use std::io::Read;
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub enum ScanStatus {
    Complete,
    Partial,
    #[allow(dead_code)]
    Unsupported,
    Error(String),
}

#[derive(Clone, Debug)]
pub struct ExplorerProcess {
    pub pid: u32,
    pub parent_pid: u32,
    pub session_id: u32,
    pub image_path: String,
    pub signature_status: String,
    pub signer: String,
}

#[derive(Clone, Debug)]
pub struct IntegrityResult {
    pub status: ScanStatus,
    pub system_explorer_path: String,
    pub system_signature_status: String,
    pub system_signer: String,
    pub processes: Vec<ExplorerProcess>,
    pub notes: Vec<String>,
}

impl IntegrityResult {
    fn failed(status: ScanStatus) -> Self {
        Self {
            status,
            system_explorer_path: String::new(),
            system_signature_status: String::new(),
            system_signer: String::new(),
            processes: Vec::new(),
            notes: Vec::new(),
        }
    }
}

/// Run the bounded, read-only Explorer signature and process inventory scan
/// on a worker thread. The returned receiver can be polled by an async UI.
pub fn start_scan() -> std::sync::mpsc::Receiver<IntegrityResult> {
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(scan());
    });
    receiver
}

/// Collect Authenticode status for `%SystemRoot%\\explorer.exe` and all
/// currently visible `explorer.exe` processes.
pub fn scan() -> IntegrityResult {
    #[cfg(not(windows))]
    {
        return IntegrityResult::failed(ScanStatus::Unsupported);
    }

    #[cfg(windows)]
    scan_windows()
}

/// Launch Windows Resource Protection's verify-only check for the system
/// Explorer binary in a visible console. This does not repair or modify files.
pub fn launch_system_explorer_verify() -> std::io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
        let root = std::env::var_os("SystemRoot")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
        let target = root.join("explorer.exe");
        std::process::Command::new("sfc.exe")
            .arg(format!("/VERIFYFILE={}", target.display()))
            .creation_flags(CREATE_NEW_CONSOLE)
            .spawn()
            .map(|_| ())
    }
    #[cfg(not(windows))]
    {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "Windows Resource Protection is available only on Windows.",
        ))
    }
}

#[cfg(windows)]
fn scan_windows() -> IntegrityResult {
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const TIMEOUT: Duration = Duration::from_secs(25);
    // Paths are obtained inside PowerShell from the process environment and
    // never interpolated into command text. All values cross the output
    // boundary as base64-encoded UTF-8 fields.
    const SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
function Emit([string]$kind, [object[]]$values) {
  $encoded = foreach ($value in $values) {
    [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes([string]$value))
  }
  [Console]::Out.WriteLine($kind + "`t" + ($encoded -join "`t"))
}
function Signature([string]$path) {
  try {
    $sig = Get-AuthenticodeSignature -LiteralPath $path -ErrorAction Stop
    $signer = if ($null -ne $sig.SignerCertificate) { $sig.SignerCertificate.Subject } else { '' }
    return @([string]$sig.Status, [string]$signer)
  } catch { return @('CheckError: ' + $_.Exception.Message, '') }
}
$systemPath = Join-Path $env:SystemRoot 'explorer.exe'
$systemSig = Signature $systemPath
Emit 'SYSTEM' @($systemPath, $systemSig[0], $systemSig[1])
$all = @(Get-CimInstance -ClassName Win32_Process -Filter "Name = 'explorer.exe'" -ErrorAction Stop)
$cache = @{}
foreach ($proc in $all) {
  $path = [string]$proc.ExecutablePath
  if ([string]::IsNullOrWhiteSpace($path)) { $path = '[path unavailable; access may be restricted]' }
  if (-not $cache.ContainsKey($path)) { $cache[$path] = Signature $path }
  $sig = $cache[$path]
  Emit 'PROCESS' @([string]$proc.ProcessId, [string]$proc.ParentProcessId,
    [string]$proc.SessionId, $path, [string]$sig[0], [string]$sig[1])
}
Emit 'DONE' @([string]$all.Count)
"#;

    let powershell = std::env::var_os("SystemRoot")
        .map(|root| PathBuf::from(root).join(r"System32\WindowsPowerShell\v1.0\powershell.exe"))
        .unwrap_or_else(|| PathBuf::from("powershell.exe"));
    let mut child = match Command::new(powershell)
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-WindowStyle",
            "Hidden",
            "-Command",
            SCRIPT,
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            return IntegrityResult::failed(ScanStatus::Error(format!(
                "Could not start the Windows integrity query: {error}"
            )));
        }
    };

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let out_reader = std::thread::spawn(move || read_pipe(stdout));
    let err_reader = std::thread::spawn(move || read_pipe(stderr));
    let started = Instant::now();
    let exit_status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if started.elapsed() < TIMEOUT => {
                std::thread::sleep(Duration::from_millis(100));
            }
            Ok(None) => {
                let _ = child.kill();
                break None;
            }
            Err(error) => {
                let _ = child.kill();
                return IntegrityResult::failed(ScanStatus::Error(format!(
                    "Could not read the integrity query status: {error}"
                )));
            }
        }
    };
    let output = out_reader.join().unwrap_or_default();
    let error_output = err_reader.join().unwrap_or_default();
    let Some(exit_status) = exit_status else {
        return IntegrityResult::failed(ScanStatus::Error(
            "The integrity query timed out after 25 seconds.".into(),
        ));
    };
    if !exit_status.success() {
        let detail = String::from_utf8_lossy(&error_output).trim().to_owned();
        return IntegrityResult::failed(ScanStatus::Error(if detail.is_empty() {
            format!("PowerShell exited with {exit_status}.")
        } else {
            detail
        }));
    }

    let mut result = IntegrityResult {
        status: ScanStatus::Partial,
        system_explorer_path: String::new(),
        system_signature_status: String::new(),
        system_signer: String::new(),
        processes: Vec::new(),
        notes: vec![
            "A valid signature is useful evidence, but does not prove a file is harmless.".into(),
            "More than one Explorer process can be normal; review session, path, and signature together.".into(),
            "SFC verify checks Windows-protected files and may not provide a checksum for every binary.".into(),
        ],
    };
    let mut done = false;
    for line in String::from_utf8_lossy(&output).lines() {
        let mut fields = line.split('\t');
        let kind = fields.next().unwrap_or_default();
        let values: Vec<String> = fields.map(decode_base64_utf8).collect();
        match kind {
            "SYSTEM" if values.len() == 3 => {
                result.system_explorer_path = values[0].clone();
                result.system_signature_status = values[1].clone();
                result.system_signer = values[2].clone();
            }
            "PROCESS" if values.len() == 6 => {
                if let (Ok(pid), Ok(parent_pid), Ok(session_id)) =
                    (values[0].parse(), values[1].parse(), values[2].parse())
                {
                    result.processes.push(ExplorerProcess {
                        pid,
                        parent_pid,
                        session_id,
                        image_path: values[3].clone(),
                        signature_status: values[4].clone(),
                        signer: values[5].clone(),
                    });
                }
            }
            "DONE" => done = true,
            _ => {}
        }
    }
    if result.system_explorer_path.is_empty() {
        result.status = ScanStatus::Error("The query returned no system Explorer result.".into());
    } else if done {
        result.status = ScanStatus::Complete;
    }
    result
}

#[cfg(windows)]
fn read_pipe<R: Read>(pipe: Option<R>) -> Vec<u8> {
    let mut bytes = Vec::new();
    if let Some(mut pipe) = pipe {
        let _ = pipe.read_to_end(&mut bytes);
    }
    bytes
}

#[cfg(windows)]
fn decode_base64_utf8(value: &str) -> String {
    let decoded: Vec<u8> = value
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
                    b'=' => Some(0),
                    _ => None,
                }
            };
            let a = digit(chunk[0])? as u32;
            let b = digit(chunk[1])? as u32;
            let c = digit(chunk[2])? as u32;
            let d = digit(chunk[3])? as u32;
            Some([
                ((a << 2) | (b >> 4)) as u8,
                ((b << 4) | (c >> 2)) as u8,
                ((c << 6) | d) as u8,
            ])
        })
        .flatten()
        .collect();
    String::from_utf8_lossy(&decoded)
        .trim_end_matches('\0')
        .to_owned()
}
