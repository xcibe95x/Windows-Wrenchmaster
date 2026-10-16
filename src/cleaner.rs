//! The Cleaner tab.
//!
//! The 2017 version had a single `Clean` button that called
//! `DeleteFilesFromFolder("C:\")` on a timer tick and swallowed every error.
//! This one enumerates a fixed list of known-safe caches, always measures
//! before it deletes, runs on a worker thread, and defaults to a dry run.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};

use walkdir::WalkDir;

use crate::util::{expand, powershell_wait};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Delete matching files under the given roots, keeping the roots.
    Files,
    /// Hand the job to the shell, which is the only sane way to empty it.
    RecycleBin,
    /// Delete every value under one registry key, keeping the key itself.
    /// Measured in entries rather than bytes.
    RegistryValues,
}

pub struct Target {
    pub name: &'static str,
    pub what: &'static str,
    pub kind: Kind,
    /// Only files with these extensions are touched. Empty means every file.
    pub extensions: &'static [&'static str],
    pub roots: Vec<PathBuf>,
    /// Set for `Kind::RegistryValues`: an HKCU-relative key path.
    pub reg_path: Option<&'static str>,
    pub needs_admin: bool,
    pub selected: bool,
    pub scan: Option<Measure>,
    pub cleaned: Option<Measure>,
    pub errors: usize,
}

#[derive(Clone, Copy, Default)]
pub struct Measure {
    pub bytes: u64,
    pub files: u64,
}

impl Measure {
    fn add_file(&mut self, bytes: u64) {
        self.bytes += bytes;
        self.files += 1;
    }
}

impl Target {
    pub fn available(&self) -> bool {
        match self.kind {
            Kind::RecycleBin => true,
            Kind::RegistryValues => self.reg_path.is_some(),
            Kind::Files => !self.roots.is_empty(),
        }
    }

    /// Registry targets have no meaningful size, only a count.
    pub fn counts_entries(&self) -> bool {
        self.kind == Kind::RegistryValues
    }

    pub fn roots_label(&self) -> String {
        if self.kind == Kind::RecycleBin {
            return "All drives".into();
        }
        if let Some(path) = self.reg_path {
            return format!(r"HKCU\{path}");
        }
        if matches!(
            self.name,
            "Chrome cache" | "Edge cache" | "Brave cache" | "Firefox cache"
        ) {
            return format!("{} cache folders across browser profiles", self.roots.len());
        }
        self.roots
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join("  •  ")
    }
}

