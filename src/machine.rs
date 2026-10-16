//! Everything the old "PC Info" labels showed, read from the registry and
//! `sysinfo`. The Windows activation state is read locally from Software
//! Licensing WMI; no Wrenchmaster licensing or network calls are involved.

use crate::util::human_bytes;
use std::sync::OnceLock;

#[cfg(windows)]
use winreg::RegKey;
#[cfg(windows)]
use winreg::enums::HKEY_LOCAL_MACHINE;

/// The Windows generation, with the brand colour the 2017 build painted its
/// splash screen with. That version switched a `PictureBox` logo and a
/// progress-bar colour per OS; the logos were bitmaps in the resources folder,
/// but the palette is just data, so it lives on as this app's accent colour.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Era {
    Win11,
    Win10,
    Win81,
    Win8,
    Win7,
    Vista,
    WinXp,
    Legacy,
    Unknown,
}

impl Era {
    /// Original values from `OsSelection.vb`, kept exactly.
    pub fn accent(self) -> [u8; 3] {
        match self {
            // Windows 11 postdates the original, so it gets the app's own blue.
            Era::Win11 => [0x4D, 0xA3, 0xFF],
            Era::Win10 => [0, 120, 214],
            Era::Win81 => [2, 50, 140],
            Era::Win8 => [0, 173, 239],
            Era::Win7 => [64, 182, 232],
            Era::Vista => [2, 23, 140],
            Era::WinXp => [235, 101, 28],
            Era::Legacy => [93, 172, 57],
            Era::Unknown => [0x4D, 0xA3, 0xFF],
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Era::Win11 => "Windows 11 era",
            Era::Win10 => "Windows 10 era",
            Era::Win81 => "Windows 8.1 era",
            Era::Win8 => "Windows 8 era",
            Era::Win7 => "Windows 7 era",
            Era::Vista => "Windows Vista era",
            Era::WinXp => "Windows XP era",
            Era::Legacy => "Pre-XP Windows",
            Era::Unknown => "Unrecognised Windows",
        }
    }

    /// Everything from Vista up is a supported NT 6+ shell; the older ones are
    /// listed for the same reason the original listed them, as a courtesy.
    pub fn supported(self) -> bool {
        matches!(
            self,
            Era::Win11 | Era::Win10 | Era::Win81 | Era::Win8 | Era::Win7
        )
    }
}

fn era_from(product: &str, build: u32) -> Era {
    // Build numbers first, because `ProductName` lies on Windows 11.
    match build {
        22_000.. => return Era::Win11,
        10_240..=21_999 => return Era::Win10,
        9600..=10_239 => return Era::Win81,
        9200..=9599 => return Era::Win8,
        7600..=9199 => return Era::Win7,
        6000..=7599 => return Era::Vista,
        2600..=5999 => return Era::WinXp,
        _ => {}
    }
    // Fall back to the string match the original used.
    let name = product.to_ascii_lowercase();
    for (needle, era) in [
        ("windows 11", Era::Win11),
        ("windows 10", Era::Win10),
        ("windows 8.1", Era::Win81),
        ("windows 8", Era::Win8),
        ("windows 7", Era::Win7),
        ("vista", Era::Vista),
        ("windows xp", Era::WinXp),
        ("windows 2000", Era::Legacy),
        ("windows me", Era::Legacy),
        ("windows 98", Era::Legacy),
        ("windows 95", Era::Legacy),
    ] {
        if name.contains(needle) {
            return era;
        }
    }
    Era::Unknown
}

