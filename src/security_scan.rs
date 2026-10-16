//! Read-only heuristic scan for common token-grabber script patterns.
//!
//! This deliberately reads only source-like files from common user-writable
//! locations. It never opens browser credential stores, reads token databases,
//! removes files, or sends any data over the network. Microsoft Defender is
//! used separately for broad malware detection.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use walkdir::{DirEntry, WalkDir};

const MAX_FILES: usize = 25_000;
const MAX_ENTRIES: usize = 100_000;
const MAX_TOTAL_BYTES: u64 = 128 * 1024 * 1024;
const MAX_FILE_BYTES: u64 = 1024 * 1024;
const MAX_DEPTH: usize = 7;
const MAX_DURATION: Duration = Duration::from_secs(30);

#[derive(Clone, Debug)]
pub struct Finding {
    pub path: PathBuf,
    pub reason: String,
}

#[derive(Clone, Debug, Default)]
pub struct ScanReport {
    pub files_scanned: usize,
    pub locations_checked: usize,
    pub unreadable: usize,
    pub skipped_large: usize,
    pub limit_reached: bool,
    pub findings: Vec<Finding>,
}

pub fn start_token_scan() -> Receiver<ScanReport> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(scan_token_grabbers());
    });
    receiver
}

pub fn scan_token_grabbers() -> ScanReport {
    let roots = scan_roots();
    let mut report = ScanReport {
        ..Default::default()
    };
    let started = Instant::now();
    let mut entries_seen = 0usize;
    let mut bytes_read = 0u64;

    'roots: for root in roots {
        report.locations_checked += 1;
        for entry in WalkDir::new(&root)
            .max_depth(MAX_DEPTH)
            .follow_links(false)
            .into_iter()
            .filter_entry(should_visit)
        {
            entries_seen += 1;
            if entries_seen >= MAX_ENTRIES
                || report.files_scanned >= MAX_FILES
                || bytes_read >= MAX_TOTAL_BYTES
                || started.elapsed() >= MAX_DURATION
            {
                report.limit_reached = true;
                break 'roots;
            }
            let Ok(entry) = entry else {
                report.unreadable += 1;
                continue;
            };
            if !entry.file_type().is_file() || !is_source_file(entry.path()) {
                continue;
            }
            let Ok(metadata) = entry.metadata() else {
                report.unreadable += 1;
                continue;
            };
            if metadata.len() > MAX_FILE_BYTES {
                report.skipped_large += 1;
                continue;
            }
            if bytes_read.saturating_add(metadata.len()) > MAX_TOTAL_BYTES {
                report.limit_reached = true;
                break 'roots;
            }
            report.files_scanned += 1;
            let Ok(bytes) = std::fs::read(entry.path()) else {
                report.unreadable += 1;
                continue;
            };
            bytes_read += bytes.len() as u64;

            let text = String::from_utf8_lossy(&bytes).to_ascii_lowercase();
            if let Some(reason) = token_grabber_indicators(&text) {
                report.findings.push(Finding {
                    path: entry.path().to_path_buf(),
                    reason,
                });
            }
        }
    }

    report
}

fn scan_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(roaming) = std::env::var_os("APPDATA").map(PathBuf::from) {
        push_existing(
            &mut roots,
            roaming.join("Microsoft\\Windows\\Start Menu\\Programs\\Startup"),
        );
    }
    if let Some(profile) = std::env::var_os("USERPROFILE").map(PathBuf::from) {
        push_existing(&mut roots, profile.join("Downloads"));
    }
    if let Some(roaming) = std::env::var_os("APPDATA").map(PathBuf::from) {
        for relative in ["Discord", "discordcanary", "discordptb", "BetterDiscord"] {
            push_existing(&mut roots, roaming.join(relative));
        }
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) {
        for relative in [
            "Google\\Chrome\\User Data\\Default\\Extensions",
            "Microsoft\\Edge\\User Data\\Default\\Extensions",
            "Temp",
        ] {
            push_existing(&mut roots, local.join(relative));
        }
    }
    roots
}

fn push_existing(roots: &mut Vec<PathBuf>, path: PathBuf) {
    if path.is_dir() && !roots.iter().any(|existing| existing == &path) {
        roots.push(path);
    }
}

fn should_visit(entry: &DirEntry) -> bool {
    if !entry.file_type().is_dir() {
        return true;
    }
    let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
    !matches!(
        name.as_str(),
        ".git"
            | "node_modules"
            | "cache"
            | "code cache"
            | "gpucache"
            | "blob_storage"
            | "leveldb"
            | "service worker"
    )
}

fn is_source_file(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some(
            "js" | "jse"
                | "mjs"
                | "cjs"
                | "ps1"
                | "psm1"
                | "vbs"
                | "py"
                | "bat"
                | "cmd"
                | "hta"
                | "html"
        )
    )
}

fn token_grabber_indicators(text: &str) -> Option<String> {
    let discord_store = text.contains("leveldb")
        && (text.contains("discord") || text.contains("local state"))
        && ["readdir", "readfile", "read_file", "glob(", ".ldb"]
            .iter()
            .any(|needle| text.contains(needle));
    let browser_credentials = ["login data", "cookies", "local state"]
        .iter()
        .any(|needle| text.contains(needle))
        && ["sqlite", "copyfile", "readfile", "read_file", "chromium"]
            .iter()
            .any(|needle| text.contains(needle));
    let token_handling = ["token", "authorization", "document.cookie"]
        .iter()
        .any(|needle| text.contains(needle))
        && ["match(", "regex", "split(", "replace(", "decrypt"]
            .iter()
            .any(|needle| text.contains(needle));
    let exfiltration = [
        "webhook",
        "axios.post",
        "fetch(",
        "request.post",
        "http.post",
    ]
    .iter()
    .any(|needle| text.contains(needle));

    if ((discord_store || browser_credentials) && token_handling && exfiltration)
        || (discord_store && exfiltration && text.contains("token"))
    {
        Some(
            "Possible credential-store access combined with token handling and outbound requests"
                .into(),
        )
    } else {
        None
    }
}
