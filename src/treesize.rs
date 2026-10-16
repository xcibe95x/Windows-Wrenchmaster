//! Bounded, asynchronous folder-size scanning for the Diagnose UI.
//!
//! The scanner reports one node per represented folder. Each node contains
//! aggregate bytes and file/folder counts for its subtree, and `parent` points
//! to an earlier node in `ScanResult::folders` (the selected root is node 0).
//! Symlinks are skipped and never followed.

use std::cmp::{Ordering as CmpOrdering, Reverse};
use std::collections::{BinaryHeap, HashMap};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use walkdir::{DirEntry, WalkDir};

/// Hard bounds on traversal and retained folder rows. Both bounds protect the
/// worker and UI from unexpectedly huge or deeply nested directory trees.
#[derive(Clone, Copy, Debug)]
pub struct ScanLimits {
    /// Maximum filesystem entries inspected, including the root and errors.
    pub max_entries: usize,
    /// Maximum folder nodes kept in the returned tree, including the root.
    pub max_folders: usize,
    /// Maximum directory depth below the selected root.
    pub max_depth: usize,
    /// Maximum inaccessible-path messages retained in the result.
    pub max_error_samples: usize,
}

impl Default for ScanLimits {
    fn default() -> Self {
        Self {
            max_entries: 250_000,
            max_folders: 50_000,
            max_depth: 128,
            max_error_samples: 32,
        }
    }
}

/// One folder in the returned tree. `parent` is an index into
/// `ScanResult::folders`; it is `None` only for the selected root.
#[derive(Clone, Debug)]
pub struct FolderSize {
    pub path: PathBuf,
    pub name: String,
    pub parent: Option<usize>,
    /// Sum of file sizes anywhere in this folder's represented subtree.
    pub bytes: u64,
    /// Number of non-symlink files in the subtree.
    pub file_count: u64,
    /// Number of descendant folders in the subtree (does not include self).
    pub folder_count: u64,
}

/// A large file found during the scan. `parent` refers to the nearest folder
/// represented in `ScanResult::folders`.
#[derive(Clone, Debug)]
pub struct LargestFile {
    pub path: PathBuf,
    pub bytes: u64,
    pub parent: usize,
}

impl PartialEq for LargestFile {
    fn eq(&self, other: &Self) -> bool {
        self.bytes == other.bytes && self.path == other.path && self.parent == other.parent
    }
}

impl Eq for LargestFile {}

impl PartialOrd for LargestFile {
    fn partial_cmp(&self, other: &Self) -> Option<CmpOrdering> {
        Some(self.cmp(other))
    }
}

impl Ord for LargestFile {
    fn cmp(&self, other: &Self) -> CmpOrdering {
        self.bytes
            .cmp(&other.bytes)
            .then_with(|| self.path.cmp(&other.path))
            .then_with(|| self.parent.cmp(&other.parent))
    }
}

const LARGEST_FILES_LIMIT: usize = 200;

#[derive(Clone, Debug, Default)]
pub struct ScanProgress {
    pub visited_entries: usize,
    pub files: u64,
    pub folders: u64,
    pub bytes: u64,
    pub inaccessible: u64,
    pub skipped_symlinks: u64,
}

#[derive(Clone, Debug)]
pub struct ScanResult {
    pub root: PathBuf,
    /// Parent-indexed folder tree, with the selected root at index 0.
    pub folders: Vec<FolderSize>,
    /// Child folder indices grouped by parent, sorted largest-first once scan completes.
    pub children: Vec<Vec<usize>>,
    /// Up to 200 largest files, sorted largest first.
    pub largest_files: Vec<LargestFile>,
    pub progress: ScanProgress,
    /// True when `max_entries` or `max_folders` stopped full representation.
    pub capped: bool,
    /// True when cancellation stopped traversal before exhaustion.
    pub cancelled: bool,
    /// True when one or more folders reached `max_depth` and were not descended.
    pub depth_limited: bool,
    /// A bounded sample of inaccessible paths and their errors.
    pub errors: Vec<String>,
    /// Number of additional errors not included in `errors`.
    pub omitted_errors: u64,
}

#[derive(Clone, Debug)]
pub enum ScanError {
    NotDirectory(PathBuf),
    RootUnavailable(String),
}