pub struct DiskInfo {
    pub name: String,
    pub mount: String,
    pub total: u64,
    pub available: u64,
    pub fs: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ActivationStatus {
    Active,
    NotActive,
    Unknown,
}

static WINDOWS_ACTIVATION_STATUS: OnceLock<ActivationStatus> = OnceLock::new();

impl DiskInfo {
    pub fn used(&self) -> u64 {
        self.total.saturating_sub(self.available)
    }
    pub fn used_fraction(&self) -> f32 {
        if self.total == 0 {
            0.0
        } else {
            self.used() as f32 / self.total as f32
        }
    }
}

pub struct Machine {
    pub windows_name: String,
    pub activation_status: ActivationStatus,
    pub era: Era,
    pub windows_build: String,
    pub windows_arch: String,
    pub host_name: String,
    pub app_arch: String,
    pub cpu: String,
    pub cpu_cores: String,
    pub gpus: Vec<String>,
    pub motherboard: String,
    pub bios: String,
    pub ram_total: u64,
    pub ram_used: u64,
    pub uptime: String,
    pub uptime_seconds: u64,
    pub machine_guid: String,
    pub disks: Vec<DiskInfo>,
}

impl Machine {
    pub fn collect() -> Self {
        // CPU identity is already in the registry. Avoid sysinfo's initial CPU
        // sampling delay; the process and disk details are refreshed elsewhere.
        let cpu = reg_hw("CentralProcessor\\0", "ProcessorNameString")
            .unwrap_or_else(|| "Unknown processor".into());

        let logical_threads = std::thread::available_parallelism()
            .map(std::num::NonZeroUsize::get)
            .unwrap_or_default();
        let cpu_cores = match sysinfo::System::physical_core_count() {
            Some(p) if logical_threads > 0 => format!("{p} cores / {logical_threads} threads"),
            Some(p) => format!("{p} cores"),
            None => format!("{logical_threads} threads"),
        };

        let disks = sysinfo::Disks::new_with_refreshed_list()
            .iter()
            .map(|d| DiskInfo {
                name: {
                    let n = d.name().to_string_lossy().trim().to_string();
                    if n.is_empty() { "Local disk".into() } else { n }
                },
                mount: d.mount_point().to_string_lossy().to_string(),
                total: d.total_space(),
                available: d.available_space(),
                fs: d.file_system().to_string_lossy().to_string(),
            })
            .collect();
        let memory = crate::ram::memory_snapshot().unwrap_or_default();

        Self {
            windows_name: windows_name(),
            activation_status: *WINDOWS_ACTIVATION_STATUS.get_or_init(detect_windows_activation),
            era: era_from(
                &reg_sw(r"Microsoft\Windows NT\CurrentVersion", "ProductName").unwrap_or_default(),
                current_build(),
            ),
            windows_build: windows_build(),
            windows_arch: match std::env::var("PROCESSOR_ARCHITECTURE").as_deref() {
                Ok("AMD64") => "64-bit (x64)".into(),
                Ok("ARM64") => "64-bit (ARM64)".into(),
                Ok("x86") => "32-bit (x86)".into(),
                Ok(other) => other.to_string(),
                Err(_) => "Unknown".into(),
            },
            host_name: sysinfo::System::host_name().unwrap_or_else(|| "this-pc".into()),
            app_arch: if cfg!(target_pointer_width = "64") {
                "64-bit".into()
            } else {
                "32-bit".into()
            },
            cpu,
            cpu_cores,
            gpus: graphics_adapters(),
            motherboard: {
                let vendor = reg_hw_bios("BaseBoardManufacturer").unwrap_or_default();
                let board = reg_hw_bios("BaseBoardProduct").unwrap_or_default();
                let joined = format!("{vendor} {board}").trim().to_string();
                if joined.is_empty() {
                    "Unknown motherboard".into()
                } else {
                    joined
                }
            },
            bios: {
                let vendor = reg_hw_bios("BIOSVendor").unwrap_or_default();
                let version = reg_hw_bios("BIOSVersion").unwrap_or_default();
                let date = reg_hw_bios("BIOSReleaseDate").unwrap_or_default();
                let joined = format!("{vendor} {version} {date}").trim().to_string();
                if joined.is_empty() {
                    "Unknown firmware".into()
                } else {
                    joined
                }
            },
            ram_total: memory.physical_total_bytes,
            ram_used: memory
                .physical_total_bytes
                .saturating_sub(memory.physical_available_bytes),
            uptime: format_uptime(sysinfo::System::uptime()),
            uptime_seconds: sysinfo::System::uptime(),
            machine_guid: reg_sw("Microsoft\\Cryptography", "MachineGuid")
                .unwrap_or_else(|| "unavailable".into()),
            disks,
        }
    }