/// Build the target list, dropping anything that does not exist on this
/// machine so the UI never offers a button that cannot work.
pub fn targets() -> Vec<Target> {
    let mut list = vec![
        spec(
            "User temporary files",
            "Everything under your per-user TEMP folder.",
            Kind::Files,
            &[],
            &["%TEMP%", "%LOCALAPPDATA%\\Temp"],
            false,
            true,
        ),
        spec(
            "Windows temporary files",
            "The machine-wide scratch folder used by installers and services.",
            Kind::Files,
            &[],
            &["%SystemRoot%\\Temp"],
            true,
            true,
        ),
        spec(
            "Windows Update cache",
            "Downloaded update payloads. Windows re-downloads what it needs.",
            Kind::Files,
            &[],
            &["%SystemRoot%\\SoftwareDistribution\\Download"],
            true,
            true,
        ),
        spec(
            "Component store logs (CBS)",
            "Servicing logs written by DISM, SFC and Windows Update.",
            Kind::Files,
            &["log", "cab", "persist"],
            &["%SystemRoot%\\Logs\\CBS"],
            true,
            true,
        ),
        spec(
            "Windows log files",
            "Rotated *.log files under the Windows logs and debug folders.",
            Kind::Files,
            &["log", "etl", "evtx_bak"],
            &[
                "%SystemRoot%\\Logs",
                "%SystemRoot%\\Debug",
                "%SystemRoot%\\Panther",
                "%SystemRoot%\\SoftwareDistribution\\DataStore\\Logs",
            ],
            true,
            true,
        ),
        spec(
            "Error reports (WER)",
            "Queued and archived Windows Error Reporting payloads.",
            Kind::Files,
            &[],
            &[
                "%ProgramData%\\Microsoft\\Windows\\WER\\ReportQueue",
                "%ProgramData%\\Microsoft\\Windows\\WER\\ReportArchive",
                "%LOCALAPPDATA%\\Microsoft\\Windows\\WER",
            ],
            true,
            true,
        ),
        spec(
            "Crash and memory dumps",
            "Application CrashDumps plus kernel minidumps from blue screens.",
            Kind::Files,
            &["dmp", "mdmp", "hdmp"],
            &[
                "%LOCALAPPDATA%\\CrashDumps",
                "%SystemRoot%\\Minidump",
                "%SystemRoot%",
            ],
            true,
            true,
        ),
        spec(
            "Thumbnail cache",
            "Explorer thumbnail databases. Rebuilt on demand.",
            Kind::Files,
            &["db"],
            &["%LOCALAPPDATA%\\Microsoft\\Windows\\Explorer"],
            false,
            true,
        ),
        spec(
            "Delivery Optimization cache",
            "Peer-to-peer update chunks cached for other PCs on your network.",
            Kind::Files,
            &[],
            &[
                "%SystemRoot%\\ServiceProfiles\\NetworkService\\AppData\\Local\\Microsoft\\Windows\\DeliveryOptimization\\Cache",
            ],
            true,
            false,
        ),
        spec(
            "Prefetch data",
            "Boot and app launch traces. Clearing these slows the next launch.",
            Kind::Files,
            &["pf"],
            &["%SystemRoot%\\Prefetch"],
            true,
            false,
        ),
        spec(
            "Internet Explorer leftovers",
            "Logs and caches left by the IE/WebView shell components.",
            Kind::Files,
            &["log", "dat", "tmp"],
            &[r"%LOCALAPPDATA%\Microsoft\Internet Explorer"],
            false,
            false,
        ),
        registry_spec(
            "MUICache entries",
            "Stale Explorer name cache for programs you have since removed.",
            r"Software\Classes\Local Settings\Software\Microsoft\Windows\Shell\MuiCache",
        ),
        spec(
            "Recycle Bin",
            "Permanently removes everything currently in the bin.",
            Kind::RecycleBin,
            &[],
            &[],
            false,
            false,
        ),
    ];
    list.extend(browser_cache_targets());
    list.retain(|t| t.available());
    list
}

/// Browser cleanup is deliberately limited to disposable cache directories.
/// Password stores, cookies, history, and profile databases are never included.
fn browser_cache_targets() -> Vec<Target> {
    let browsers: [(&str, &str, &str); 4] = [
        (
            "Chrome cache",
            "%LOCALAPPDATA%\\Google\\Chrome\\User Data",
            "chromium",
        ),
        (
            "Edge cache",
            "%LOCALAPPDATA%\\Microsoft\\Edge\\User Data",
            "chromium",
        ),
        (
            "Brave cache",
            "%LOCALAPPDATA%\\BraveSoftware\\Brave-Browser\\User Data",
            "chromium",
        ),
        (
            "Firefox cache",
            "%LOCALAPPDATA%\\Mozilla\\Firefox\\Profiles",
            "firefox",
        ),
    ];
    browsers
        .into_iter()
        .map(|(name, base, layout)| {
            let mut roots = Vec::new();
            if let Some(base) = expand(base) {
                if let Ok(profiles) = std::fs::read_dir(&base) {
                    for profile in profiles.flatten() {
                        let path = profile.path();
                        let Ok(meta) = std::fs::symlink_metadata(&path) else {
                            continue;
                        };
                        if !meta.is_dir() || is_reparse_metadata(&meta) {
                            continue;
                        }
                        if layout == "firefox" {
                            roots.push(path.join("cache2"));
                        } else {
                            for relative in [
                                "Cache\\Cache_Data",
                                "Code Cache",
                                "GPUCache",
                                "Service Worker\\CacheStorage",
                            ] {
                                roots.push(path.join(relative));
                            }
                        }
                    }
                }
            }
            Target {
                name,
                what: "Disposable cache only. Close the browser first. Saved passwords, cookies and history are never touched.",
                kind: Kind::Files,
                extensions: &[],
                roots: roots
                    .into_iter()
                    .filter(|path| {
                        std::fs::symlink_metadata(path)
                            .map(|metadata| metadata.is_dir() && !is_reparse_metadata(&metadata))
                            .unwrap_or(false)
                    })
                    .collect(),
                reg_path: None,
                needs_admin: false,
                selected: false,
                scan: None,
                cleaned: None,
                errors: 0,
            }
        })
        .collect()
}

