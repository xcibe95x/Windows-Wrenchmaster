//! Read-only, metadata-only review of TCP listeners and active connections.
//!
//! This is a small triage aid, not a packet inspector or antivirus. It never
//! captures payloads or sends traffic. Findings are heuristic and need review.

#[cfg(windows)]
use std::io::Read;
use std::{process::Command, sync::mpsc::Receiver, time::Duration};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    IncomingListener,
    IncomingEstablished,
    Outgoing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Info,
    Review,
    Elevated,
}

#[derive(Clone, Debug)]
pub struct Connection {
    pub direction: Direction,
    pub severity: Severity,
    pub local_address: String,
    pub local_port: u16,
    pub remote_address: String,
    pub remote_port: u16,
    pub process_id: u32,
    pub process_name: String,
    pub executable_path: String,
    pub signature: String,
    pub explanation: String,
}

#[derive(Clone, Debug)]
pub enum ScanStatus {
    Complete,
    Partial,
    #[allow(dead_code)]
    Unsupported,
    Error(String),
}

#[derive(Clone, Debug)]
pub struct ScanResult {
    pub status: ScanStatus,
    pub connections: Vec<Connection>,
    pub notes: Vec<String>,
}

impl ScanResult {
    fn empty(status: ScanStatus) -> Self {
        Self {
            status,
            connections: Vec::new(),
            notes: Vec::new(),
        }
    }
}

/// Begin a bounded read-only inventory on a worker thread.
pub fn start_scan() -> Receiver<ScanResult> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(scan());
    });
    rx
}

pub fn scan() -> ScanResult {
    #[cfg(not(windows))]
    {
        return ScanResult::empty(ScanStatus::Unsupported);
    }

    #[cfg(windows)]
    {
        const SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [Text.Encoding]::UTF8
$rows = @(Get-NetTCPConnection -ErrorAction Stop | Where-Object { $_.State -eq 'Listen' -or $_.State -eq 'Established' } | Select-Object -First 250)
$listeners = @{}
foreach ($c in $rows) { if ($c.State -eq 'Listen') { $listeners[([string]$c.OwningProcess + ':' + [string]$c.LocalPort)] = $true } }
$procCache = @{}
foreach ($c in $rows) {
  try {
    $procId = [uint32]$c.OwningProcess
    if (-not $procCache.ContainsKey($procId)) {
      $path = ''
      $name = ''
      try {
        $p = Get-CimInstance Win32_Process -Filter ('ProcessId=' + $procId) -ErrorAction Stop
        $path = [string]$p.ExecutablePath
        $name = [string]$p.Name
      } catch {
        try { $name = [string](Get-Process -Id $procId -ErrorAction Stop).ProcessName } catch { $name = 'Unknown process' }
      }
      $sig = 'Unavailable'
      if ($path -and (Test-Path -LiteralPath $path)) { try { $sig = [string](Get-AuthenticodeSignature -FilePath $path -ErrorAction Stop).Status } catch { $sig = 'Check failed' } }
      $procCache[$procId] = @($name, $path, $sig)
    }
    $pinfo = $procCache[$procId]
    $direction = 'OUT'
    $remote = [string]$c.RemoteAddress
    $rport = [uint16]$c.RemotePort
    if ($c.State -eq 'Listen') { $direction = 'LISTEN'; $remote = ''; $rport = 0 }
    elseif ($listeners.ContainsKey(([string]$procId + ':' + [string]$c.LocalPort))) { $direction = 'IN' }
    $vals = @($direction, [string]$c.LocalAddress, [string]$c.LocalPort, $remote, [string]$rport, [string]$procId, [string]$pinfo[0], [string]$pinfo[1], [string]$pinfo[2])
    $encoded = foreach ($v in $vals) { [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes([string]$v)) }
    [Console]::Out.WriteLine('CONN' + "`t" + ($encoded -join "`t"))
  } catch { continue }
}
[Console]::Out.WriteLine('DONE' + "`t" + [string]$rows.Count)
"#;
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        const TIME_LIMIT: Duration = Duration::from_secs(20);
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
            Ok(c) => c,
            Err(e) => {
                return ScanResult::empty(ScanStatus::Error(format!(
                    "Could not start PowerShell: {e}"
                )));
            }
        };
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let out_thread = std::thread::spawn(move || read_pipe(stdout));
        let err_thread = std::thread::spawn(move || read_pipe(stderr));
        let started = std::time::Instant::now();
        let exit = loop {
            match child.try_wait() {
                Ok(Some(s)) => break Some(s),
                Ok(None) if started.elapsed() < TIME_LIMIT => {
                    std::thread::sleep(Duration::from_millis(100))
                }
                Ok(None) => {
                    let _ = child.kill();
                    break None;
                }
                Err(_) => {
                    let _ = child.kill();
                    break None;
                }
            }
        };
        let output = out_thread.join().unwrap_or_default();
        let stderr = err_thread.join().unwrap_or_default();
        let successful = exit.as_ref().is_some_and(std::process::ExitStatus::success);
        let mut result = ScanResult::empty(if successful {
            ScanStatus::Complete
        } else {
            ScanStatus::Partial
        });
        if exit.is_none() {
            result
                .notes
                .push("Network inventory exceeded its 20 second limit.".into());
        } else if !successful {
            result
                .notes
                .push("PowerShell could not complete the network inventory.".into());
        }
        if !stderr.trim().is_empty() {
            result.notes.push(stderr.trim().chars().take(500).collect());
        }
        let mut saw_done = false;
        for line in output.lines() {
            let mut parts = line.split('\t');
            if parts.next() != Some("CONN") {
                if line.starts_with("DONE\t") {
                    saw_done = true;
                }
                continue;
            }
            let fields: Vec<String> = parts.map(decode).collect();
            if fields.len() != 9 {
                continue;
            }
            let direction = match fields[0].as_str() {
                "LISTEN" => Direction::IncomingListener,
                "IN" => Direction::IncomingEstablished,
                _ => Direction::Outgoing,
            };
            let local_port = fields[2].parse().unwrap_or(0);
            let remote_port = fields[4].parse().unwrap_or(0);
            let process_id = fields[5].parse().unwrap_or(0);
            let public_remote = direction == Direction::Outgoing && is_public_ip(&fields[3]);
            let uncommon_remote = remote_port != 0
                && !matches!(
                    remote_port,
                    20..=25
                        | 53
                        | 67
                        | 68
                        | 80
                        | 110
                        | 123
                        | 143
                        | 389
                        | 443
                        | 465
                        | 587
                        | 993
                        | 995
                        | 1433
                        | 3306
                        | 3389
                        | 5432
                        | 5900
                        | 8080
                        | 8443
                );
            let unusual_listener = direction == Direction::IncomingListener
                && !matches!(
                    local_port,
                    53 | 80 | 135 | 139 | 443 | 445 | 3389 | 5357 | 5358
                );
            let path_lower = fields[7].to_ascii_lowercase();
            let profile_lower = std::env::var("USERPROFILE")
                .unwrap_or_default()
                .to_ascii_lowercase();
            let suspicious_path = direction == Direction::IncomingListener
                && !path_lower.is_empty()
                && (path_lower.contains("\\appdata\\")
                    || path_lower.contains("\\temp\\")
                    || path_lower.contains("\\downloads\\")
                    || (!profile_lower.is_empty() && path_lower.starts_with(&profile_lower)));
            let bad_signature = !matches!(
                fields[8].as_str(),
                "Valid" | "NotSigned" | "Unavailable" | "Check failed" | ""
            );
            let mut reasons = Vec::new();
            if unusual_listener {
                reasons.push("listener uses a non-common port".to_string());
            }
            if suspicious_path {
                reasons.push("listener executable is in a user-writable location".to_string());
            }
            if bad_signature {
                reasons.push(format!("signature status is {}", fields[8]));
            }
            if public_remote && uncommon_remote {
                reasons.push("outgoing connection uses a public IP and uncommon port".to_string());
            }
            let severity =
                if (suspicious_path && bad_signature) || (unusual_listener && suspicious_path) {
                    Severity::Elevated
                } else if !reasons.is_empty() {
                    Severity::Review
                } else {
                    Severity::Info
                };
            let explanation = if reasons.is_empty() {
                "No basic heuristic was triggered; this does not establish that the connection is safe.".into()
            } else {
                reasons.join("; ")
                    + ". Heuristics are a review aid, not proof of malicious activity."
            };
            result.connections.push(Connection {
                direction,
                severity,
                local_address: fields[1].clone(),
                local_port,
                remote_address: fields[3].clone(),
                remote_port,
                process_id,
                process_name: fields[6].clone(),
                executable_path: fields[7].clone(),
                signature: fields[8].clone(),
                explanation,
            });
        }
        if !saw_done && matches!(result.status, ScanStatus::Complete) {
            result.status = ScanStatus::Partial;
        }
        result
            .connections
            .sort_by_key(|c| (c.direction as u8, c.severity as u8));
        if result.connections.is_empty() && !matches!(result.status, ScanStatus::Partial) {
            result
                .notes
                .push("No TCP listeners or established connections were returned.".into());
        }
        result
    }
}

