//! Read-only inventory of common Windows startup locations.
//!
//! This reports the four Run/RunOnce registry keys (in both registry views)
//! and shortcut files directly inside the per-user and common Startup folders.
//! It does not resolve `.lnk` targets or inspect services, scheduled tasks,
//! logon scripts, or deeper folders; those require separate inventory paths.

#[cfg(windows)]
use std::collections::HashSet;
#[cfg(windows)]
use std::path::{Path, PathBuf};

const MAX_REGISTRY_ENTRIES: usize = 4096;
const MAX_FOLDER_ENTRIES: usize = 512;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StartupSource {
    Registry {
        hive: RegistryHive,
        key: &'static str,
    },
    StartupFolder {
        scope: StartupFolderScope,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RegistryHive {
    CurrentUser,
    LocalMachine,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartupFolderScope {
    CurrentUser,
    Common,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResolutionStatus {
    /// The command's executable was found on disk or through PATH.
    Resolved,
    /// The executable could not be confirmed, or a shortcut target was not inspected.
    Unresolved,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StartupEntry {
    pub display_name: String,
    pub source: StartupSource,
    /// Registry command line, or the full path to a Startup-folder shortcut.
    pub command_or_path: String,
    pub status: ResolutionStatus,
    /// Why status is unresolved; empty when the executable was confirmed.
    pub resolution_note: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StartupInventory {
    pub entries: Vec<StartupEntry>,
    pub unreadable_locations: usize,
    pub limit_reached: bool,
}

/// Collect startup entries without changing registry values or files.
pub fn scan_startup_inventory() -> StartupInventory {
    let mut inventory = StartupInventory::default();
    #[cfg(windows)]
    {
        inventory.entries = scan_registry(&mut inventory);
        scan_startup_folders(&mut inventory);
    }
    inventory
}

#[cfg(windows)]
fn scan_registry(inventory: &mut StartupInventory) -> Vec<StartupEntry> {
    use winreg::RegKey;
    use winreg::enums::{
        HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY,
        REG_EXPAND_SZ, REG_SZ,
    };

    let locations = [
        (
            RegistryHive::CurrentUser,
            HKEY_CURRENT_USER,
            "Software\\Microsoft\\Windows\\CurrentVersion\\Run",
        ),
        (
            RegistryHive::CurrentUser,
            HKEY_CURRENT_USER,
            "Software\\Microsoft\\Windows\\CurrentVersion\\RunOnce",
        ),
        (
            RegistryHive::LocalMachine,
            HKEY_LOCAL_MACHINE,
            "Software\\Microsoft\\Windows\\CurrentVersion\\Run",
        ),
        (
            RegistryHive::LocalMachine,
            HKEY_LOCAL_MACHINE,
            "Software\\Microsoft\\Windows\\CurrentVersion\\RunOnce",
        ),
    ];
    let mut result = Vec::new();
    let mut seen = HashSet::new();

    'locations: for (hive, raw_hive, path) in locations {
        for view in [KEY_WOW64_64KEY, KEY_WOW64_32KEY] {
            let root = RegKey::predef(raw_hive);
            let Ok(key) = root.open_subkey_with_flags(path, KEY_READ | view) else {
                // Missing keys are normal on some Windows configurations.
                continue;
            };
            for value in key.enum_values() {
                let Ok((name, value)) = value else {
                    inventory.unreadable_locations += 1;
                    continue;
                };
                if !matches!(value.vtype, REG_SZ | REG_EXPAND_SZ) {
                    continue;
                }
                let command = value.to_string();
                if command.trim().is_empty() {
                    continue;
                }
                if !seen.insert((hive, path, name.clone(), command.clone())) {
                    continue;
                }
                let (status, note) = resolve_command(&command);
                result.push(StartupEntry {
                    display_name: if name.is_empty() {
                        "(Default)".into()
                    } else {
                        name
                    },
                    source: StartupSource::Registry {
                        hive,
                        key: if path.ends_with("RunOnce") {
                            "RunOnce"
                        } else {
                            "Run"
                        },
                    },
                    command_or_path: command,
                    status,
                    resolution_note: note,
                });
                if result.len() >= MAX_REGISTRY_ENTRIES {
                    inventory.limit_reached = true;
                    break 'locations;
                }
            }
        }
    }
    result
}

#[cfg(windows)]
fn scan_startup_folders(inventory: &mut StartupInventory) {
    let current_user = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .map(|path| path.join("Microsoft\\Windows\\Start Menu\\Programs\\Startup"));
    let common = std::env::var_os("ProgramData")
        .map(PathBuf::from)
        .map(|path| path.join("Microsoft\\Windows\\Start Menu\\Programs\\Startup"));

    for (scope, path) in [
        (StartupFolderScope::CurrentUser, current_user),
        (StartupFolderScope::Common, common),
    ] {
        let Some(path) = path else { continue };
        let Ok(items) = std::fs::read_dir(path) else {
            continue;
        };
        let mut seen = 0usize;
        for item in items {
            seen += 1;
            if seen > MAX_FOLDER_ENTRIES {
                inventory.limit_reached = true;
                break;
            }
            let Ok(item) = item else {
                inventory.unreadable_locations += 1;
                continue;
            };
            let item_path = item.path();
            if !item_path.is_file() || !is_shortcut(&item_path) {
                continue;
            }
            let display_name = item_path
                .file_stem()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| {
                    item_path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned()
                });
            inventory.entries.push(StartupEntry {
                display_name,
                source: StartupSource::StartupFolder { scope },
                command_or_path: item_path.to_string_lossy().into_owned(),
                status: ResolutionStatus::Unresolved,
                resolution_note: "Shortcut target is not inspected in this inventory.".into(),
            });
        }
    }
}
#[cfg(windows)]
fn is_shortcut(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("lnk") || extension.eq_ignore_ascii_case("url")
        })
}

#[cfg(windows)]
fn resolve_command(command: &str) -> (ResolutionStatus, String) {
    let expanded = expand_environment(command.trim());
    if let Some(program) = executable_candidate(&expanded)
        && executable_exists(&program)
    {
        return (ResolutionStatus::Resolved, String::new());
    }
    (
        ResolutionStatus::Unresolved,
        "Executable path could not be confirmed; command was not run.".into(),
    )
}

#[cfg(windows)]
fn executable_candidate(command: &str) -> Option<String> {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(rest) = trimmed.strip_prefix('"') {
        return rest.find('"').map(|end| rest[..end].to_owned());
    }
    let lower = trimmed.to_ascii_lowercase();
    let end = [".exe", ".com", ".bat", ".cmd"]
        .iter()
        .filter_map(|extension| lower.find(extension).map(|index| index + extension.len()))
        .min()
        .unwrap_or_else(|| trimmed.find(char::is_whitespace).unwrap_or(trimmed.len()));
    Some(trimmed[..end].trim().to_owned())
}

#[cfg(windows)]
fn executable_exists(program: &str) -> bool {
    let path = Path::new(program);
    if path.is_file() {
        return true;
    }
    if path.components().count() > 1 {
        return false;
    }
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|value| std::env::split_paths(&value).collect::<Vec<_>>())
        .any(|directory| directory.join(program).is_file())
}

#[cfg(windows)]
fn expand_environment(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    let mut remaining = value;
    while let Some(start) = remaining.find('%') {
        result.push_str(&remaining[..start]);
        let after_start = &remaining[start + 1..];
        let Some(end) = after_start.find('%') else {
            result.push_str(&remaining[start..]);
            return result;
        };
        let variable = &after_start[..end];
        if let Some(value) = std::env::var_os(variable) {
            result.push_str(&value.to_string_lossy());
        } else {
            result.push('%');
            result.push_str(variable);
            result.push('%');
        }
        remaining = &after_start[end + 1..];
    }
    result.push_str(remaining);
    result
}