#[cfg(windows)]
fn is_reparse_metadata(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.file_attributes() & 0x0400 != 0
}

#[cfg(not(windows))]
fn is_reparse_metadata(metadata: &std::fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

#[allow(clippy::too_many_arguments)]
fn spec(
    name: &'static str,
    what: &'static str,
    kind: Kind,
    extensions: &'static [&'static str],
    roots: &[&str],
    needs_admin: bool,
    default_on: bool,
) -> Target {
    let roots: Vec<PathBuf> = roots
        .iter()
        .filter_map(|r| expand(r))
        .filter(|p| p.is_dir())
        .collect();
    Target {
        name,
        what,
        kind,
        extensions,
        roots,
        reg_path: None,
        needs_admin,
        selected: default_on,
        scan: None,
        cleaned: None,
        errors: 0,
    }
}

/// A registry target: no roots, no extensions, HKCU only, never on by default.
fn registry_spec(name: &'static str, what: &'static str, reg_path: &'static str) -> Target {
    Target {
        name,
        what,
        kind: Kind::RegistryValues,
        extensions: &[],
        roots: Vec::new(),
        reg_path: Some(reg_path),
        needs_admin: false,
        selected: false,
        scan: None,
        cleaned: None,
        errors: 0,
    }
}

fn matches(path: &std::path::Path, extensions: &[&str], recursive_root: bool) -> bool {
    if extensions.is_empty() {
        return true;
    }
    // The dump sweep points at %SystemRoot% itself, where only the extension
    // filter keeps us away from the rest of Windows.
    let _ = recursive_root;
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| extensions.iter().any(|w| w.eq_ignore_ascii_case(e)))
        .unwrap_or(false)
}

/// A root that is the Windows directory itself is swept one level deep only:
/// MEMORY.DMP lives there, and nothing below it should be in scope.
fn depth_for(root: &std::path::Path) -> usize {
    match expand("%SystemRoot%") {
        Some(win) if win == root => 1,
        _ => usize::MAX,
    }
}

pub enum Progress {
    Current {
        path: String,
    },
    Measured {
        target: usize,
        measure: Measure,
        errors: usize,
    },
    Removed {
        target: usize,
        measure: Measure,
        errors: usize,
    },
    Finished,
}

pub struct Job {
    pub rx: Receiver<Progress>,
    pub cancel: Arc<AtomicBool>,
    pub cleaning: bool,
    pub current: String,
    pub done: usize,
    pub total: usize,
}

impl Job {
    pub fn fraction(&self) -> f32 {
        if self.total == 0 {
            0.0
        } else {
            self.done as f32 / self.total as f32
        }
    }
}

/// Snapshot of the work to do, so the worker thread never touches the UI state.
struct Unit {
    index: usize,
    kind: Kind,
    extensions: Vec<String>,
    roots: Vec<PathBuf>,
    reg_path: Option<&'static str>,
}

pub fn start(targets: &[Target], clean: bool, dry_run: bool) -> Job {
    let units: Vec<Unit> = targets
        .iter()
        .enumerate()
        .filter(|(_, t)| t.selected)
        .map(|(index, t)| Unit {
            index,
            kind: t.kind,
            extensions: t.extensions.iter().map(|s| s.to_string()).collect(),
            roots: t.roots.clone(),
            reg_path: t.reg_path,
        })
        .collect();

    let total = units.len();
    let (tx, rx) = channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = cancel.clone();

    std::thread::spawn(move || {
        for unit in units {
            if worker_cancel.load(Ordering::Relaxed) {
                break;
            }
            run_unit(&unit, clean && !dry_run, &tx, &worker_cancel);
        }
        let _ = tx.send(Progress::Finished);
    });

    Job {
        rx,
        cancel,
        cleaning: clean,
        current: String::new(),
        done: 0,
        total,
    }
}