    pub fn ram_summary(&self) -> String {
        format!(
            "{} used of {}",
            human_bytes(self.ram_used),
            human_bytes(self.ram_total)
        )
    }

    pub fn uptime_days(&self) -> u64 {
        self.uptime_seconds / 86_400
    }

    pub fn ram_fraction(&self) -> f32 {
        if self.ram_total == 0 {
            0.0
        } else {
            self.ram_used as f32 / self.ram_total as f32
        }
    }
}

/// Read the local Windows Software Licensing provider once per process.
/// A missing provider or failed query remains `Unknown`; it is never treated
/// as an unactivated Windows installation.
#[cfg(windows)]
fn detect_windows_activation() -> ActivationStatus {
    use std::os::windows::process::CommandExt;
    use std::process::Command;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const SCRIPT: &str = r#"$ErrorActionPreference = 'Stop'; [Console]::OutputEncoding = [System.Text.Encoding]::UTF8; $appId = '55c92734-d682-4d71-983e-d6ec3f16059f'; $products = @(Get-CimInstance -Namespace root\cimv2 -ClassName SoftwareLicensingProduct -Filter "ApplicationID='$appId'"); if ($products | Where-Object { $_.Name -like 'Windows*' -and $_.LicenseStatus -eq 1 }) { [Console]::Out.Write('1') } else { [Console]::Out.Write('0') }"#;

    let output = Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-WindowStyle",
            "Hidden",
            "-Command",
            SCRIPT,
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output();

    let Ok(output) = output else {
        return ActivationStatus::Unknown;
    };
    if !output.status.success() {
        return ActivationStatus::Unknown;
    }

    match String::from_utf8_lossy(&output.stdout).trim() {
        "1" => ActivationStatus::Active,
        "0" => ActivationStatus::NotActive,
        _ => ActivationStatus::Unknown,
    }
}

#[cfg(not(windows))]
fn detect_windows_activation() -> ActivationStatus {
    ActivationStatus::Unknown
}

fn format_uptime(seconds: u64) -> String {
    let days = seconds / 86_400;
    let hours = (seconds % 86_400) / 3_600;
    let minutes = (seconds % 3_600) / 60;
    if days > 0 {
        format!("{days}d {hours}h {minutes}m")
    } else if hours > 0 {
        format!("{hours}h {minutes}m")
    } else {
        format!("{minutes}m")
    }
}

/// `ProductName` still reads "Windows 10 Pro" on Windows 11 — Microsoft never
/// updated the value, and the 2017 build believed it, which is why that one
/// reported Windows 11 machines as 10. The build number is the real answer:
/// 22000 is the first Windows 11 build, 10240 the first Windows 10 one.
fn current_build() -> u32 {
    reg_sw(r"Microsoft\Windows NT\CurrentVersion", "CurrentBuild")
        .and_then(|b| b.parse().ok())
        .unwrap_or(0)
}

fn windows_name() -> String {
    let product = reg_sw("Microsoft\\Windows NT\\CurrentVersion", "ProductName")
        .or_else(sysinfo::System::long_os_version)
        .unwrap_or_else(|| "Windows".into());

    let build: u32 = reg_sw("Microsoft\\Windows NT\\CurrentVersion", "CurrentBuild")
        .and_then(|b| b.parse().ok())
        .unwrap_or(0);

    // Server SKUs have their own naming and a different build mapping; leave
    // whatever the registry says alone.
    let is_server = reg_sw("Microsoft\\Windows NT\\CurrentVersion", "InstallationType")
        .is_some_and(|t| !t.eq_ignore_ascii_case("Client"));

    resolve_name(&product, build, is_server)
}

fn resolve_name(product: &str, build: u32, is_server: bool) -> String {
    if is_server || build < 22_000 || product.contains("Windows 11") {
        return product.to_string();
    }
    // Keep the edition suffix: "Windows 10 Pro" becomes "Windows 11 Pro".
    match product.split_once("Windows 10") {
        Some((_, rest)) => format!("Windows 11{rest}"),
        None => format!("Windows 11 ({product})"),
    }
}

fn windows_build() -> String {
    let display = reg_sw("Microsoft\\Windows NT\\CurrentVersion", "DisplayVersion")
        .or_else(|| reg_sw("Microsoft\\Windows NT\\CurrentVersion", "ReleaseId"))
        .unwrap_or_default();
    let build = reg_sw("Microsoft\\Windows NT\\CurrentVersion", "CurrentBuild")
        .unwrap_or_else(|| "?".into());
    let ubr = reg_sw_dword("Microsoft\\Windows NT\\CurrentVersion", "UBR");
    match (display.is_empty(), ubr) {
        (false, Some(ubr)) => format!("{display} (build {build}.{ubr})"),
        (false, None) => format!("{display} (build {build})"),
        (true, Some(ubr)) => format!("build {build}.{ubr}"),
        (true, None) => format!("build {build}"),
    }
}

/// Walk the display-adapter class key instead of asking WMI for
/// `win32_VideoController`, which is what used to raise the "We have a WMI
/// problem..." dialog on half the machines that ran the 2017 build.
fn graphics_adapters() -> Vec<String> {
    #[cfg(windows)]
    {
        const CLASS: &str =
            r"SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}";
        let mut out: Vec<String> = Vec::new();
        if let Ok(class) = RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey(CLASS) {
            for name in class.enum_keys().flatten() {
                if !name.chars().all(|c| c.is_ascii_digit()) {
                    continue;
                }
                if let Ok(sub) = class.open_subkey(&name) {
                    if let Ok(desc) = sub.get_value::<String, _>("DriverDesc") {
                        if !out.contains(&desc) {
                            out.push(desc);
                        }
                    }
                }
            }
        }
        if out.is_empty() {
            out.push("No display adapter reported".into());
        }
        out
    }
    #[cfg(not(windows))]
    {
        vec!["Not Windows".into()]
    }
}

#[cfg(windows)]
fn reg_sw(path: &str, value: &str) -> Option<String> {
    RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey(format!("SOFTWARE\\{path}"))
        .ok()?
        .get_value::<String, _>(value)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

#[cfg(windows)]
fn reg_sw_dword(path: &str, value: &str) -> Option<u32> {
    RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey(format!("SOFTWARE\\{path}"))
        .ok()?
        .get_value::<u32, _>(value)
        .ok()
}

#[cfg(windows)]
fn reg_hw(path: &str, value: &str) -> Option<String> {
    RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey(format!("HARDWARE\\DESCRIPTION\\System\\{path}"))
        .ok()?
        .get_value::<String, _>(value)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

#[cfg(windows)]
fn reg_hw_bios(value: &str) -> Option<String> {
    reg_hw("BIOS", value)
}

#[cfg(not(windows))]
fn reg_sw(_path: &str, _value: &str) -> Option<String> {
    None
}
#[cfg(not(windows))]
fn reg_sw_dword(_path: &str, _value: &str) -> Option<u32> {
    None
}
#[cfg(not(windows))]
fn reg_hw(_path: &str, _value: &str) -> Option<String> {
    None
}
#[cfg(not(windows))]
fn reg_hw_bios(_value: &str) -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::resolve_name;

    #[test]
    fn promotes_windows_11_builds_despite_the_registry() {
        // What this machine actually reports: 25H2, build 26200, "Windows 10 Pro".
        assert_eq!(
            resolve_name("Windows 10 Pro", 26_200, false),
            "Windows 11 Pro"
        );
        assert_eq!(
            resolve_name("Windows 10 Home", 22_000, false),
            "Windows 11 Home"
        );
    }

    #[test]
    fn leaves_real_windows_10_alone() {
        assert_eq!(
            resolve_name("Windows 10 Pro", 19_045, false),
            "Windows 10 Pro"
        );
        assert_eq!(
            resolve_name("Windows 7 Ultimate", 7601, false),
            "Windows 7 Ultimate"
        );
    }

    #[test]
    fn leaves_servers_and_already_correct_names_alone() {
        assert_eq!(
            resolve_name("Windows Server 2025 Standard", 26_100, true),
            "Windows Server 2025 Standard"
        );
        assert_eq!(
            resolve_name("Windows 11 Pro", 26_200, false),
            "Windows 11 Pro"
        );
    }
}
