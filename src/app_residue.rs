//! Read-only, heuristic scanner for possible leftovers in the current user's
//! AppData folders. Findings are suggestions for review, never deletion targets.

use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use walkdir::WalkDir;

const MAX_CANDIDATES: usize = 2_000;
const MAX_ENTRIES: usize = 40_000;
const MAX_DEPTH: usize = 5;
const MAX_SCAN_TIME: Duration = Duration::from_secs(12);
const MAX_SIZE_ENTRIES_PER_FOLDER: usize = 8_000;
const MAX_SIZE_TIME_PER_FOLDER: Duration = Duration::from_secs(2);

#[derive(Clone, Debug)]
pub struct Finding {
    /// Exact path for user inspection.
    pub path: PathBuf,
    pub name: String,
    /// Estimated total file bytes. May be partial when `size_complete` is false.
    pub bytes: u64,
    pub size_complete: bool,
    pub last_write: Option<SystemTime>,
    /// Always low confidence: filesystem names cannot prove an app was removed.
    pub caveat: &'static str,
}

#[derive(Clone, Debug, Default)]
pub struct ScanResult {
    pub findings: Vec<Finding>,
    pub registry_apps_seen: usize,
    pub folders_examined: usize,
    pub unreadable: usize,
    pub limit_reached: bool,
}

#[derive(Clone, Debug)]
struct InstalledApp {
    name: String,
    install_location: Option<PathBuf>,
}

/// Scans the current user's Local and Roaming AppData for likely app-specific
/// folders that do not appear to belong to an app still registered as installed.
/// This is strictly read-only. Matching is deliberately conservative and all
/// findings require manual review. On non-Windows platforms, returns an empty
/// result.
pub fn scan() -> ScanResult {
    let mut result = ScanResult::default();
    #[cfg(not(windows))]
    return result;

    #[cfg(windows)]
    {
        let apps = installed_apps();
        result.registry_apps_seen = apps.len();
        let roots = appdata_roots();
        let started = Instant::now();
        let mut seen = HashSet::new();

        'roots: for root in roots {
            for entry in WalkDir::new(&root)
                .max_depth(MAX_DEPTH)
                .follow_links(false)
                .into_iter()
            {
                if started.elapsed() >= MAX_SCAN_TIME
                    || result.folders_examined >= MAX_ENTRIES
                    || result.findings.len() >= MAX_CANDIDATES
                {
                    result.limit_reached = true;
                    break 'roots;
                }
                let entry = match entry {
                    Ok(entry) => entry,
                    Err(_) => {
                        result.unreadable += 1;
                        continue;
                    }
                };
                result.folders_examined += 1;
                if !entry.file_type().is_dir() || entry.file_type().is_symlink() {
                    continue;
                }
                let path = entry.path();
                if is_reparse_point(path) || excluded_path(path) {
                    continue;
                }
                // Only inspect direct app folders and one vendor/app nesting level.
                let depth = entry.depth();
                if !(1..=2).contains(&depth) {
                    continue;
                }
                let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
                    continue;
                };
                if !plausible_app_folder(name) || belongs_to_installed_app(path, name, &apps) {
                    continue;
                }
                let key = path.to_string_lossy().to_ascii_lowercase();
                if !seen.insert(key) {
                    continue;
                }
                let metadata = match fs::symlink_metadata(path) {
                    Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
                        metadata
                    }
                    _ => {
                        result.unreadable += 1;
                        continue;
                    }
                };
                let (bytes, complete) = folder_size(path, started + MAX_SCAN_TIME);
                result.findings.push(Finding {
                    path: path.to_path_buf(),
                    name: name.to_owned(),
                    bytes,
                    size_complete: complete,
                    last_write: metadata.modified().ok(),
                    caveat: "Low-confidence candidate: this folder may belong to a portable app, shared component, or an app still in use. Review its exact path and contents before taking action.",
                });
            }
        }
        result
            .findings
            .sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.name.cmp(&b.name)));
        result
    }
}

#[cfg(windows)]
fn installed_apps() -> Vec<InstalledApp> {
    use winreg::RegKey;
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ};

    const UNINSTALL: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall";
    let mut apps = Vec::new();
    let sources = [
        (HKEY_CURRENT_USER, UNINSTALL, KEY_READ),
        (
            HKEY_LOCAL_MACHINE,
            UNINSTALL,
            KEY_READ | winreg::enums::KEY_WOW64_64KEY,
        ),
        (
            HKEY_LOCAL_MACHINE,
            UNINSTALL,
            KEY_READ | winreg::enums::KEY_WOW64_32KEY,
        ),
    ];
    for (hive, path, access) in sources {
        let Ok(root) = RegKey::predef(hive).open_subkey_with_flags(path, access) else {
            continue;
        };
        for key_name in root.enum_keys().filter_map(Result::ok) {
            let Ok(key) = root.open_subkey_with_flags(key_name, KEY_READ) else {
                continue;
            };
            let Ok(name) = key.get_value::<String, _>("DisplayName") else {
                continue;
            };
            if name.trim().is_empty() {
                continue;
            }
            let location = key
                .get_value::<String, _>("InstallLocation")
                .ok()
                .map(|s| PathBuf::from(s.trim_matches('"')))
                .filter(|p| !p.as_os_str().is_empty());
            apps.push(InstalledApp {
                name,
                install_location: location,
            });
        }
    }
    apps
}