#[cfg(windows)]
fn read_pipe<R: std::io::Read>(pipe: Option<R>) -> String {
    let mut s = String::new();
    if let Some(p) = pipe {
        let _ = p.take(512 * 1024).read_to_string(&mut s);
    }
    s
}

#[cfg(windows)]
fn decode(value: &str) -> String {
    base64_decode(value)
        .and_then(|b| String::from_utf8(b).ok())
        .unwrap_or_default()
}

#[cfg(windows)]
fn base64_decode(value: &str) -> Option<Vec<u8>> {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::with_capacity(value.len() * 3 / 4);
    let mut acc = 0u32;
    let mut bits = 0;
    for b in value.bytes() {
        if b == b'=' {
            break;
        }
        let n = TABLE.iter().position(|x| *x == b)? as u32;
        acc = (acc << 6) | n;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((acc >> bits) & 255) as u8);
        }
    }
    Some(out)
}

#[cfg(windows)]
fn is_public_ip(value: &str) -> bool {
    use std::net::IpAddr;
    match value.parse::<IpAddr>() {
        Ok(IpAddr::V4(ip)) => {
            !(ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_broadcast()
                || ip.is_unspecified()
                || ip.octets()[0] == 0
                || ip.octets()[0] >= 224)
        }
        Ok(IpAddr::V6(ip)) => {
            !(ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_multicast()
                || ip.is_unique_local()
                || ip.is_unicast_link_local()
                || ip
                    .to_ipv4_mapped()
                    .is_some_and(|v| v.is_private() || v.is_loopback()))
        }
        Err(_) => false,
    }
}