impl fmt::Display for ScanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotDirectory(path) => write!(f, "{} is not a folder", path.display()),
            Self::RootUnavailable(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for ScanError {}

/// Messages delivered by the worker. Progress is sent periodically; exactly
/// one `Finished` message is sent unless the receiver has been dropped.
#[derive(Debug)]
pub enum ScanMessage {
    Progress(ScanProgress),
    Finished(Result<ScanResult, ScanError>),
}

/// A running scan. Poll `messages` from the UI thread and call `cancel()` when
/// the user stops the scan. Dropping the handle requests cancellation.
pub struct ScanHandle {
    pub messages: Receiver<ScanMessage>,
    cancel: Arc<AtomicBool>,
}

impl ScanHandle {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

impl Drop for ScanHandle {
    fn drop(&mut self) {
        self.cancel();
    }
}

/// Start scanning `root` on a dedicated worker thread.
pub fn start_scan(root: PathBuf) -> ScanHandle {
    start_scan_with_limits(root, ScanLimits::default())
}

/// Start scanning with explicit resource limits. Use this for a smaller cap in
/// constrained environments or to tune the progress cadence for a UI.
pub fn start_scan_with_limits(root: PathBuf, limits: ScanLimits) -> ScanHandle {
    let (tx, messages) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = Arc::clone(&cancel);
    std::thread::Builder::new()
        .name("folder-size-scan".into())
        .spawn(move || {
            let result = scan(root, limits, &worker_cancel, &tx);
            let _ = tx.send(ScanMessage::Finished(result));
        })
        .expect("failed to start folder size scan worker");
    ScanHandle { messages, cancel }
}

fn scan(
    root: PathBuf,
    limits: ScanLimits,
    cancel: &AtomicBool,
    tx: &Sender<ScanMessage>,
) -> Result<ScanResult, ScanError> {
    let root_meta = std::fs::symlink_metadata(&root)
        .map_err(|e| ScanError::RootUnavailable(format!("Cannot open {}: {e}", root.display())))?;
    if is_reparse_metadata(&root_meta) || !root_meta.is_dir() {
        return Err(ScanError::NotDirectory(root));
    }

    let root_name = root
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| root.display().to_string());
    let folders = vec![FolderSize {
        path: root.clone(),
        name: root_name,
        parent: None,
        bytes: 0,
        file_count: 0,
        folder_count: 0,
    }];
    let mut by_path = HashMap::new();
    by_path.insert(root.clone(), 0usize);

    let mut result = ScanResult {
        root: root.clone(),
        folders,
        children: Vec::new(),
        largest_files: Vec::new(),
        progress: ScanProgress::default(),
        capped: false,
        cancelled: false,
        depth_limited: false,
        errors: Vec::new(),
        omitted_errors: 0,
    };
    let mut last_report = Instant::now();
    // A min-heap retains only the largest N entries seen so far.
    let mut largest_files: BinaryHeap<Reverse<LargestFile>> = BinaryHeap::new();

    // follow_links(false) keeps traversal inside the chosen tree and prevents
    // cycles. Symlink entries remain visible so they can be counted as skipped.
    let walker = WalkDir::new(&root)
        .follow_links(false)
        .max_depth(limits.max_depth)
        .into_iter();

    for item in walker {
        if cancel.load(Ordering::Relaxed) {
            result.cancelled = true;
            break;
        }
        if result.progress.visited_entries >= limits.max_entries {
            result.capped = true;
            break;
        }
        result.progress.visited_entries += 1;

        let entry = match item {
            Ok(entry) => entry,
            Err(error) => {
                record_error(&mut result, error.to_string(), limits.max_error_samples);
                continue;
            }
        };

        if is_symlink(&entry) {
            result.progress.skipped_symlinks += 1;
            continue;
        }

        let depth = entry.depth();
        if entry.file_type().is_dir() {
            if depth >= limits.max_depth {
                result.depth_limited = true;
            }
            if depth == 0 {
                continue;
            }

            result.progress.folders += 1;
            let parent_path = entry.path().parent().unwrap_or(&root);
            let parent = nearest_folder(parent_path, &by_path, &root);
            add_direct_folder(&mut result.folders, parent);

            if result.folders.len() >= limits.max_folders {
                result.capped = true;
                continue;
            }

            let index = result.folders.len();
            let path = entry.path().to_path_buf();
            let name = entry.file_name().to_string_lossy().into_owned();
            result.folders.push(FolderSize {
                path: path.clone(),
                name,
                parent: Some(parent),
                bytes: 0,
                file_count: 0,
                folder_count: 0,
            });
            by_path.insert(path, index);
        } else if entry.file_type().is_file() {
            match entry.metadata() {
                Ok(metadata) => {
                    let size = metadata.len();
                    result.progress.files += 1;
                    result.progress.bytes = result.progress.bytes.saturating_add(size);
                    let parent_path = entry.path().parent().unwrap_or(&root);
                    let parent = nearest_folder(parent_path, &by_path, &root);
                    add_direct_file(&mut result.folders, parent, size);
                    retain_largest_file(
                        &mut largest_files,
                        LargestFile {
                            path: entry.path().to_path_buf(),
                            bytes: size,
                            parent,
                        },
                    );
                }
                Err(error) => record_error(
                    &mut result,
                    format!("{}: {error}", entry.path().display()),
                    limits.max_error_samples,
                ),
            }
        }

        if result.progress.visited_entries % 256 == 0
            || last_report.elapsed() >= Duration::from_millis(200)
        {
            let _ = tx.send(ScanMessage::Progress(result.progress.clone()));
            last_report = Instant::now();
        }
    }

    result.largest_files = largest_files
        .into_iter()
        .map(|Reverse(file)| file)
        .collect();
    result
        .largest_files
        .sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.path.cmp(&b.path)));
    roll_up_folder_totals(&mut result.folders);
    result.children = vec![Vec::new(); result.folders.len()];
    for (index, folder) in result.folders.iter().enumerate().skip(1) {
        if let Some(parent) = folder.parent {
            if let Some(siblings) = result.children.get_mut(parent) {
                siblings.push(index);
            }
        }
    }
    for siblings in &mut result.children {
        siblings.sort_unstable_by(|a, b| {
            result.folders[*b]
                .bytes
                .cmp(&result.folders[*a].bytes)
                .then_with(|| {
                    result.folders[*a]
                        .name
                        .to_lowercase()
                        .cmp(&result.folders[*b].name.to_lowercase())
                })
        });
    }
    let _ = tx.send(ScanMessage::Progress(result.progress.clone()));
    Ok(result)
}