#[cfg(windows)]
fn appdata_roots() -> Vec<PathBuf> {
    ["LOCALAPPDATA", "APPDATA"]
        .into_iter()
        .filter_map(env::var_os)
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
        .collect()
}

#[cfg(windows)]
fn is_reparse_point(path: &Path) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    fs::symlink_metadata(path)
        .map(|m| m.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0)
        .unwrap_or(true)
}

#[cfg(not(windows))]
fn is_reparse_point(_: &Path) -> bool {
    false
}

fn excluded_path(path: &Path) -> bool {
    let normalized = path
        .to_string_lossy()
        .replace('/', "\\")
        .to_ascii_lowercase();
    let leaf = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    const EXCLUDED_PARTS: &[&str] = &[
        "\\mozilla\\",
        "\\firefox\\",
        "\\chrome\\",
        "\\chromium\\",
        "\\edge\\",
        "\\msedge\\",
        "\\bravesoftware\\",
        "\\opera\\",
        "\\vivaldi\\",
        "\\discord\\",
        "\\teams\\",
        "\\microsoft\\",
        "\\windows\\",
        "\\packages\\",
        "\\package cache\\",
        "\\installer\\",
        "\\nvidia\\",
        "\\amd\\",
        "\\intel\\",
        "\\dotnet\\",
        "\\nuget\\",
        "\\java\\",
        "\\jre\\",
        "\\node-gyp\\",
        "\\electron\\",
        "\\webview2\\",
        "\\squirreltemp\\",
        "\\crashdumps\\",
    ];
    if EXCLUDED_PARTS.iter().any(|part| normalized.contains(part)) {
        return true;
    }
    matches!(
        leaf.as_str(),
        "temp"
            | "temporary internet files"
            | "cache"
            | "caches"
            | "logs"
            | "crashreports"
            | "crashpad"
            | "code cache"
            | "gpu cache"
            | "shadercache"
    ) || leaf.starts_with("{")
}

fn plausible_app_folder(name: &str) -> bool {
    let n = normalize(name);
    let tokens = meaningful_tokens(&n);
    n.len() >= 4 && !tokens.is_empty() && !is_generic_name(&n)
}

#[cfg(windows)]
fn belongs_to_installed_app(path: &Path, folder_name: &str, apps: &[InstalledApp]) -> bool {
    let normalized_folder = normalize(folder_name);
    let folder_tokens = meaningful_tokens(&normalized_folder);
    apps.iter().any(|app| {
        if let Some(location) = &app.install_location {
            if !location.as_os_str().is_empty() && path.starts_with(location) {
                return true;
            }
        }
        let name = normalize(&app.name);
        let tokens = meaningful_tokens(&name);
        if name.len() >= 4
            && (name == normalized_folder || normalized_folder == name.replace(' ', ""))
        {
            return true;
        }
        // A single distinctive token is sufficient only when it is long.
        tokens
            .iter()
            .any(|t| t.len() >= 7 && folder_tokens.iter().any(|f| f == t))
            || (tokens.len() > 1
                && folder_tokens.len() > 1
                && tokens
                    .iter()
                    .filter(|t| t.len() >= 4 && folder_tokens.contains(t))
                    .count()
                    >= 2)
    })
}

#[cfg(not(windows))]
fn belongs_to_installed_app(_: &Path, _: &str, _: &[InstalledApp]) -> bool {
    false
}

fn folder_size(root: &Path, overall_deadline: Instant) -> (u64, bool) {
    let started = Instant::now();
    let mut bytes = 0u64;
    for (index, entry) in WalkDir::new(root)
        .max_depth(12)
        .follow_links(false)
        .into_iter()
        .enumerate()
    {
        if index >= MAX_SIZE_ENTRIES_PER_FOLDER
            || started.elapsed() >= MAX_SIZE_TIME_PER_FOLDER
            || Instant::now() >= overall_deadline
        {
            return (bytes, false);
        }
        let Ok(entry) = entry else { continue };
        if entry.file_type().is_symlink()
            || (entry.file_type().is_dir() && is_reparse_point(entry.path()))
        {
            continue;
        }
        if entry.file_type().is_file() {
            if let Ok(metadata) = entry.metadata() {
                bytes = bytes.saturating_add(metadata.len());
            }
        }
    }
    (bytes, true)
}

fn normalize(text: &str) -> String {
    text.chars()
        .flat_map(char::to_lowercase)
        .map(|c| if c.is_ascii_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn meaningful_tokens(normalized: &str) -> Vec<String> {
    const STOP: &[&str] = &[
        "app",
        "apps",
        "application",
        "software",
        "inc",
        "llc",
        "ltd",
        "corp",
        "corporation",
        "company",
        "the",
        "and",
        "for",
        "windows",
        "update",
        "updater",
        "service",
        "runtime",
    ];
    normalized
        .split_whitespace()
        .filter(|t| t.len() >= 4 && !STOP.contains(t) && !is_generic_name(t))
        .map(str::to_owned)
        .collect()
}

fn is_generic_name(name: &str) -> bool {
    matches!(
        name,
        "data"
            | "local"
            | "roaming"
            | "user data"
            | "program data"
            | "shared"
            | "common"
            | "cache"
            | "temp"
            | "temporary"
            | "logs"
            | "packages"
            | "package cache"
            | "microsoft"
            | "windows"
            | "system"
            | "drivers"
            | "runtime"
            | "runtimes"
            | "crashdumps"
            | "diagnostics"
            | "telemetry"
    )
}