fn run_unit(unit: &Unit, delete: bool, tx: &Sender<Progress>, cancel: &AtomicBool) {
    let mut measure = Measure::default();
    let mut errors = 0usize;

    if unit.kind == Kind::RecycleBin {
        if delete {
            let _ = tx.send(Progress::Current {
                path: "Emptying the Recycle Bin".into(),
            });
            if powershell_wait("Clear-RecycleBin -Force -ErrorAction SilentlyContinue")
                .map(|s| !s.success())
                .unwrap_or(true)
            {
                errors += 1;
            }
            let _ = tx.send(Progress::Removed {
                target: unit.index,
                measure,
                errors,
            });
        } else {
            // Sizing the bin means reading every drive; report it as unknown
            // rather than pretending we measured it.
            let _ = tx.send(Progress::Measured {
                target: unit.index,
                measure,
                errors,
            });
        }
        return;
    }

    if unit.kind == Kind::RegistryValues {
        let (measure, errors) = registry_values(unit, delete, tx);
        let progress = if delete {
            Progress::Removed {
                target: unit.index,
                measure,
                errors,
            }
        } else {
            Progress::Measured {
                target: unit.index,
                measure,
                errors,
            }
        };
        let _ = tx.send(progress);
        return;
    }

    let extensions: Vec<&str> = unit.extensions.iter().map(|s| s.as_str()).collect();
    let mut last_report = std::time::Instant::now();

    for root in &unit.roots {
        let walker = WalkDir::new(root)
            .max_depth(depth_for(root))
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| !is_reparse(e));

        for entry in walker {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            let entry = match entry {
                Ok(e) => e,
                Err(_) => {
                    errors += 1;
                    continue;
                }
            };
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.path();
            if !matches(path, &extensions, true) {
                continue;
            }
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);

            if last_report.elapsed().as_millis() > 40 {
                last_report = std::time::Instant::now();
                let _ = tx.send(Progress::Current {
                    path: path.display().to_string(),
                });
            }

            if delete {
                match std::fs::remove_file(path) {
                    Ok(()) => measure.add_file(size),
                    Err(_) => errors += 1,
                }
            } else {
                measure.add_file(size);
            }
        }

        if delete {
            prune_empty_dirs(root);
        }
    }

    let progress = if delete {
        Progress::Removed {
            target: unit.index,
            measure,
            errors,
        }
    } else {
        Progress::Measured {
            target: unit.index,
            measure,
            errors,
        }
    };
    let _ = tx.send(progress);
}

/// Junctions under AppData\Local (the old "Application Data" loops) would
/// otherwise send the walker in circles.
fn is_reparse(entry: &walkdir::DirEntry) -> bool {
    entry.depth() > 0 && entry.path_is_symlink()
}

/// Remove directories left behind by the sweep, never the root itself.
fn prune_empty_dirs(root: &std::path::Path) {
    let mut dirs: Vec<PathBuf> = WalkDir::new(root)
        .min_depth(1)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_dir())
        .map(|e| e.into_path())
        .collect();
    // Deepest first, so a directory tree collapses bottom-up.
    dirs.sort_by_key(|p| std::cmp::Reverse(p.components().count()));
    for dir in dirs {
        let _ = std::fs::remove_dir(dir);
    }
}

/// Count or delete every value under an HKCU key. Entries are reported in
/// `Measure::files`, with `bytes` left at zero: the registry footprint is not
/// disk space and pretending otherwise would inflate the totals.
#[cfg(windows)]
fn registry_values(unit: &Unit, delete: bool, tx: &Sender<Progress>) -> (Measure, usize) {
    use winreg::RegKey;
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};

    let mut measure = Measure::default();
    let mut errors = 0usize;
    let Some(path) = unit.reg_path else {
        return (measure, errors);
    };

    let flags = if delete {
        KEY_READ | KEY_WRITE
    } else {
        KEY_READ
    };
    let Ok(key) = RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(path, flags) else {
        // A missing key is simply nothing to clean.
        return (measure, errors);
    };

    let names: Vec<String> = key
        .enum_values()
        .filter_map(Result::ok)
        .map(|(n, _)| n)
        .collect();
    for name in names {
        // These two are the key's own bookkeeping, not cached program names.
        if name.eq_ignore_ascii_case("LangID") || name.is_empty() {
            continue;
        }
        if delete {
            let _ = tx.send(Progress::Current {
                path: format!(r"HKCU\{path}\{name}"),
            });
            match key.delete_value(&name) {
                Ok(()) => measure.files += 1,
                Err(_) => errors += 1,
            }
        } else {
            measure.files += 1;
        }
    }
    (measure, errors)
}

#[cfg(not(windows))]
fn registry_values(_unit: &Unit, _delete: bool, _tx: &Sender<Progress>) -> (Measure, usize) {
    (Measure::default(), 0)
}
