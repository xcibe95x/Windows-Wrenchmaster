//! Read-only, conservative audit of installed-app uninstall records.
//!
//! A record is only suggested as a possible leftover when its InstallLocation
//! does not point to an existing absolute directory and a resolved uninstaller
//! executable does not exist. This module never writes to the registry.

use std::path::{Path, PathBuf};

const UNINSTALL_KEY: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall";
const MAX_RECORDS: usize = 4096;
const MAX_CANDIDATES: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegistryHive {
    CurrentUser,
    LocalMachine,
}

impl RegistryHive {
    fn label(self) -> &'static str {
        match self {
            Self::CurrentUser => "HKCU",
            Self::LocalMachine => "HKLM",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegistryView {
    Default,
    View32,
    View64,
}

#[derive(Clone, Debug)]
pub struct Candidate {
    pub display_name: String,
    pub publisher: Option<String>,
    /// Exact registry location, including hive and registry view.
    pub key: String,
    pub source_hive: RegistryHive,
    pub source_view: RegistryView,
    pub evidence: Vec<String>,
    pub reason: String,
}

#[derive(Clone, Debug, Default)]
pub struct AuditReport {
    pub records_scanned: usize,
    pub unreadable_records: usize,
    pub candidates: Vec<Candidate>,
    pub limit_reached: bool,
}

/// Scan installed-app uninstall records without modifying registry data.
///
/// On 64-bit Windows this checks both 64-bit and 32-bit views of HKLM and
/// HKCU. On 32-bit Windows it checks the default view. Other platforms return
/// an empty report.
pub fn scan_uninstall_entries() -> AuditReport {
    #[cfg(windows)]
    {
        scan_windows()
    }
    #[cfg(not(windows))]
    {
        AuditReport::default()
    }
}

#[cfg(windows)]
fn scan_windows() -> AuditReport {
    use winreg::RegKey;
    use winreg::enums::{
        HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY,
    };

    let mut report = AuditReport::default();
    let hives = [
        (RegistryHive::CurrentUser, HKEY_CURRENT_USER),
        (RegistryHive::LocalMachine, HKEY_LOCAL_MACHINE),
    ];
    let views: Vec<(RegistryView, u32)> = if cfg!(target_pointer_width = "64") {
        vec![
            (RegistryView::View64, KEY_WOW64_64KEY),
            (RegistryView::View32, KEY_WOW64_32KEY),
        ]
    } else {
        vec![(RegistryView::Default, 0)]
    };

    'hives: for (hive_kind, hive_handle) in hives {
        for (view, view_flag) in &views {
            let root = RegKey::predef(hive_handle);
            let Ok(uninstall) = root.open_subkey_with_flags(UNINSTALL_KEY, KEY_READ | *view_flag)
            else {
                continue;
            };
            for subkey_name in uninstall.enum_keys() {
                if report.records_scanned >= MAX_RECORDS
                    || report.candidates.len() >= MAX_CANDIDATES
                {
                    report.limit_reached = true;
                    break 'hives;
                }
                let Ok(subkey_name) = subkey_name else {
                    report.unreadable_records += 1;
                    continue;
                };
                report.records_scanned += 1;
                let Ok(record) = uninstall.open_subkey_with_flags(&subkey_name, KEY_READ) else {
                    report.unreadable_records += 1;
                    continue;
                };
                let display_name: String = match record.get_value::<String, _>("DisplayName") {
                    Ok(value) if !value.trim().is_empty() => value,
                    _ => continue,
                };
                if is_excluded_record(&record) {
                    continue;
                }
                let publisher = record
                    .get_value::<String, _>("Publisher")
                    .ok()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty());

                let install_location = record
                    .get_value::<String, _>("InstallLocation")
                    .ok()
                    .map(|s| expand_environment(&s));
                let has_existing_absolute_install_location = install_location
                    .as_deref()
                    .map(Path::new)
                    .is_some_and(|path| path.is_absolute() && path.is_dir());
                if has_existing_absolute_install_location {
                    continue;
                }

                // Prefer QuietUninstallString when supplied. An unparseable
                // command makes the record inconclusive and is never flagged.
                let commands: Vec<String> = ["QuietUninstallString", "UninstallString"]
                    .into_iter()
                    .filter_map(|name| record.get_value::<String, _>(name).ok())
                    .filter(|command| !command.trim().is_empty())
                    .collect();
                if commands.is_empty() {
                    continue;
                }
                let resolved: Option<Vec<PathBuf>> = commands
                    .iter()
                    .map(|command| resolve_uninstaller_executable(command))
                    .collect();
                let Some(executables) = resolved else {
                    continue;
                };
                if executables.iter().any(|path| path.is_file()) {
                    continue;
                }
                let missing_executables = executables
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join("; ");

                let install_evidence = match install_location.as_deref() {
                    Some(path) if !path.trim().is_empty() => {
                        format!("InstallLocation is not an existing absolute directory: {path}")
                    }
                    _ => "InstallLocation value is absent".to_string(),
                };
                let evidence = vec![
                    install_evidence,
                    format!(
                        "Uninstall command(s) resolve to missing executable(s): {missing_executables}"
                    ),
                ];
                report.candidates.push(Candidate {
                    display_name,
                    publisher,
                    key: format!(r"{}\{}\{}", hive_kind.label(), UNINSTALL_KEY, subkey_name),
                    source_hive: hive_kind,
                    source_view: *view,
                    evidence,
                    reason: "Possible leftover uninstall record: no existing install directory or uninstaller executable was found.".into(),
                });
            }
        }
    }
    report
}