fn retain_largest_file(heap: &mut BinaryHeap<Reverse<LargestFile>>, file: LargestFile) {
    if heap.len() < LARGEST_FILES_LIMIT {
        heap.push(Reverse(file));
        return;
    }
    let should_replace = heap
        .peek()
        .is_some_and(|Reverse(smallest)| file.bytes > smallest.bytes);
    if should_replace {
        heap.pop();
        heap.push(Reverse(file));
    }
}

fn nearest_folder(path: &Path, by_path: &HashMap<PathBuf, usize>, root: &Path) -> usize {
    let mut current = Some(path);
    while let Some(candidate) = current {
        if let Some(index) = by_path.get(candidate) {
            return *index;
        }
        if candidate == root {
            break;
        }
        current = candidate.parent();
    }
    0
}

fn add_direct_file(folders: &mut [FolderSize], index: usize, bytes: u64) {
    folders[index].bytes = folders[index].bytes.saturating_add(bytes);
    folders[index].file_count = folders[index].file_count.saturating_add(1);
}

fn add_direct_folder(folders: &mut [FolderSize], index: usize) {
    folders[index].folder_count = folders[index].folder_count.saturating_add(1);
}

/// Folder indices are created by a preorder walk, so parents always precede
/// children. One reverse pass aggregates subtree totals in O(folder count),
/// instead of walking the full depth for every scanned file and directory.
fn roll_up_folder_totals(folders: &mut [FolderSize]) {
    for index in (1..folders.len()).rev() {
        let Some(parent) = folders[index].parent else {
            continue;
        };
        let bytes = folders[index].bytes;
        let files = folders[index].file_count;
        let descendants = folders[index].folder_count;
        if let Some(parent) = folders.get_mut(parent) {
            parent.bytes = parent.bytes.saturating_add(bytes);
            parent.file_count = parent.file_count.saturating_add(files);
            parent.folder_count = parent.folder_count.saturating_add(descendants);
        }
    }
}

fn record_error(result: &mut ScanResult, message: String, sample_limit: usize) {
    result.progress.inaccessible = result.progress.inaccessible.saturating_add(1);
    if result.errors.len() < sample_limit {
        result.errors.push(message);
    } else {
        result.omitted_errors = result.omitted_errors.saturating_add(1);
    }
}

fn is_symlink(entry: &DirEntry) -> bool {
    entry.file_type().is_symlink()
        || std::fs::symlink_metadata(entry.path())
            .map(|metadata| is_reparse_metadata(&metadata))
            .unwrap_or(false)
}

#[cfg(windows)]
fn is_reparse_metadata(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_reparse_metadata(metadata: &std::fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}