#[cfg(windows)]
fn is_excluded_record(record: &winreg::RegKey) -> bool {
    // Windows Installer and Store registrations use package identity or
    // installer-managed commands; they are intentionally outside this audit.
    let windows_installer: u32 = record.get_value("WindowsInstaller").unwrap_or_default();
    let system_component: u32 = record.get_value("SystemComponent").unwrap_or_default();
    let release_type: String = record.get_value("ReleaseType").unwrap_or_default();
    let parent_key: String = record.get_value("ParentKeyName").unwrap_or_default();
    let store_id: String = record.get_value("StoreID").unwrap_or_default();
    windows_installer != 0
        || system_component != 0
        || !release_type.trim().is_empty()
        || !parent_key.trim().is_empty()
        || !store_id.trim().is_empty()
}

#[cfg(windows)]
fn resolve_uninstaller_executable(command: &str) -> Option<PathBuf> {
    let command = expand_environment(command.trim());
    if command.is_empty() || command.contains('%') {
        return None;
    }
    let lower = command.to_ascii_lowercase();
    if lower.starts_with("msiexec")
        || lower.starts_with("msiexec.exe")
        || lower.contains("windowsapps")
        || lower.starts_with("shell:")
        || lower.starts_with("rundll32")
        || lower.starts_with("cmd.exe")
        || lower.starts_with("powershell")
        || lower.starts_with("pwsh")
    {
        return None;
    }

    let executable = if let Some(after_quote) = command.strip_prefix('"') {
        let end = after_quote.find('"')?;
        after_quote[..end].to_string()
    } else {
        // For unquoted commands, stop at the first .exe boundary. This handles
        // paths containing spaces while avoiding guesses for commands without
        // an explicit executable boundary.
        let end = lower.find(".exe")?.checked_add(4)?;
        command[..end].trim().to_string()
    };
    let path = PathBuf::from(executable);
    if !path.is_absolute()
        || !path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
    {
        return None;
    }
    Some(path)
}

#[cfg(windows)]
fn expand_environment(value: &str) -> String {
    let mut expanded = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(start) = rest.find('%') {
        expanded.push_str(&rest[..start]);
        let after_start = &rest[start + 1..];
        let Some(end) = after_start.find('%') else {
            expanded.push_str(&rest[start..]);
            return expanded;
        };
        let name = &after_start[..end];
        if let Some(value) = std::env::var_os(name) {
            expanded.push_str(&value.to_string_lossy());
        } else {
            expanded.push('%');
            expanded.push_str(name);
            expanded.push('%');
        }
        rest = &after_start[end + 1..];
    }
    expanded.push_str(rest);
    expanded
}
