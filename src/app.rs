//! The window: a sidebar, one page per job, and a status bar that tells you
//! what just happened instead of a MsgBox that tells you nothing.

use std::path::PathBuf;
use std::process::Command;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

use egui::{
    Align, Button, CentralPanel, Color32, Id, Label, Layout, Margin, Modal, Panel, RichText, Ui,
};

use crate::chrome::{self, Pointer};
use crate::cleaner::{self, Measure, Progress};
use crate::diagnose::{self, EventFinding, ScanResult as EventScanResult, ScanStatus};
use crate::health::{self, Health};
use crate::history::History;
use crate::machine::{ActivationStatus, Machine};
use crate::malware_lookup::{self, LookupResult, LookupStatus, RecentFeedScanResult};
use crate::network_watch::{
    self, Direction as NetworkDirection, ScanResult as NetworkScanResult,
    Severity as NetworkSeverity,
};
use crate::ram::{self, MemorySnapshot, ProcessMemory};
use crate::security_integrity;
use crate::security_scan::{self, ScanReport};
use crate::settings::{Mode, Settings};
use crate::theme::{self, ACCENT, FAINT, MUTED};
use crate::tools::{self, Action};
use crate::treesize::{self, ScanHandle, ScanMessage, ScanProgress, ScanResult};
use crate::tweaks::{self, State, Tweak};
use crate::util::{
    human_bytes, is_development_mode, is_elevated, plural, relaunch_elevated,
    run_activation_scripts,
};
use crate::{app_residue, registry_audit, startup};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Overview,
    Diagnose,
    SystemInfo,
    Hardware,
    Memory,
    Storage,
    Tweaks,
    Security,
    Privacy,
    Troubleshoot,
    Repair,
    Startup,
    Tools,
    Backup,
    Settings,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum StorageView {
    Summary,
    Cleanup,
    SizeExplorer,
    AppLeftovers,
}

impl StorageView {
    /// Companion to Tab::from_env, so a Storage sub-view can be opened
    /// directly while developing. Development builds only.
    fn from_env() -> Self {
        if !crate::util::is_development_mode() {
            return Self::Summary;
        }
        match std::env::var("WRENCHMASTER_VIEW")
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "cleanup" => Self::Cleanup,
            "size" | "sizeexplorer" => Self::SizeExplorer,
            "leftovers" | "appleftovers" => Self::AppLeftovers,
            _ => Self::Summary,
        }
    }
}

/// Which of the three ways of naming an MCP server the settings page is
/// showing.
///
/// Claude and OpenAI are not providers being called here; they are the two
/// programs on this machine that already keep a list of local MCP servers.
/// Custom is the pair of text fields this section used to be, and stays the
/// answer whenever neither list has anything in it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum AiSource {
    Claude,
    OpenAi,
    Custom,
}

/// The panes of the Settings page.
///
/// About used to be a tab of its own in the sidebar, which gave a version
/// number and a three-line changelog the same standing as the cleaner. It is
/// something you read once, so it belongs behind the same switcher as the
/// rest of the things you set up once.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SettingsView {
    Appearance,
    Connections,
    About,
}

impl SettingsView {
    const ALL: [SettingsView; 3] = [
        SettingsView::Appearance,
        SettingsView::Connections,
        SettingsView::About,
    ];

    fn label(self) -> &'static str {
        match self {
            SettingsView::Appearance => "Appearance",
            SettingsView::Connections => "AI connection",
            SettingsView::About => "About",
        }
    }
}

impl AiSource {
    fn label(self) -> &'static str {
        match self {
            AiSource::Claude => "Claude",
            AiSource::OpenAi => "OpenAI",
            AiSource::Custom => "Custom",
        }
    }

    /// The discovery source this choice lists, or `None` for Custom, which
    /// lists nothing and asks instead.
    fn discovery(self) -> Option<crate::ai_config::Source> {
        match self {
            AiSource::Claude => Some(crate::ai_config::Source::ClaudeDesktop),
            AiSource::OpenAi => Some(crate::ai_config::Source::OpenAiCodex),
            AiSource::Custom => None,
        }
    }
}

impl Tab {
    const ALL: [Tab; 12] = [
        Tab::Overview,
        Tab::Diagnose,
        Tab::Hardware,
        Tab::Memory,
        Tab::Storage,
        Tab::Tweaks,
        Tab::Security,
        Tab::Privacy,
        Tab::Tools,
        Tab::Backup,
        Tab::SystemInfo,
        Tab::Settings,
    ];

    /// Open straight onto a page, for looking at one while developing.
    /// Development builds only, so it cannot affect a shipped run.
    fn from_env() -> Option<Self> {
        if !crate::util::is_development_mode() {
            return None;
        }
        let want = std::env::var("WRENCHMASTER_TAB").ok()?;
        Self::ALL
            .into_iter()
            .find(|tab| tab.title().eq_ignore_ascii_case(want.trim()))
    }

    fn title(self) -> &'static str {
        match self {
            Tab::Overview => "Overview",
            Tab::Diagnose => "Diagnose",
            Tab::SystemInfo => "System Info",
            Tab::Hardware => "Hardware",
            Tab::Memory => "Memory",
            Tab::Storage => "Storage",
            Tab::Tweaks => "Tweaks",
            Tab::Privacy => "Privacy",
            Tab::Troubleshoot => "Troubleshoot",
            Tab::Security => "Security",
            Tab::Repair => "Repair",
            Tab::Startup => "Startup",
            Tab::Tools => "Tools",
            Tab::Backup => "Backup",
            Tab::Settings => "Settings",
        }
    }

    fn blurb(self) -> &'static str {
        match self {
            Tab::Overview => "Current system health, performance, and space.",
            Tab::Diagnose => {
                "Read-only checks of load, memory, drives, devices and Windows events."
            }
            Tab::SystemInfo => "Windows edition, activation, computer identity and uptime.",
            Tab::Hardware => "Processor, graphics and board, with live sensors where available.",
            Tab::Memory => {
                "Everything about RAM: live use, what is holding it, and how to free it."
            }
            Tab::Storage => "Drives, cleanup, folder sizes and app leftovers.",
            Tab::Tweaks => "Registry switches, each one reversible from the same row.",
            Tab::Privacy => "Review privacy and Windows sharing controls one setting at a time.",
            Tab::Troubleshoot => "Windows diagnostics and guided troubleshooting options.",
            Tab::Security => "Scan for threats and review Windows protection.",
            Tab::Repair => "Commands that change the system. Each one says what it does first.",
            Tab::Startup => "Review common startup entries and whether their commands resolve.",
            Tab::Tools => "Read-only places to look when something is wrong.",
            Tab::Backup => "Export the registry and drop a restore point before you tweak.",
            Tab::Settings => "Appearance, the AI server connection, and what this build is.",
        }
    }
}

/// Which long-running backup task is in flight.
///
/// Both of these used to run inline on the UI thread. A full `reg export` of
/// HKLM takes thirty seconds to two minutes and `Checkpoint-Computer` takes
/// twenty to ninety, so the window stopped painting, Windows greyed it out
/// and marked it Not Responding, and the obvious move was to kill the app --
/// in the middle of writing the registry backup the user had just asked for.
#[derive(Clone, Copy, PartialEq, Eq)]
enum BackupJob {
    Export,
    RestorePoint,
}

impl BackupJob {
    fn running_text(self) -> &'static str {
        match self {
            BackupJob::Export => "Exporting the registry hives\u{2026}",
            BackupJob::RestorePoint => "Creating a restore point\u{2026}",
        }
    }
}

/// Where the support button goes. The only URL the app contains.
const PATREON_URL: &str = "https://www.patreon.com/aboveconstraints";

struct Toast {
    text: String,
    error: bool,
    at: Instant,
}

#[derive(Clone, Debug, Default)]
struct GpuTelemetry {
    name: String,
    temperature_c: Option<String>,
    fan_percent: Option<String>,
    power_w: Option<String>,
    power_limit_w: Option<String>,
}

#[derive(Clone, Debug)]
struct HardwareSensorReading {
    hardware_type: String,
    hardware_name: String,
    parent_type: String,
    parent_name: String,
    sensor_type: String,
    sensor_name: String,
    value: String,
    unit: String,
}

struct SensorQueryResult {
    gpu: Result<Vec<GpuTelemetry>, String>,
    hardware: Result<Vec<HardwareSensorReading>, String>,
}

pub struct App {
    tab: Tab,
    storage_view: StorageView,
    settings_view: SettingsView,
    // One sidebar group is open at a time, the same rule the page sections
    // follow. Four open groups push the lower entries off a short window,
    // which is the bug the nav list grew its own scrollbar to paper over.
    // `open_group` is set through `expand_group`; the per-group bools are
    // derived from it so no call site can set two of them.
    storage_expanded: bool,
    hardware_expanded: bool,
    security_expanded: bool,
    tools_expanded: bool,
    machine: Machine,
    machine_refreshed: Instant,
    gpu_telemetry: Vec<GpuTelemetry>,
    gpu_sensor_error: Option<String>,
    hardware_telemetry: Vec<HardwareSensorReading>,
    history: History,
    wordmark: Option<(bool, egui::TextureHandle)>,
    chart_window: usize,
    hardware_sensor_error: Option<String>,
    gpu_sensor_rx: Option<Receiver<SensorQueryResult>>,
    gpu_sensor_requested: Instant,
    cpu_monitor: sysinfo::System,
    cpu_usage: f32,
    ram_snapshot: Option<MemorySnapshot>,
    ram_error: Option<String>,
    ram_processes: Vec<ProcessMemory>,
    pending_ram_trim: Option<ProcessMemory>,
    pending_ram_action: Option<ram::RamMapAction>,
    size_root: Option<PathBuf>,
    size_scan: Option<ScanHandle>,
    size_progress: ScanProgress,
    size_result: Option<ScanResult>,
    size_show_files: bool,
    size_selected_folder: usize,
    cpu_refreshed: Instant,
    targets: Vec<cleaner::Target>,
    job: Option<cleaner::Job>,
    dry_run: bool,
    last_run_was_dry: bool,
    confirm_clean: bool,
    /// A destructive repair waiting to be confirmed, and the activation
    /// script if that is what is pending.
    confirm_action: Option<Action>,
    confirm_activation: bool,
    tweaks: Vec<Tweak>,
    troubleshooters: Vec<Action>,
    security_actions: Vec<Action>,
    repairs: Vec<Action>,
    utilities: Vec<Action>,
    diagcabs: Vec<PathBuf>,
    token_scan_rx: Option<Receiver<ScanReport>>,
    token_scan_report: Option<ScanReport>,
    diagnose_scan_rx: Option<Receiver<diagnose::CheckMessage>>,
    diagnose_running_check: Option<diagnose::CheckId>,
    diagnose_outcomes: Vec<diagnose::CheckOutcome>,
    diagnose_event_result: Option<EventScanResult>,
    /// How long each finished check actually took, in the order they finished.
    /// Timed by the worker around the check itself, so the figure the stepper
    /// shows is the work, not the delay before the window drained the channel.
    diagnose_durations: Vec<(diagnose::CheckId, Duration)>,
    /// The silent startup probe, kept entirely apart from the diagnostic.
    ///
    /// Sharing `diagnose_outcomes` meant the Diagnose page filled itself in
    /// and ran its stepper on launch, which is the pipeline running in front
    /// of a user who never asked for it. These results only feed the score.
    probe_rx: Option<Receiver<diagnose::CheckMessage>>,
    probe_outcomes: Vec<diagnose::CheckOutcome>,
    probe_durations: Vec<(diagnose::CheckId, Duration)>,
    /// Whether Diagnose is showing the short answer or the whole report.
    ///
    /// Starts on the short answer. Someone who wants the evidence will find
    /// one switch at the top of the page; someone who wanted to know whether
    /// their machine is alright should not have to scroll past seventeen
    /// check rows to find out that it is.
    diagnose_simple: bool,
    app_residue_rx: Option<Receiver<app_residue::ScanResult>>,
    app_residue_result: Option<app_residue::ScanResult>,
    registry_audit_rx: Option<Receiver<registry_audit::AuditReport>>,
    registry_audit_result: Option<registry_audit::AuditReport>,
    startup_scan_rx: Option<Receiver<startup::StartupInventory>>,
    startup_scan_result: Option<startup::StartupInventory>,
    explorer_integrity_rx: Option<Receiver<security_integrity::IntegrityResult>>,
    explorer_integrity_result: Option<security_integrity::IntegrityResult>,
    malware_lookup_rx: Option<Receiver<Result<LookupResult, String>>>,
    malware_lookup_result: Option<Result<LookupResult, String>>,
    malware_lookup_key: String,
    malware_lookup_path: Option<PathBuf>,
    malware_feed_scan_rx: Option<Receiver<Result<RecentFeedScanResult, String>>>,
    malware_feed_scan_result: Option<Result<RecentFeedScanResult, String>>,
    malware_feed_scan_folder: Option<PathBuf>,
    network_scan_rx: Option<Receiver<NetworkScanResult>>,
    network_scan_result: Option<NetworkScanResult>,
    elevated: bool,
    settings: Settings,
    appearance: crate::appearance::Appearance,
    health_cache_refreshed: Instant,
    accent: Color32,
    toast: Option<Toast>,
    backups: Vec<PathBuf>,
    /// The running registry export or restore point, and which of the two it
    /// is so the status line can name it.
    backup_job: Option<(BackupJob, Receiver<Result<Vec<PathBuf>, String>>)>,
    mcp_program: String,
    mcp_args: String,
    mcp_connection: Option<crate::mcp_client::Connection>,
    mcp_status: String,
    mcp_server: Option<(String, String)>,
    mcp_tools: Vec<crate::mcp_client::ToolInfo>,
    ai_source: AiSource,
    /// Filled on the first frame the AI section is drawn and then kept,
    /// because discovery reads two files off disk and the answer does not
    /// change while the window is open.
    ai_discovered: Option<Vec<crate::ai_config::Discovered>>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let machine = Machine::collect();
        // The overview only needs per-process memory here; avoid refreshing
        // every sysinfo collector during startup.
        let mut cpu_monitor = sysinfo::System::new();
        cpu_monitor.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
        let ram_snapshot = ram::memory_snapshot().ok();
        let ram_error = ram::memory_snapshot().err();
        let ram_processes = ram::top_processes(&cpu_monitor);
        let mut settings = Settings::load();
        // Development override, so the light theme can be opened directly
        // without clicking the switch on every run.
        if crate::util::is_development_mode() && std::env::var("WRENCHMASTER_LIGHT").is_ok() {
            settings.mode = Mode::Light;
        }
        chrome::install_fonts(&cc.egui_ctx);
        let [r, g, b] = machine.era.accent();
        let default_accent = Color32::from_rgb(r, g, b);
        let appearance = crate::appearance::Appearance::load(default_accent);
        let accent = appearance.accent();
        appearance.apply(&cc.egui_ctx, &settings, accent);

        // Restore the target ticks the user left behind last time.
        let mut targets = cleaner::targets();
        for target in &mut targets {
            if settings.deselected.contains(target.name) {
                target.selected = false;
            }
        }

        let mut app = Self {
            tab: Tab::from_env().unwrap_or(Tab::Overview),
            storage_view: StorageView::from_env(),
            settings_view: SettingsView::Appearance,
            storage_expanded: false,
            hardware_expanded: false,
            security_expanded: false,
            tools_expanded: false,
            machine,
            machine_refreshed: Instant::now(),
            gpu_telemetry: Vec::new(),
            gpu_sensor_error: None,
            hardware_telemetry: Vec::new(),
            history: History::default(),
            wordmark: None,
            chart_window: 1,
            hardware_sensor_error: Some("Sensor provider is starting.".into()),
            gpu_sensor_rx: None,
            gpu_sensor_requested: Instant::now() - Duration::from_secs(30),
            cpu_monitor,
            cpu_usage: 0.0,
            ram_snapshot,
            ram_error,
            ram_processes,
            pending_ram_trim: None,
            pending_ram_action: None,
            size_root: None,
            size_scan: None,
            size_progress: ScanProgress::default(),
            size_result: None,
            size_show_files: false,
            size_selected_folder: 0,
            cpu_refreshed: Instant::now(),
            targets,
            job: None,
            dry_run: settings.dry_run,
            last_run_was_dry: true,
            confirm_clean: false,
            confirm_action: None,
            confirm_activation: false,
            tweaks: tweaks::all(),
            troubleshooters: tools::troubleshooters(),
            security_actions: tools::security_actions(),
            repairs: tools::repairs(),
            utilities: tools::utilities(),
            diagcabs: tools::local_diagcabs(),
            token_scan_rx: None,
            token_scan_report: None,
            diagnose_scan_rx: None,
            diagnose_running_check: None,
            diagnose_outcomes: Vec::new(),
            diagnose_event_result: None,
            diagnose_durations: Vec::new(),
            probe_rx: None,
            probe_outcomes: Vec::new(),
            probe_durations: Vec::new(),
            diagnose_simple: true,
            app_residue_rx: None,
            app_residue_result: None,
            registry_audit_rx: None,
            registry_audit_result: None,
            startup_scan_rx: None,
            startup_scan_result: None,
            explorer_integrity_rx: None,
            explorer_integrity_result: None,
            malware_lookup_rx: None,
            malware_lookup_result: None,
            malware_lookup_key: String::new(),
            malware_lookup_path: None,
            malware_feed_scan_rx: None,
            malware_feed_scan_result: None,
            malware_feed_scan_folder: None,
            network_scan_rx: None,
            network_scan_result: None,
            elevated: is_elevated(),
            settings,
            appearance,
            health_cache_refreshed: Instant::now(),
            accent,
            toast: None,
            backups: Vec::new(),
            backup_job: None,
            mcp_program: String::new(),
            mcp_args: String::new(),
            mcp_connection: None,
            mcp_status: "Not connected".into(),
            mcp_server: None,
            mcp_tools: Vec::new(),
            ai_source: AiSource::Claude,
            ai_discovered: None,
        };
        // A quick, quiet pass so the health score has something real behind
        // it shortly after the window opens. It is the cheap checks only --
        // no DISM, no update history, no crash log -- and it says nothing in
        // the status bar: nobody asked for it, so it should not announce
        // itself. Until it lands the score reads a neutral 50 rather than
        // pretending to a verdict. The full sweep runs when the user asks.
        app.start_quick_diagnosis_if_stale();
        app
    }

    /// Mirror the live UI state into the preferences file.
    fn remember(&mut self) {
        self.settings.dry_run = self.dry_run;
        self.settings.deselected = self
            .targets
            .iter()
            .filter(|t| !t.selected)
            .map(|t| t.name.to_string())
            .collect();
        self.settings.save();
    }

    fn say(&mut self, text: impl Into<String>) {
        self.toast = Some(Toast {
            text: text.into(),
            error: false,
            at: Instant::now(),
        });
    }

    fn complain(&mut self, text: impl Into<String>) {
        self.toast = Some(Toast {
            text: text.into(),
            error: true,
            at: Instant::now(),
        });
    }

    fn selected_count(&self) -> usize {
        self.targets.iter().filter(|t| t.selected).count()
    }

    fn total<F>(&self, pick: F) -> Measure
    where
        F: Fn(&cleaner::Target) -> Option<Measure>,
    {
        self.targets
            .iter()
            .filter_map(pick)
            .fold(Measure::default(), |mut acc, m| {
                acc.bytes += m.bytes;
                acc.files += m.files;
                acc
            })
    }

    fn scanned_total(&self) -> Measure {
        self.total(|t| if t.selected { t.scan } else { None })
    }

    fn cleaned_total(&self) -> Measure {
        self.total(|t| t.cleaned)
    }

    fn start_job(&mut self, clean: bool) {
        if self.selected_count() == 0 {
            self.complain("Nothing selected.");
            return;
        }
        for target in &mut self.targets {
            if target.selected {
                target.errors = 0;
                if clean {
                    target.cleaned = None;
                } else {
                    target.scan = None;
                }
            }
        }
        self.last_run_was_dry = clean && self.dry_run;
        self.job = Some(cleaner::start(&self.targets, clean, self.dry_run));
    }

    fn poll_job(&mut self, ctx: &egui::Context) {
        let Some(job) = &mut self.job else { return };
        let cleaning = job.cleaning;
        let mut finished = false;
        let mut messages = Vec::new();
        while let Ok(progress) = job.rx.try_recv() {
            messages.push(progress);
        }
        for progress in messages {
            match progress {
                Progress::Current { path } => {
                    if let Some(job) = &mut self.job {
                        job.current = path;
                    }
                }
                Progress::Measured {
                    target,
                    measure,
                    errors,
                } => {
                    if let Some(t) = self.targets.get_mut(target) {
                        t.scan = Some(measure);
                        t.errors = errors;
                    }
                    if let Some(job) = &mut self.job {
                        job.done += 1;
                    }
                }
                Progress::Removed {
                    target,
                    measure,
                    errors,
                } => {
                    if let Some(t) = self.targets.get_mut(target) {
                        t.cleaned = Some(measure);
                        t.scan = Some(Measure::default());
                        t.errors = errors;
                    }
                    if let Some(job) = &mut self.job {
                        job.done += 1;
                    }
                }
                Progress::Finished => finished = true,
            }
        }

        if !finished {
            ctx.request_repaint_after(Duration::from_millis(80));
            return;
        }

        self.job = None;
        if cleaning && !self.last_run_was_dry {
            let total = self.cleaned_total();
            self.say(format!(
                "Cleaned {} across {} files.",
                human_bytes(total.bytes),
                total.files
            ));
            self.machine = Machine::collect();
            self.machine_refreshed = Instant::now();
        } else if cleaning {
            let total = self.scanned_total();
            self.say(format!(
                "Dry run complete. {} in {} files would be removed.",
                human_bytes(total.bytes),
                total.files
            ));
        } else {
            let total = self.scanned_total();
            self.say(format!(
                "Scan complete. {} reclaimable in {} files.",
                human_bytes(total.bytes),
                total.files
            ));
        }
    }

    fn refresh_ram_data(&mut self) {
        self.cpu_monitor
            .refresh_processes(sysinfo::ProcessesToUpdate::All, true);
        match ram::memory_snapshot() {
            Ok(snapshot) => {
                self.ram_snapshot = Some(snapshot);
                self.ram_error = None;
            }
            Err(error) => {
                self.ram_snapshot = None;
                self.ram_error = Some(error);
            }
        }
        self.ram_processes = ram::top_processes(&self.cpu_monitor);
    }

    fn start_size_scan(&mut self, root: PathBuf) {
        if let Some(scan) = &self.size_scan {
            scan.cancel();
        }
        self.size_root = Some(root.clone());
        self.size_progress = ScanProgress::default();
        self.size_result = None;
        self.size_selected_folder = 0;
        self.size_scan = Some(treesize::start_scan(root));
    }

    fn poll_size_scan(&mut self, ctx: &egui::Context) {
        let mut messages = Vec::new();
        if let Some(scan) = &self.size_scan {
            loop {
                match scan.messages.try_recv() {
                    Ok(message) => messages.push(message),
                    Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
                }
            }
        }

        let mut finished = false;
        for message in messages {
            match message {
                ScanMessage::Progress(progress) => self.size_progress = progress,
                // A tree built from what has been walked so far. Shown
                // immediately: the folders already measured are useful
                // before the rest arrive, and a scan of a whole drive is
                // long enough that waiting for all of it feels broken.
                // Folder indices are append-only across snapshots, so
                // whatever the user has drilled into stays put.
                ScanMessage::Partial(result) => {
                    self.size_progress = result.progress.clone();
                    if self.size_selected_folder >= result.folders.len() {
                        self.size_selected_folder = 0;
                    }
                    self.size_result = Some(result);
                }
                ScanMessage::Finished(Ok(result)) => {
                    let status = if result.cancelled {
                        "Folder scan stopped"
                    } else if result.capped
                        || result.depth_limited
                        || result.progress.inaccessible > 0
                    {
                        "Partial folder scan"
                    } else {
                        "Folder scan complete"
                    };
                    self.size_progress = result.progress.clone();
                    self.size_result = Some(result);
                    self.say(format!(
                        "{status}: {} files, {} scanned.",
                        self.size_progress.files,
                        human_bytes(self.size_progress.bytes)
                    ));
                    finished = true;
                }
                ScanMessage::Finished(Err(error)) => {
                    self.size_result = None;
                    self.complain(format!("Folder scan failed: {error}"));
                    finished = true;
                }
            }
        }
        if finished {
            self.size_scan = None;
        } else if self.size_scan.is_some() {
            ctx.request_repaint_after(Duration::from_millis(120));
        }
    }

    fn poll_token_scan(&mut self, ctx: &egui::Context) {
        let received = self.token_scan_rx.as_ref().map(Receiver::try_recv);
        match received {
            Some(Ok(report)) => {
                self.token_scan_rx = None;
                let message = if report.findings.is_empty() {
                    format!(
                        "Script scan finished: no suspicious matches in {} files.",
                        report.files_scanned
                    )
                } else {
                    format!(
                        "Script scan finished: {} possible match(es) in {} files.",
                        report.findings.len(),
                        report.files_scanned
                    )
                };
                self.token_scan_report = Some(report);
                self.say(message);
            }
            Some(Err(TryRecvError::Empty)) => {
                ctx.request_repaint_after(Duration::from_millis(200));
            }
            Some(Err(TryRecvError::Disconnected)) => {
                self.token_scan_rx = None;
                self.complain("The local script scan stopped unexpectedly.");
            }
            None => {}
        }
    }

    /// Hand a backup task to a worker and remember the channel.
    ///
    /// Both tasks report a `Vec<PathBuf>`; a restore point simply has no
    /// files to report, which keeps one channel type for both rather than
    /// two nearly identical ones.
    fn start_backup_job(&mut self, job: BackupJob, directory: Option<PathBuf>) {
        if self.backup_job.is_some() {
            return;
        }
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let outcome = match job {
                BackupJob::Export => match directory {
                    Some(directory) => tools::export_registry(&directory),
                    None => Err("No folder was chosen for the backup.".to_string()),
                },
                BackupJob::RestorePoint => tools::create_restore_point().map(|()| Vec::new()),
            };
            let _ = sender.send(outcome);
        });
        self.backup_job = Some((job, receiver));
    }

    fn poll_backup_job(&mut self, ctx: &egui::Context) {
        let Some((job, receiver)) = &self.backup_job else {
            return;
        };
        let job = *job;
        match receiver.try_recv() {
            Ok(Ok(files)) => {
                self.backup_job = None;
                match job {
                    BackupJob::Export => {
                        let count = files.len();
                        self.backups = files;
                        self.say(format!("Exported {count} hives."));
                    }
                    BackupJob::RestorePoint => self.say("Restore point created."),
                }
            }
            Ok(Err(error)) => {
                self.backup_job = None;
                self.complain(error);
            }
            Err(TryRecvError::Disconnected) => {
                self.backup_job = None;
                self.complain("The backup task stopped without reporting a result.");
            }
            Err(TryRecvError::Empty) => {
                ctx.request_repaint_after(Duration::from_millis(200));
            }
        }
    }

    /// Drain everything the check worker has queued since the last frame.
    ///
    /// Drained into a local first because the handlers assign to fields of
    /// `self`, which cannot be done while the receiver is still borrowed out
    /// of it. A whole batch per frame is also the point: nine checks can
    /// finish inside one repaint on a healthy machine.
    fn poll_diagnose_events(&mut self, ctx: &egui::Context) {
        let mut messages = Vec::new();
        let mut disconnected = false;
        let mut drained = false;
        match self.diagnose_scan_rx.as_ref() {
            Some(receiver) => loop {
                match receiver.try_recv() {
                    Ok(message) => messages.push(message),
                    Err(TryRecvError::Empty) => {
                        drained = true;
                        break;
                    }
                    Err(TryRecvError::Disconnected) => {
                        disconnected = true;
                        break;
                    }
                }
            },
            None => return,
        }

        for message in messages {
            match message {
                diagnose::CheckMessage::Started(id) => self.diagnose_running_check = Some(id),
                diagnose::CheckMessage::Events(result) => self.diagnose_event_result = Some(result),
                diagnose::CheckMessage::Finished(outcome, elapsed) => {
                    self.diagnose_running_check = None;
                    self.diagnose_durations.retain(|(id, _)| *id != outcome.id);
                    self.diagnose_durations.push((outcome.id, elapsed));
                    // Re-running the scan without clearing first would stack a
                    // second row per check, so the old verdict is dropped.
                    self.diagnose_outcomes
                        .retain(|existing| existing.id != outcome.id);
                    self.diagnose_outcomes.push(outcome);
                }
                diagnose::CheckMessage::AllDone => {
                    self.diagnose_scan_rx = None;
                    self.diagnose_running_check = None;
                }
            }
        }

        // The sender is dropped without an `AllDone` only if a check panicked
        // or the thread could not start. The event panel is the one place the
        // user would otherwise sit waiting on a spinner forever.
        if disconnected && self.diagnose_scan_rx.is_some() {
            self.diagnose_scan_rx = None;
            self.diagnose_running_check = None;
            if self.diagnose_event_result.is_none() {
                self.diagnose_event_result = Some(EventScanResult {
                    status: ScanStatus::Error("The Windows checks stopped unexpectedly.".into()),
                    findings: Vec::new(),
                    log_errors: Vec::new(),
                });
            }
        }
        if drained && self.diagnose_scan_rx.is_some() {
            ctx.request_repaint_after(Duration::from_millis(200));
        }
    }

    /// Whether the event log check in particular has yet to report. The event
    /// panel tracks this rather than the whole scan, because the slow checks
    /// run after it and would otherwise keep the panel spinning over results
    /// it already has.
    fn diagnose_events_pending(&self) -> bool {
        self.diagnose_scan_rx.is_some() && self.diagnose_event_result.is_none()
    }

    fn start_diagnosis(&mut self, notify: bool) {
        self.refresh_ram_data();
        self.diagnose_event_result = None;
        self.diagnose_outcomes.clear();
        self.diagnose_durations.clear();
        self.diagnose_running_check = None;
        self.diagnose_scan_rx = Some(diagnose::start_system_scan(diagnose::Scope::Full));
        if notify {
            self.say("Running the read-only diagnostic…");
        }
    }

    /// The startup pass: the cheap checks, no announcement.
    ///
    /// Kept separate from `start_diagnosis` rather than hidden behind another
    /// boolean, because the two have different jobs. This one feeds the score
    /// on the Overview; that one is a diagnosis the user asked for and whose
    /// progress they are watching.
    /// Run the startup pass only when the cached score is old or missing.
    ///
    /// Twelve hours. Launching the app is not a request to scan the machine,
    /// and with the dev loop restarting it on every save it was scanning
    /// constantly. A score from this morning is a better answer than a fresh
    /// sweep nobody asked for.
    fn start_quick_diagnosis_if_stale(&mut self) {
        // Once a week. The probe exists so a machine nobody has run a
        // diagnostic on still shows something true; it is not a schedule for
        // checking up on anyone, and running it per launch was the complaint
        // that produced it.
        const MAX_AGE_SECONDS: u64 = 7 * 24 * 60 * 60;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|since| since.as_secs())
            .unwrap_or(0);
        let last_look = self
            .settings
            .last_probe_at
            .max(self.settings.last_health_checked);
        let fresh = last_look.is_some_and(|checked| now.saturating_sub(checked) < MAX_AGE_SECONDS);
        if fresh && self.settings.last_health_score.is_some() {
            return;
        }
        self.settings.last_probe_at = Some(now);
        self.settings.save();
        self.start_quick_diagnosis();
    }

    fn start_quick_diagnosis(&mut self) {
        self.refresh_ram_data();
        self.probe_outcomes.clear();
        self.probe_durations.clear();
        self.probe_rx = Some(diagnose::start_system_scan(diagnose::Scope::HealthProbe));
    }

    /// Drain the probe. Nothing here touches the Diagnose page's state: no
    /// stepper, no check rows, no running-check spinner. The event result is
    /// the one exception, because the score counts reliability events and the
    /// figure is the same either way.
    fn poll_probe(&mut self, ctx: &egui::Context) {
        let Some(receiver) = self.probe_rx.as_ref() else {
            return;
        };
        let mut messages = Vec::new();
        let mut finished = false;
        loop {
            match receiver.try_recv() {
                Ok(message) => messages.push(message),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    finished = true;
                    break;
                }
            }
        }
        for message in messages {
            match message {
                diagnose::CheckMessage::Events(result) => {
                    if self.diagnose_event_result.is_none() {
                        self.diagnose_event_result = Some(result);
                    }
                }
                diagnose::CheckMessage::Finished(outcome, elapsed) => {
                    self.probe_durations.retain(|(id, _)| *id != outcome.id);
                    self.probe_durations.push((outcome.id, elapsed));
                    self.probe_outcomes
                        .retain(|existing| existing.id != outcome.id);
                    self.probe_outcomes.push(outcome);
                }
                diagnose::CheckMessage::AllDone => finished = true,
                diagnose::CheckMessage::Started(_) => {}
            }
        }
        if finished {
            self.probe_rx = None;
        } else {
            ctx.request_repaint_after(Duration::from_millis(400));
        }
    }

    /// How far the score has moved since the previous measurement.
    fn health_trend(&self) -> Option<i32> {
        let now = self.settings.last_health_score? as i32;
        let before = self.settings.prior_health_score? as i32;
        let delta = now - before;
        (delta != 0).then_some(delta)
    }

    /// How long ago the cached score was measured, in hours.
    fn health_age_hours(&self) -> Option<f64> {
        let checked = self.settings.last_health_checked?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_secs();
        Some(now.saturating_sub(checked) as f64 / 3600.0)
    }

    /// The check results the score should use: the diagnostic when the user
    /// has run one, the probe otherwise.
    fn scored_checks(&self) -> &[diagnose::CheckOutcome] {
        if self.diagnose_outcomes.is_empty() {
            &self.probe_outcomes
        } else {
            &self.diagnose_outcomes
        }
    }

    /// Remember the score, and -- only for a diagnostic the user ran -- when
    /// it was measured. The probe refreshes the figure without claiming the
    /// machine was checked: "last checked" has to mean the real sweep, or it
    /// means nothing.
    fn cache_health(&mut self, health: &Health) {
        if health.mood == health::Mood::Checking
            || self.settings.last_health_score == Some(health.score)
        {
            return;
        }
        if self.settings.last_health_score.is_none()
            || self.health_cache_refreshed.elapsed() >= Duration::from_secs(30)
        {
            self.settings.prior_health_score = self.settings.last_health_score;
            self.settings.last_health_score = Some(health.score);
            if !self.diagnose_outcomes.is_empty() {
                self.settings.last_health_checked = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|since| since.as_secs())
                    .ok();
            }
            self.settings.save();
            self.health_cache_refreshed = Instant::now();
        }
    }

    fn reliability_event_count(&self) -> Option<usize> {
        let result = self.diagnose_event_result.as_ref()?;
        if !matches!(&result.status, ScanStatus::Complete | ScanStatus::Partial) {
            return None;
        }
        Some(
            result
                .findings
                .iter()
                .filter(|finding| {
                    let source = finding.source.to_ascii_lowercase();
                    source.contains("whea")
                        || source.contains("disk")
                        || source.contains("ntfs")
                        || source.contains("resource-exhaustion")
                        || source.contains("kernel-power")
                        || source.contains("display")
                        || source.contains("application error")
                        || source.contains("windows error reporting")
                })
                .count(),
        )
    }

    fn start_app_residue_scan(&mut self) {
        let (tx, rx) = mpsc::channel();
        self.app_residue_rx = Some(rx);
        self.app_residue_result = None;
        thread::spawn(move || {
            let _ = tx.send(app_residue::scan());
        });
    }

    fn start_registry_audit(&mut self) {
        let (tx, rx) = mpsc::channel();
        self.registry_audit_rx = Some(rx);
        self.registry_audit_result = None;
        thread::spawn(move || {
            let _ = tx.send(registry_audit::scan_uninstall_entries());
        });
    }

    fn start_startup_scan(&mut self) {
        let (tx, rx) = mpsc::channel();
        self.startup_scan_rx = Some(rx);
        self.startup_scan_result = None;
        thread::spawn(move || {
            let _ = tx.send(startup::scan_startup_inventory());
        });
    }

    fn poll_readonly_scans(&mut self, ctx: &egui::Context) {
        let residue = self.app_residue_rx.as_ref().map(Receiver::try_recv);
        match residue {
            Some(Ok(result)) => {
                self.app_residue_rx = None;
                let count = result.findings.len();
                self.app_residue_result = Some(result);
                self.say(format!(
                    "AppData review finished: {} possible leftover.",
                    plural(count, "folder")
                ));
            }
            Some(Err(TryRecvError::Empty)) => ctx.request_repaint_after(Duration::from_millis(200)),
            Some(Err(TryRecvError::Disconnected)) => {
                self.app_residue_rx = None;
                self.complain("The AppData review stopped unexpectedly.");
            }
            None => {}
        }

        let registry = self.registry_audit_rx.as_ref().map(Receiver::try_recv);
        match registry {
            Some(Ok(result)) => {
                self.registry_audit_rx = None;
                let count = result.candidates.len();
                self.registry_audit_result = Some(result);
                self.say(format!(
                    "Registry review finished: {} possibly stale. No changes made.",
                    plural(count, "record")
                ));
            }
            Some(Err(TryRecvError::Empty)) => ctx.request_repaint_after(Duration::from_millis(200)),
            Some(Err(TryRecvError::Disconnected)) => {
                self.registry_audit_rx = None;
                self.complain("The registry review stopped unexpectedly.");
            }
            None => {}
        }

        let startup = self.startup_scan_rx.as_ref().map(Receiver::try_recv);
        match startup {
            Some(Ok(result)) => {
                self.startup_scan_rx = None;
                let count = result.entries.len();
                self.startup_scan_result = Some(result);
                self.say(format!(
                    "Startup inventory finished: {count} entries found."
                ));
            }
            Some(Err(TryRecvError::Empty)) => ctx.request_repaint_after(Duration::from_millis(200)),
            Some(Err(TryRecvError::Disconnected)) => {
                self.startup_scan_rx = None;
                self.complain("The startup inventory stopped unexpectedly.");
            }
            None => {}
        }
    }

    fn poll_security_integrity(&mut self, ctx: &egui::Context) {
        let received = self.explorer_integrity_rx.as_ref().map(Receiver::try_recv);
        match received {
            Some(Ok(result)) => {
                self.explorer_integrity_rx = None;
                self.explorer_integrity_result = Some(result);
            }
            Some(Err(TryRecvError::Empty)) => ctx.request_repaint_after(Duration::from_millis(200)),
            Some(Err(TryRecvError::Disconnected)) => {
                self.explorer_integrity_rx = None;
                self.complain("The Explorer integrity scan stopped unexpectedly.");
            }
            None => {}
        }

        let received = self.malware_lookup_rx.as_ref().map(Receiver::try_recv);
        match received {
            Some(Ok(result)) => {
                self.malware_lookup_rx = None;
                self.malware_lookup_result = Some(result);
            }
            Some(Err(TryRecvError::Empty)) => ctx.request_repaint_after(Duration::from_millis(200)),
            Some(Err(TryRecvError::Disconnected)) => {
                self.malware_lookup_rx = None;
                self.malware_lookup_result =
                    Some(Err("The online hash lookup stopped unexpectedly.".into()));
            }
            None => {}
        }

        let received = self.malware_feed_scan_rx.as_ref().map(Receiver::try_recv);
        match received {
            Some(Ok(result)) => {
                self.malware_feed_scan_rx = None;
                self.malware_feed_scan_result = Some(result);
            }
            Some(Err(TryRecvError::Empty)) => ctx.request_repaint_after(Duration::from_millis(200)),
            Some(Err(TryRecvError::Disconnected)) => {
                self.malware_feed_scan_rx = None;
                self.malware_feed_scan_result =
                    Some(Err("The recent-feed scan stopped unexpectedly.".into()));
            }
            None => {}
        }

        let received = self.network_scan_rx.as_ref().map(Receiver::try_recv);
        match received {
            Some(Ok(result)) => {
                self.network_scan_rx = None;
                self.network_scan_result = Some(result);
            }
            Some(Err(TryRecvError::Empty)) => ctx.request_repaint_after(Duration::from_millis(200)),
            Some(Err(TryRecvError::Disconnected)) => {
                self.network_scan_rx = None;
                self.complain("The network activity scan stopped unexpectedly.");
            }
            None => {}
        }
    }

    fn start_network_scan(&mut self) {
        self.network_scan_result = None;
        self.network_scan_rx = Some(network_watch::start_scan());
        self.say("Taking a read-only snapshot of active network connections…");
    }

    fn start_malware_lookup(&mut self) {
        let Some(path) = self.malware_lookup_path.clone() else {
            self.complain("Choose a file before starting the online lookup.");
            return;
        };
        let key = self.malware_lookup_key.trim().to_owned();
        if key.is_empty() {
            self.complain("Enter a MalwareBazaar Auth-Key to opt in to the online lookup.");
            return;
        }
        let (tx, rx) = mpsc::channel();
        self.malware_lookup_rx = Some(rx);
        self.malware_lookup_result = None;
        self.malware_lookup_key.clear();
        thread::spawn(move || {
            let _ = tx.send(malware_lookup::lookup_file(&path.to_string_lossy(), &key));
        });
        self.say("Hashing the selected file locally and querying MalwareBazaar…");
    }

    fn start_malware_feed_scan(&mut self) {
        let Some(folder) = self.malware_feed_scan_folder.clone() else {
            self.complain("Choose a folder before starting the recent-feed scan.");
            return;
        };
        let key = self.malware_lookup_key.trim().to_owned();
        if key.is_empty() {
            self.complain("Enter a MalwareBazaar Auth-Key to opt in to the online scan.");
            return;
        }
        let (tx, rx) = mpsc::channel();
        self.malware_feed_scan_rx = Some(rx);
        self.malware_feed_scan_result = None;
        self.malware_lookup_key.clear();
        thread::spawn(move || {
            let _ = tx.send(malware_lookup::scan_folder(&folder.to_string_lossy(), &key));
        });
        self.say(
            "Downloading MalwareBazaar's recent hash feed and scanning selected files locally…",
        );
    }

    /// Most recent GPU load value reported by LibreHardwareMonitor, if present.
    fn sensor_gpu_load(&self) -> Option<f32> {
        self.hardware_telemetry
            .iter()
            .filter(|reading| {
                reading
                    .hardware_type
                    .to_ascii_lowercase()
                    .starts_with("gpu")
                    && reading.sensor_type.eq_ignore_ascii_case("Load")
            })
            .filter_map(|reading| reading.value.trim().parse::<f32>().ok())
            .filter(|load| load.is_finite())
            .map(|load| load.clamp(0.0, 100.0))
            .max_by(f32::total_cmp)
    }

    /// Why a temperature is missing, in terms the person can act on.
    ///
    /// LibreHardwareMonitor reads CPU core temperatures and motherboard
    /// sensors through a kernel driver it can only load as administrator. Run
    /// unelevated it still reports CPU voltages, which is why the card looks
    /// populated while the temperature is simply absent. Saying so beats
    /// showing nothing and letting it look like the hardware has no sensor.
    fn temperature_hint(&self) -> &'static str {
        if self.elevated {
            "This provider reported no temperature sensor for this device."
        } else {
            "Temperatures need administrator rights: the sensor driver cannot load without them. Use \"Standard user\" in the status bar to restart elevated."
        }
    }

    /// The warmest temperature the provider reports for a kind of device.
    /// Several sensors can describe one part (core, hot spot, package); the
    /// highest is the one that decides whether it is in trouble.
    fn sensor_temperature(&self, hardware_type: &str) -> Option<f32> {
        let want = hardware_type.to_ascii_lowercase();
        self.hardware_telemetry
            .iter()
            .filter(|r| {
                if !r.sensor_type.eq_ignore_ascii_case("Temperature") {
                    return false;
                }
                let owner = r.hardware_type.to_ascii_lowercase();
                if owner.starts_with(&want) {
                    return true;
                }
                // Many boards expose the CPU temperature through their SuperIO
                // chip, so it arrives owned by the motherboard rather than by
                // the processor. Match those by sensor name instead.
                let board = owner.starts_with("mainboard")
                    || owner.starts_with("motherboard")
                    || owner.starts_with("superio");
                board && r.sensor_name.to_ascii_lowercase().contains(&want)
            })
            .filter_map(|r| r.value.trim().parse::<f32>().ok())
            .fold(None, |best: Option<f32>, v| {
                Some(best.map_or(v, |b| b.max(v)))
            })
    }
}

/// A time-series chart with a window selector. The selector sits with the
/// chart rather than in a toolbar, because it only changes this chart.
/// A time series card: the chart is the content, everything else is chrome
/// around it.
///
/// The header carries the title on the left and the range switches on the
/// right, the plot takes the full width of the card, and the key sits under
/// it as dots. The plot library's own legend is off: it floats inside the
/// data in a bordered box, which on a card like this reads as a second
/// surface laid over the first.
fn history_chart(
    ui: &mut Ui,
    window: &mut usize,
    title: &str,
    series: &[(&str, &crate::history::Series, Color32)],
    unit: &str,
) {
    let accent = theme::accent_on(ui, theme::ACCENT);
    theme::widget(ui).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.add(
                Label::new(
                    RichText::new(title)
                        .font(theme::card_title())
                        .strong()
                        .color(theme::text(ui)),
                )
                .truncate(),
            );
            ui.add(Label::new(RichText::new(unit).font(theme::eyebrow()).color(MUTED)).truncate());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                for (index, (label, _)) in History::WINDOWS.iter().enumerate().rev() {
                    if chrome::segment(ui, label, *window == index, accent)
                        .pointer()
                        .clicked()
                    {
                        *window = index;
                    }
                }
            });
        });

        let (_, window_secs) = History::WINDOWS[(*window).min(2)];
        if series.iter().all(|(_, s, _)| s.len() < 2) {
            ui.add_space(theme::GAP);
            ui.label(
                RichText::new("Collecting readings\u{2026}")
                    .small()
                    .color(MUTED),
            );
            ui.add_space(theme::GAP);
            return;
        }

        ui.add_space(theme::GAP);
        egui_plot::Plot::new(format!("chart-{title}"))
            .height(180.0)
            .allow_drag(false)
            .allow_zoom(false)
            .allow_scroll(false)
            .allow_boxed_zoom(false)
            .show_axes([false, true])
            // Let the card show through. The plot default paints
            // extreme_bg_color, which is sized for a thin gauge track and
            // reads as a heavy grey slab at chart size -- obvious on light.
            .show_background(false)
            // The frame boxes the plot on all four sides. The card is already
            // the boundary, so a second one just adds a line.
            .show_grid([false, true])
            // Both series are read as percentages or degrees on the same
            // 0..100 scale, so pin it. A free axis rescales on every sample
            // and makes a flat reading look like it is swinging.
            .include_y(0.0)
            .include_y(100.0)
            .include_x(-window_secs as f64)
            .include_x(0.0)
            // The default grid draws a line per minor step, which at this
            // size is a mesh rather than a reference.
            .y_grid_spacer(egui_plot::uniform_grid_spacer(|_| [50.0, 25.0, 25.0]))
            .x_grid_spacer(egui_plot::uniform_grid_spacer(move |_| {
                let step = (window_secs / 6.0).max(1.0) as f64;
                [step * 2.0, step, step]
            }))
            .x_axis_formatter(|mark, _| {
                let secs = -mark.value;
                if secs <= 0.5 {
                    "now".to_owned()
                } else {
                    format!("-{secs:.0}s")
                }
            })
            .y_axis_formatter(|mark, _| format!("{:.0}", mark.value))
            .show(ui, |plot| {
                for (name, data, color) in series {
                    let points = data.points(window_secs);
                    if points.len() >= 2 {
                        // One series gets the wash under it; several would
                        // overlap into mud, so those stay as lines.
                        if series.len() == 1 {
                            plot.polygon(
                                egui_plot::Polygon::new(
                                    *name,
                                    egui_plot::PlotPoints::from(floor_under(&points)),
                                )
                                .fill_color(color.gamma_multiply(0.16))
                                .stroke(egui::Stroke::NONE)
                                .allow_hover(false),
                            );
                        }
                        plot.line(
                            egui_plot::Line::new(*name, egui_plot::PlotPoints::from(points))
                                .color(*color)
                                .width(1.8),
                        );
                    }
                }
            });

        ui.add_space(theme::GAP_TIGHT);
        let key: Vec<(&str, Color32)> = series
            .iter()
            .map(|(name, _, color)| (*name, *color))
            .collect();
        chrome::legend(ui, &key);
    });
}

/// Close a trace into a polygon by running back along the zero line, so the
/// area under it can be filled.
fn floor_under(points: &[[f64; 2]]) -> Vec<[f64; 2]> {
    let mut shape = Vec::with_capacity(points.len() + 2);
    shape.push([points[0][0], 0.0]);
    shape.extend_from_slice(points);
    shape.push([points[points.len() - 1][0], 0.0]);
    shape
}

impl App {
    /// The wordmark texture for the current theme, uploaded once and kept
    /// until the theme changes.
    fn wordmark(&mut self, ui: &Ui) -> egui::TextureHandle {
        let dark = ui.visuals().dark_mode;
        if let Some((cached_dark, texture)) = &self.wordmark {
            if *cached_dark == dark {
                return texture.clone();
            }
        }
        let texture = ui.ctx().load_texture(
            if dark {
                "wordmark-dark"
            } else {
                "wordmark-light"
            },
            chrome::wordmark_image(dark),
            egui::TextureOptions::LINEAR,
        );
        self.wordmark = Some((dark, texture.clone()));
        texture
    }

    fn poll_cpu_usage(&mut self, ctx: &egui::Context) {
        if self.cpu_refreshed.elapsed() >= Duration::from_secs(1) {
            self.cpu_monitor.refresh_cpu_usage();
            self.cpu_usage = self.cpu_monitor.global_cpu_usage().clamp(0.0, 100.0);
            self.cpu_refreshed = Instant::now();

            // Record the live readings on the same tick, so every series shares
            // a clock and the charts line up with each other.
            self.history.cpu.push(self.cpu_usage);
            self.history.ram.push(self.machine.ram_fraction() * 100.0);
            if let Some(load) = self.sensor_gpu_load() {
                self.history.gpu_load.push(load);
            }
            if let Some(c) = self.sensor_temperature("Cpu") {
                self.history.cpu_temp.push(c);
            }
            if let Some(c) = self.sensor_temperature("Gpu") {
                self.history.gpu_temp.push(c);
            }
            if !self.settings.first_run_baseline_completed && self.history.cpu.len() >= 30 {
                self.settings.first_run_baseline_completed = true;
                self.settings.save();
            }
        }
        ctx.request_repaint_after(Duration::from_millis(250));
    }

    fn poll_gpu_sensors(&mut self, ctx: &egui::Context) {
        if self.gpu_sensor_rx.is_none()
            && self.gpu_sensor_requested.elapsed() >= Duration::from_secs(10)
        {
            let (tx, rx) = mpsc::channel();
            self.gpu_sensor_rx = Some(rx);
            self.gpu_sensor_requested = Instant::now();
            thread::spawn(move || {
                let _ = tx.send(SensorQueryResult {
                    gpu: read_nvidia_sensors(),
                    hardware: read_hardware_sensor_provider(),
                });
            });
        }
        match self.gpu_sensor_rx.as_ref().map(Receiver::try_recv) {
            Some(Ok(readings)) => {
                match readings.gpu {
                    Ok(values) => {
                        self.gpu_telemetry = values;
                        self.gpu_sensor_error = None;
                    }
                    Err(error) => self.gpu_sensor_error = Some(error),
                }
                match readings.hardware {
                    Ok(values) => {
                        self.hardware_telemetry = values;
                        self.hardware_sensor_error = None;
                    }
                    Err(error) => {
                        self.hardware_telemetry.clear();
                        self.hardware_sensor_error = Some(error);
                    }
                }
                self.gpu_sensor_rx = None;
            }
            Some(Err(TryRecvError::Disconnected)) => {
                self.gpu_sensor_error = Some("Graphics sensor query stopped unexpectedly.".into());
                self.hardware_sensor_error =
                    Some("Hardware sensor query stopped unexpectedly.".into());
                self.gpu_sensor_rx = None;
            }
            Some(Err(TryRecvError::Empty)) => ctx.request_repaint_after(Duration::from_millis(200)),
            None => {}
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll_job(&ctx);
        self.poll_token_scan(&ctx);
        self.poll_diagnose_events(&ctx);
        self.poll_probe(&ctx);
        self.poll_backup_job(&ctx);
        self.poll_readonly_scans(&ctx);
        self.poll_security_integrity(&ctx);
        self.poll_size_scan(&ctx);
        self.poll_cpu_usage(&ctx);
        self.poll_gpu_sensors(&ctx);

        if self.machine_refreshed.elapsed().as_secs() >= 5 && self.job.is_none() {
            self.machine = Machine::collect();
            self.refresh_ram_data();
            self.machine_refreshed = Instant::now();
        }

        let title_bar = Panel::top(Id::new("title-bar"))
            .exact_size(chrome::TITLE_BAR_HEIGHT)
            .resizable(false)
            .frame(theme::chrome(ui))
            .show(ui, |ui| chrome::title_bar(ui, self.elevated))
            .inner;
        if title_bar.close_requested {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if title_bar.elevation_requested {
            self.elevate_now(&ctx);
        }

        self.sidebar(ui);
        self.status_bar(ui);
        self.body(ui);
        self.confirm_dialog(&ctx);
        self.confirm_action_dialog(&ctx);
        self.confirm_activation_dialog(&ctx);
        self.confirm_ram_trim_dialog(&ctx);
        self.confirm_ram_action_dialog(&ctx);
        chrome::resize_edges(ui);
    }
}

impl App {
    fn sidebar(&mut self, ui: &mut Ui) {
        let accent = theme::accent_on(ui, self.accent);
        let frame = theme::chrome(ui).inner_margin(Margin::symmetric(12, 16));
        Panel::left(Id::new("nav"))
            .exact_size(212.0)
            .resizable(false)
            .frame(frame)
            .show(ui, |ui| {
                let logo = self.wordmark(ui);
                let width = ui.available_width().min(176.0);
                let size = egui::vec2(width, width / logo.aspect_ratio());
                ui.add(egui::Image::new(&logo).fit_to_exact_size(size));
                // Just the version. "Free and open source" belongs on the
                // About page, not under the logo on every screen.
                ui.label(
                    RichText::new(format!("v{}", env!("CARGO_PKG_VERSION")))
                        .font(theme::readout())
                        .color(FAINT),
                );
                ui.add_space(theme::GAP_SECTION);

                // The list scrolls; the theme switch below it does not. Without
                // this the last entries fall off the bottom on a short window.
                let switch_room = 46.0;
                let list_height = (ui.available_height() - switch_room).max(0.0);
                egui::ScrollArea::vertical()
                    .id_salt("nav-list")
                    .max_height(list_height)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for tab in Tab::ALL {
                            let selected = self.tab == tab;
                            if tab == Tab::Memory {
                                // Memory is a separate page under Hardware.
                                continue;
                            }
                            if tab == Tab::Hardware {
                                let in_group = matches!(self.tab, Tab::Hardware | Tab::Memory);
                                if chrome::nav_group(
                                    ui,
                                    in_group,
                                    chrome::icon::DEVICES,
                                    "Hardware",
                                    accent,
                                    self.hardware_expanded,
                                )
                                .pointer()
                                .clicked()
                                {
                                    if self.tab == Tab::Hardware && self.hardware_expanded {
                                        self.hardware_expanded = false;
                                    } else {
                                        self.tab = Tab::Hardware;
                                        self.expand_group(NavGroup::Hardware);
                                    }
                                }

                                let expansion = ui.ctx().animate_bool_with_time(
                                    Id::new("hardware-submenu-expansion"),
                                    self.hardware_expanded,
                                    0.18,
                                );
                                if expansion > 0.001 {
                                    let clicked = ui
                                        .scope(|ui| {
                                            ui.set_opacity(expansion);
                                            ui.add_space(2.0);
                                            ui.horizontal(|ui| {
                                                ui.add_space(32.0);
                                                chrome::nav_item_with_height(
                                                    ui,
                                                    self.tab == Tab::Memory,
                                                    chrome::icon::MEMORY,
                                                    "Memory",
                                                    accent,
                                                    32.0 * expansion,
                                                    expansion > 0.95,
                                                )
                                                .pointer()
                                                .clicked()
                                            })
                                            .inner
                                        })
                                        .inner;
                                    if clicked {
                                        self.tab = Tab::Memory;
                                        self.expand_group(NavGroup::Hardware);
                                    }
                                }
                                continue;
                            }
                            if tab == Tab::Storage {
                                // The row is the target: clicking it opens Storage and
                                // opens its children; clicking it again folds them away.
                                if chrome::nav_group(
                                    ui,
                                    selected && self.storage_view == StorageView::Summary,
                                    chrome::icon::STORAGE,
                                    "Storage",
                                    accent,
                                    self.storage_expanded,
                                )
                                .pointer()
                                .clicked()
                                {
                                    if self.tab == Tab::Storage && self.storage_expanded {
                                        self.storage_expanded = false;
                                    } else {
                                        self.tab = Tab::Storage;
                                        self.storage_view = StorageView::Summary;
                                        self.expand_group(NavGroup::Storage);
                                    }
                                }

                                let expansion = ui.ctx().animate_bool_with_time(
                                    Id::new("storage-cleanup-expansion"),
                                    self.storage_expanded,
                                    0.18,
                                );
                                if expansion > 0.001 {
                                    let clicked = ui
                                        .scope(|ui| {
                                            ui.set_opacity(expansion);
                                            ui.add_space(2.0);
                                            ui.horizontal(|ui| {
                                                ui.add_space(32.0);
                                                chrome::nav_item_with_height(
                                                    ui,
                                                    self.tab == Tab::Storage
                                                        && self.storage_view
                                                            == StorageView::Cleanup,
                                                    chrome::icon::CLEANER,
                                                    "Cleanup",
                                                    accent,
                                                    32.0 * expansion,
                                                    expansion > 0.95,
                                                )
                                                .pointer()
                                                .clicked()
                                            })
                                            .inner
                                        })
                                        .inner;
                                    if clicked {
                                        self.tab = Tab::Storage;
                                        self.storage_view = StorageView::Cleanup;
                                        self.expand_group(NavGroup::Storage);
                                    }

                                    let clicked = ui
                                        .scope(|ui| {
                                            ui.set_opacity(expansion);
                                            ui.horizontal(|ui| {
                                                ui.add_space(32.0);
                                                chrome::nav_item_with_height(
                                                    ui,
                                                    self.tab == Tab::Storage
                                                        && self.storage_view
                                                            == StorageView::SizeExplorer,
                                                    chrome::icon::CHART,
                                                    "Size Explorer",
                                                    accent,
                                                    32.0 * expansion,
                                                    expansion > 0.95,
                                                )
                                                .pointer()
                                                .clicked()
                                            })
                                            .inner
                                        })
                                        .inner;
                                    if clicked {
                                        self.tab = Tab::Storage;
                                        self.storage_view = StorageView::SizeExplorer;
                                        self.expand_group(NavGroup::Storage);
                                    }

                                    let clicked = ui
                                        .scope(|ui| {
                                            ui.set_opacity(expansion);
                                            ui.horizontal(|ui| {
                                                ui.add_space(32.0);
                                                chrome::nav_item_with_height(
                                                    ui,
                                                    self.tab == Tab::Storage
                                                        && self.storage_view
                                                            == StorageView::AppLeftovers,
                                                    chrome::icon::APPS,
                                                    "App Leftovers",
                                                    accent,
                                                    32.0 * expansion,
                                                    expansion > 0.95,
                                                )
                                                .pointer()
                                                .clicked()
                                            })
                                            .inner
                                        })
                                        .inner;
                                    if clicked {
                                        self.tab = Tab::Storage;
                                        self.storage_view = StorageView::AppLeftovers;
                                        self.expand_group(NavGroup::Storage);
                                    }
                                }
                                continue;
                            }

                            if tab == Tab::Privacy {
                                // Privacy is a separate page under the Security group.
                                continue;
                            }

                            if tab == Tab::Security {
                                let in_group = matches!(self.tab, Tab::Security | Tab::Privacy);
                                if chrome::nav_group(
                                    ui,
                                    in_group,
                                    chrome::icon::SHIELD,
                                    "Security",
                                    accent,
                                    self.security_expanded,
                                )
                                .pointer()
                                .clicked()
                                {
                                    if self.tab == Tab::Security && self.security_expanded {
                                        self.security_expanded = false;
                                    } else {
                                        self.tab = Tab::Security;
                                        self.expand_group(NavGroup::Security);
                                    }
                                }

                                let expansion = ui.ctx().animate_bool_with_time(
                                    Id::new("security-submenu-expansion"),
                                    self.security_expanded,
                                    0.18,
                                );
                                if expansion > 0.001 {
                                    let clicked = ui
                                        .scope(|ui| {
                                            ui.set_opacity(expansion);
                                            ui.add_space(2.0);
                                            ui.horizontal(|ui| {
                                                ui.add_space(32.0);
                                                chrome::nav_item_with_height(
                                                    ui,
                                                    self.tab == Tab::Privacy,
                                                    chrome::icon::SHIELD,
                                                    "Privacy",
                                                    accent,
                                                    32.0 * expansion,
                                                    expansion > 0.95,
                                                )
                                                .pointer()
                                                .clicked()
                                            })
                                            .inner
                                        })
                                        .inner;
                                    if clicked {
                                        self.tab = Tab::Privacy;
                                        self.expand_group(NavGroup::Security);
                                    }
                                }
                                continue;
                            }

                            if tab == Tab::Tools {
                                if chrome::nav_group(
                                    ui,
                                    self.tab == Tab::Tools,
                                    chrome::icon::TOOLS,
                                    "Tools",
                                    accent,
                                    self.tools_expanded,
                                )
                                .pointer()
                                .clicked()
                                {
                                    if self.tab == Tab::Tools && self.tools_expanded {
                                        self.tools_expanded = false;
                                    } else {
                                        self.tab = Tab::Tools;
                                        self.expand_group(NavGroup::Tools);
                                    }
                                }

                                let expansion = ui.ctx().animate_bool_with_time(
                                    Id::new("tools-submenu-expansion"),
                                    self.tools_expanded,
                                    0.18,
                                );
                                if expansion > 0.001 {
                                    let mut selected_child = None;
                                    ui.scope(|ui| {
                                        ui.set_opacity(expansion);
                                        ui.add_space(2.0);
                                        ui.vertical(|ui| {
                                            if ui
                                                .horizontal(|ui| {
                                                    ui.add_space(32.0);
                                                    chrome::nav_item_with_height(
                                                        ui,
                                                        self.tab == Tab::Troubleshoot,
                                                        chrome::icon::TROUBLESHOOT,
                                                        "Troubleshoot",
                                                        accent,
                                                        32.0 * expansion,
                                                        expansion > 0.95,
                                                    )
                                                    .pointer()
                                                    .clicked()
                                                })
                                                .inner
                                            {
                                                selected_child = Some(Tab::Troubleshoot);
                                            }
                                            if ui
                                                .horizontal(|ui| {
                                                    ui.add_space(32.0);
                                                    chrome::nav_item_with_height(
                                                        ui,
                                                        self.tab == Tab::Repair,
                                                        chrome::icon::REPAIR,
                                                        "Repair",
                                                        accent,
                                                        32.0 * expansion,
                                                        expansion > 0.95,
                                                    )
                                                    .pointer()
                                                    .clicked()
                                                })
                                                .inner
                                            {
                                                selected_child = Some(Tab::Repair);
                                            }
                                            if ui
                                                .horizontal(|ui| {
                                                    ui.add_space(32.0);
                                                    chrome::nav_item_with_height(
                                                        ui,
                                                        self.tab == Tab::Startup,
                                                        chrome::icon::SCHEDULE,
                                                        "Startup",
                                                        accent,
                                                        32.0 * expansion,
                                                        expansion > 0.95,
                                                    )
                                                    .pointer()
                                                    .clicked()
                                                })
                                                .inner
                                            {
                                                selected_child = Some(Tab::Startup);
                                            }
                                        })
                                    });
                                    if let Some(child) = selected_child {
                                        self.tab = child;
                                        self.expand_group(NavGroup::Tools);
                                    }
                                }
                                continue;
                            }

                            let glyph = match tab {
                                Tab::Overview => chrome::icon::OVERVIEW,
                                Tab::Diagnose => chrome::icon::DIAGNOSTICS,
                                Tab::SystemInfo => chrome::icon::SYSTEM_INFO,
                                Tab::Hardware => chrome::icon::DEVICES,
                                Tab::Memory => unreachable!(),
                                Tab::Storage => unreachable!(),
                                Tab::Tweaks => chrome::icon::TWEAKS,
                                Tab::Privacy => unreachable!(),
                                Tab::Troubleshoot | Tab::Repair | Tab::Startup => unreachable!(),
                                Tab::Security => chrome::icon::SHIELD,
                                Tab::Tools => chrome::icon::TOOLS,
                                Tab::Backup => chrome::icon::BACKUP,
                                Tab::Settings => chrome::icon::SETTINGS,
                            };
                            if chrome::nav_item(ui, selected, glyph, tab.title(), accent)
                                .pointer()
                                .clicked()
                            {
                                self.tab = tab;
                            }
                        }
                    });
            });
    }

    /// Open one sidebar group and fold the rest away.
    ///
    /// Every place that used to write `self.x_expanded = true` goes through
    /// here instead, because the rule only holds if it is impossible to set
    /// one without clearing the others.
    fn expand_group(&mut self, group: NavGroup) {
        self.hardware_expanded = group == NavGroup::Hardware;
        self.storage_expanded = group == NavGroup::Storage;
        self.security_expanded = group == NavGroup::Security;
        self.tools_expanded = group == NavGroup::Tools;
    }

    fn status_bar(&mut self, ui: &mut Ui) {
        let accent = theme::accent_on(ui, self.accent);
        let frame = theme::chrome(ui).inner_margin(Margin::symmetric(14, 8));
        Panel::bottom(Id::new("status"))
            .exact_size(36.0)
            .resizable(false)
            .frame(frame)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    if let Some(job) = &self.job {
                        let verb = if job.cleaning { "Cleaning" } else { "Scanning" };
                        ui.label(RichText::new(format!("{verb}…")).color(accent));
                        let fraction = job.fraction();
                        ui.scope(|ui| {
                            ui.set_max_width(150.0);
                            chrome::meter(
                                ui,
                                fraction,
                                accent,
                                Some(&format!("{:.0}%", fraction * 100.0)),
                            );
                        });
                        ui.label(
                            RichText::new(shorten(&job.current, 64))
                                .small()
                                .color(MUTED),
                        );
                    } else if let Some(toast) = &self.toast {
                        let stale = toast.at.elapsed().as_secs() > 12;
                        let color = if toast.error {
                            theme::bad(ui)
                        } else if stale {
                            MUTED
                        } else {
                            theme::good(ui)
                        };
                        ui.label(RichText::new(&toast.text).color(color));
                    } else {
                        // Set as a label rather than a sentence: this line
                        // names the machine, it does not say anything about
                        // it, and upper case is how the rest of the dashboard
                        // marks a field name.
                        ui.label(
                            RichText::new(
                                format!(
                                    "{}  ·  {}  ·  up {}",
                                    self.machine.host_name,
                                    self.machine.windows_name,
                                    self.machine.uptime
                                )
                                .to_uppercase(),
                            )
                            .small()
                            .color(MUTED),
                        );
                    }

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        // The theme switch lives here now. It was a labelled
                        // toggle pinned to the bottom of the sidebar, which
                        // gave a once-a-month setting the same weight as the
                        // navigation; the architecture it replaces was a
                        // constant that never needed to be on screen.
                        //
                        // Custom is deliberately not in this rotation. It is a
                        // palette assembled on the Settings page, and cycling
                        // into it from here would repaint the window with
                        // colours nobody asked for at that moment. While Custom
                        // is active the button moves the base ramp it stands
                        // on, so the switch still lightens and darkens the app
                        // without throwing the palette away.
                        let light = self.settings.light();
                        let glyph = if light {
                            chrome::icon::SUN
                        } else {
                            chrome::icon::MOON
                        };
                        let hover = match (self.settings.mode, light) {
                            (Mode::Custom, true) => "Build Custom on the dark palette",
                            (Mode::Custom, false) => "Build Custom on the light palette",
                            (_, true) => "Switch to the dark theme",
                            (_, false) => "Switch to the light theme",
                        };
                        if chrome::icon_button(ui, glyph, light, accent)
                            .on_hover_text(hover)
                            .pointer()
                            .clicked()
                        {
                            self.settings.toggle_light();
                            self.settings.save();
                            self.appearance.apply(ui.ctx(), &self.settings, accent);
                        }
                        ui.label(
                            RichText::new(format!("RAM {}", human_bytes(self.machine.ram_used)))
                                .font(theme::readout())
                                .color(MUTED),
                        );
                    });
                });
            });
    }

    fn body(&mut self, ui: &mut Ui) {
        // No right margin on the panel: the scroll area has to reach the
        // window edge so its bar sits against the edge instead of over the
        // page. The padding the content needs is applied inside, by `gutter`,
        // so the bar has nothing underneath it.
        let frame = egui::Frame::new()
            .fill(ui.visuals().panel_fill)
            .inner_margin(Margin {
                left: 20,
                right: 0,
                top: 18,
                bottom: 18,
            });
        let gutter = egui::Frame::new().inner_margin(Margin {
            left: 0,
            right: 20,
            top: 0,
            bottom: 0,
        });
        CentralPanel::default().frame(frame).show(ui, |ui| {
            gutter.show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new(self.tab.title()).heading());
                ui.label(RichText::new(self.tab.blurb()).color(MUTED));
            });
            ui.add_space(theme::GAP_SECTION);

            // The app hides every scrollbar by default, which is right for
            // the sidebar and the small lists inside cards: they are short and
            // their edges say where they end. A page is neither -- it can run
            // four screens with no sign there is more.
            //
            // Floating, so it allocates no width and the page does not shift
            // sideways depending on whether it scrolls. It draws at the right
            // edge of the scroll area, which now reaches the window edge, so
            // it sits in the gutter rather than over a widget.
            ui.style_mut().spacing.scroll = egui::style::ScrollStyle {
                bar_width: 6.0,
                floating_allocated_width: 0.0,
                handle_min_length: 24.0,
                dormant_handle_opacity: 0.55,
                active_handle_opacity: 0.9,
                interact_handle_opacity: 1.0,
                dormant_background_opacity: 0.0,
                active_background_opacity: 0.0,
                interact_background_opacity: 0.0,
                ..egui::style::ScrollStyle::floating()
            };

            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    gutter.show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        match self.tab {
                            Tab::Overview => self.page_overview(ui),
                            Tab::Diagnose => self.full_benchmark_panel(ui),
                            Tab::SystemInfo => self.page_system_info(ui),
                            Tab::Hardware => self.page_hardware(ui),
                            Tab::Memory => self.page_memory(ui),
                            Tab::Storage => self.page_storage(ui),
                            Tab::Tweaks => self.page_tweaks(ui),
                            Tab::Privacy => self.page_privacy(ui),
                            Tab::Troubleshoot => self.page_troubleshoot(ui),
                            Tab::Security => self.page_security(ui),
                            Tab::Repair => self.page_repair(ui),
                            Tab::Tools => self.page_tools(ui),
                            Tab::Startup => self.page_startup(ui),
                            Tab::Backup => self.page_backup(ui),
                            Tab::Settings => self.page_settings(ui),
                        }
                    });
                });
        });
    }

    /// Run an action, or hold it for confirmation if it is destructive.
    ///
    /// Every launch path goes through here so a new destructive action
    /// cannot be added to the lists in `tools.rs` and quietly skip the
    /// prompt by virtue of being called from a different page.
    fn launch_action(&mut self, action: Action) {
        if action.destructive {
            self.confirm_action = Some(action);
            return;
        }
        self.run_action_now(&action);
    }

    fn run_action_now(&mut self, action: &Action) {
        match action.launch() {
            Ok(()) => self.say(format!("Launched {}.", action.name)),
            Err(error) => self.complain(format!("{}: {error}", action.name)),
        }
    }

    /// The prompt for a repair that is awkward to undo.
    ///
    /// These used to run on a single click. Resetting the network stack from
    /// a remote or VPN session drops the connection it was clicked over, and
    /// there is no way back from the other end.
    fn confirm_action_dialog(&mut self, ctx: &egui::Context) {
        let Some(action) = self.confirm_action.clone() else {
            return;
        };
        let raw_accent = self.accent;
        let mut go = false;
        let mut cancel = false;

        let response = Modal::new(Id::new("confirm-action")).show(ctx, |ui| {
            let accent = theme::accent_on(ui, raw_accent);
            ui.set_width(440.0);
            ui.label(RichText::new(&action.name).heading());
            ui.add_space(theme::GAP);
            ui.add(Label::new(RichText::new(&action.what).color(MUTED)).wrap());
            ui.add_space(theme::GAP);
            ui.add(
                Label::new(
                    RichText::new(
                        "This changes the system and is not undone by closing the window. A                          console opens so you can watch what it does.",
                    )
                    .small()
                    .color(theme::warn(ui)),
                )
                .wrap(),
            );
            ui.add_space(theme::GAP_SECTION);
            ui.horizontal(|ui| {
                if chrome::button(ui, chrome::Weight::Danger, None, "Run it", accent).pointer().clicked() {
                    go = true;
                }
                if chrome::button(ui, chrome::Weight::Ghost, None, "Cancel", accent).pointer().clicked() {
                    cancel = true;
                }
            });
        });

        if go {
            self.confirm_action = None;
            self.run_action_now(&action);
        } else if cancel || response.should_close() {
            self.confirm_action = None;
        }
    }

    /// The prompt for the activation scripts.
    ///
    /// This one gets its own wording rather than sharing the repair prompt,
    /// because what it does is categorically different: it downloads a script
    /// over the network and runs it with this process's rights. A dialog that
    /// said only "this changes the system" would be underselling it.
    fn confirm_activation_dialog(&mut self, ctx: &egui::Context) {
        if !self.confirm_activation {
            return;
        }
        let raw_accent = self.accent;
        let mut go = false;
        let mut cancel = false;

        let response = Modal::new(Id::new("confirm-activation")).show(ctx, |ui| {
            let accent = theme::accent_on(ui, raw_accent);
            ui.set_width(460.0);
            ui.label(RichText::new("Run the activation scripts?").heading());
            ui.add_space(theme::GAP);
            ui.add(
                Label::new(RichText::new(
                    "This downloads a script from get.activated.win and runs it with this                      program's rights. It is not part of Wrenchmaster and it is not the Windows                      activation settings page.",
                ))
                .wrap(),
            );
            ui.add_space(theme::GAP);
            ui.add(
                Label::new(
                    RichText::new(
                        "A console window opens with a menu to answer. Nothing happens until                          you choose something in it.",
                    )
                    .small()
                    .color(MUTED),
                )
                .wrap(),
            );
            ui.add_space(theme::GAP_SECTION);
            ui.horizontal(|ui| {
                if chrome::button(ui, chrome::Weight::Danger, None, "Download and run", accent)
                    .pointer().clicked()
                {
                    go = true;
                }
                if chrome::button(ui, chrome::Weight::Ghost, None, "Cancel", accent).pointer().clicked() {
                    cancel = true;
                }
            });
        });

        if go {
            self.confirm_activation = false;
            match run_activation_scripts() {
                Ok(()) => self.say("Opened the activation scripts in a console."),
                Err(error) => {
                    self.complain(format!("Could not start the activation scripts: {error}"))
                }
            }
        } else if cancel || response.should_close() {
            self.confirm_activation = false;
        }
    }

    fn confirm_dialog(&mut self, ctx: &egui::Context) {
        if !self.confirm_clean {
            return;
        }
        let raw_accent = self.accent;
        let total = self.scanned_total();
        let selected = self.selected_count();
        let mut go = false;
        let mut cancel = false;

        let response = Modal::new(Id::new("confirm-clean")).show(ctx, |ui| {
            let accent = theme::accent_on(ui, raw_accent);
            ui.set_width(440.0);
            ui.label(RichText::new("Delete for real?").heading());
            ui.add_space(theme::GAP);
            ui.label(format!(
                "{selected} targets selected. The last scan found {} in {} files.",
                human_bytes(total.bytes),
                total.files
            ));
            ui.add_space(theme::GAP);
            ui.label(
                RichText::new("Files are deleted, not moved to the Recycle Bin.")
                    .color(theme::warn(ui)),
            );
            ui.add_space(theme::GAP_SECTION);
            ui.horizontal(|ui| {
                if ui
                    .add(|ui: &mut Ui| {
                        chrome::button(ui, chrome::Weight::Danger, None, "Delete now", accent)
                    })
                    .pointer()
                    .clicked()
                {
                    go = true;
                }
                if ui.button("Cancel").pointer().clicked() {
                    cancel = true;
                }
            });
        });

        if go {
            self.confirm_clean = false;
            self.dry_run = false;
            self.remember();
            self.start_job(true);
        } else if cancel || response.should_close() {
            self.confirm_clean = false;
        }
    }

    fn confirm_ram_trim_dialog(&mut self, ctx: &egui::Context) {
        let Some(process) = self.pending_ram_trim.clone() else {
            return;
        };
        let mut trim = false;
        let mut cancel = false;
        let response = Modal::new(Id::new("confirm-ram-trim")).show(ctx, |ui| {
            ui.set_width(440.0);
            ui.label(RichText::new("Trim this app's working set?").heading());
            ui.add_space(theme::GAP);
            ui.label(format!(
                "Windows will temporarily remove resident RAM pages from {} (PID {}).",
                process.name, process.pid
            ));
            ui.add_space(theme::GAP);
            ui.label(
                RichText::new(
                    "The app may reload those pages and feel slower. This does not reduce committed memory or fix a memory leak.",
                )
                .color(theme::warn(ui)),
            );
            ui.add_space(theme::GAP_SECTION);
            ui.horizontal(|ui| {
                if ui.button(RichText::new("Trim working set").strong()).pointer().clicked() {
                    trim = true;
                }
                if ui.button("Cancel").pointer().clicked() {
                    cancel = true;
                }
            });
        });

        if trim {
            self.pending_ram_trim = None;
            self.refresh_ram_data();
            let still_same_process = self
                .cpu_monitor
                .processes()
                .iter()
                .find(|(pid, _)| pid.as_u32() == process.pid)
                .is_some_and(|(_, current)| current.name().to_string_lossy() == process.name);
            if !still_same_process {
                self.complain(
                    "That process has closed or changed. Refresh the RAM list and try again.",
                );
                return;
            }
            match ram::trim_working_set(process.pid) {
                Ok(()) => {
                    self.refresh_ram_data();
                    self.say(format!(
                        "Asked Windows to trim {}'s working set.",
                        process.name
                    ));
                }
                Err(error) => self.complain(format!(
                    "Could not trim {}'s working set: {error}",
                    process.name
                )),
            }
        } else if cancel || response.should_close() {
            self.pending_ram_trim = None;
        }
    }

    fn confirm_ram_action_dialog(&mut self, ctx: &egui::Context) {
        let Some(action) = self.pending_ram_action else {
            return;
        };
        let mut run = false;
        let mut cancel = false;
        let response = Modal::new(Id::new("confirm-ram-map-action")).show(ctx, |ui| {
            ui.set_width(460.0);
            ui.label(RichText::new(action.title()).heading());
            ui.add_space(theme::GAP);
            ui.label("This changes memory lists system-wide and is not a routine RAM optimization.");
            ui.add_space(theme::GAP);
            ui.label(RichText::new(action.explanation()).color(theme::warn(ui)));
            if action != ram::RamMapAction::SystemWorkingSet {
                ui.add_space(theme::GAP);
                ui.label(
                    RichText::new("This action uses a Windows native memory-list command. Windows may deny it or change its support across versions.")
                        .small()
                        .color(MUTED),
                );
            }
            ui.add_space(theme::GAP_SECTION);
            ui.horizontal(|ui| {
                if ui.button(RichText::new("Run memory action").strong()).pointer().clicked() {
                    run = true;
                }
                if ui.button("Cancel").pointer().clicked() {
                    cancel = true;
                }
            });
        });

        if run {
            self.pending_ram_action = None;
            match ram::empty_rammap_action(action) {
                Ok(()) => {
                    self.refresh_ram_data();
                    self.say(format!("{} completed.", action.title()));
                }
                Err(error) => self.complain(format!("{} failed: {error}", action.title())),
            }
        } else if cancel || response.should_close() {
            self.pending_ram_action = None;
        }
    }

    // ---------------------------------------------------------------- pages

    fn page_overview(&mut self, ui: &mut Ui) {
        let accent = theme::accent_on(ui, self.accent);
        let scanned = self.scanned_total();
        let junk = self
            .targets
            .iter()
            .any(|t| t.scan.is_some())
            .then_some(scanned.bytes);
        let health = Health::assess(health::Inputs {
            machine: &self.machine,
            junk_bytes: junk,
            cpu_load_average: self.history.cpu.recent_average(30, 15),
            gpu_load_average: self.history.gpu_load.recent_average(30, 15),
            cpu_temperature_average: self.history.cpu_temp.recent_average(30, 15),
            gpu_temperature_average: self.history.gpu_temp.recent_average(30, 15),
            reliability_events: self.reliability_event_count(),
            checks: self.scored_checks(),
        });
        self.cache_health(&health);
        // Checking means something is actually running. The mood alone said
        // "checking" forever once the score came from cache, because nothing
        // was left to finish and nothing ever would.
        let busy = self.diagnose_scan_rx.is_some() || self.probe_rx.is_some();
        // Nothing has produced a figure yet: no cache, no probe, no run. The
        // face stays neutral rather than reading 50 as "needs attention",
        // which is a verdict on a machine nobody has looked at.
        let evaluated = self.settings.last_health_score.is_some()
            || !self.probe_outcomes.is_empty()
            || !self.diagnose_outcomes.is_empty();
        let refresh_incomplete = (busy || !evaluated) && health.mood == health::Mood::Checking;
        let cached_score = self.settings.last_health_score;
        // While the first pass is still running there is a figure to show
        // either way: last session's score if there is one, and a neutral 50
        // if there is not. The ring used to be blank until the checks landed,
        // which left the one thing people open this page for missing for the
        // first few seconds.
        //
        // The cached score is held down to the running assessment rather than
        // shown as-is, so a machine that got worse since last time does not
        // greet the user with yesterday's good news.
        let shown_score = Some(if refresh_incomplete {
            // Deliberately NOT the half-finished assessment, and not the
            // lower of the two either. A sweep in progress only accumulates
            // findings -- no check ever hands back credit -- so a partial
            // score is biased downward by construction, and showing it means
            // watching the number slide for ten seconds before it settles.
            // The honest reading while still looking is last session's
            // figure, or a neutral 50 on a first run.
            cached_score.unwrap_or(health::NEUTRAL_SCORE)
        } else {
            health.score
        });
        // The face stays neutral until the pass finishes, whatever the number
        // reads: a frown drawn from half the evidence is a verdict the tool
        // has not earned yet.
        let shown_mood = if refresh_incomplete {
            health::Mood::Checking
        } else {
            shown_score
                .map(health::Mood::from_score)
                .unwrap_or(health::Mood::Checking)
        };
        // Notes a machine check also reports are left out here, the same way
        // the Diagnose list leaves them out. The drive one in particular was
        // the worst note on this machine, so the Overview card said nothing
        // else for weeks at a time -- a line that never changes stops being
        // read and starts being noise.
        let leading_note = health
            .notes
            .iter()
            .filter(|note| !note.also_a_check)
            .min_by_key(|note| match note.severity {
                health::Severity::Bad => 0,
                health::Severity::Warn => 1,
                health::Severity::Info => 2,
            });
        theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            // Centred against the column beside it, not top-aligned: that
            // column grew a second line and left the ring hanging above it.
            //
            // The height is pinned to the ring's own, because a centring
            // layout measures against whatever space is available and this
            // one is inside a card with a whole page below it -- left to
            // itself it claimed all of it and the card grew to fill the
            // window.
            ui.set_height(84.0);
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                let mood_color = shown_mood.color(ui);
                match shown_score {
                    Some(score) => {
                        chrome::ring(
                            ui,
                            84.0,
                            score as f32 / 100.0,
                            mood_color,
                            &score.to_string(),
                            "of 100",
                        );
                    }
                    None => {
                        chrome::ring(ui, 84.0, 0.0, MUTED, "\u{2014}", "first check");
                    }
                }
                ui.add_space(theme::GAP);
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new(shown_mood.headline())
                            .strong()
                            .color(shown_mood.color(ui)),
                    );
                    ui.add_space(theme::GAP_TIGHT);
                    if refresh_incomplete && cached_score.is_none() {
                        ui.label(
                            RichText::new("Building the first health snapshot…")
                                .small()
                                .color(MUTED),
                        );
                    } else if let Some(delta) = self.health_trend() {
                        // The comparison belongs with the figure: 74 alone
                        // says nothing about whether the machine is getting
                        // better or worse, which is the question somebody
                        // opening this page actually has.
                        let (arrow, color) = if delta > 0 {
                            ("\u{2191}", theme::good(ui))
                        } else {
                            ("\u{2193}", theme::bad(ui))
                        };
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(format!("{arrow} {}", delta.abs()))
                                    .small()
                                    .strong()
                                    .color(color),
                            );
                            if let Some(age) = self.health_age_hours() {
                                ui.label(
                                    RichText::new(if age < 0.05 {
                                        "since the last check, just now".to_owned()
                                    } else {
                                        format!(
                                            "since the last check, {} ago",
                                            crate::util::human_duration(age)
                                        )
                                    })
                                    .small()
                                    .color(MUTED),
                                );
                            }
                        });
                    } else if let Some(age) = self.health_age_hours() {
                        // A score is only worth what its age says it is. The
                        // cache holds for twelve hours, so without this the
                        // page can show this morning's figure as if it were
                        // taken just now.
                        ui.label(
                            RichText::new(if age < 0.05 {
                                "Last checked just now".to_owned()
                            } else {
                                format!("Last checked {} ago", crate::util::human_duration(age))
                            })
                            .small()
                            .color(MUTED),
                        );
                    }
                    if let Some(note) = leading_note.filter(|note| {
                        !refresh_incomplete || note.severity != health::Severity::Info
                    }) {
                        ui.label(
                            RichText::new(&note.text)
                                .small()
                                .color(note.severity.color(ui)),
                        );
                    } else if !refresh_incomplete {
                        ui.label(
                            RichText::new("No urgent issue detected in the latest check.")
                                .small()
                                .color(MUTED),
                        );
                    }
                    // Offered when there is a reason to run one. Above 85
                    // the checks have already been run and found nothing
                    // worth acting on, and a call to action on a healthy
                    // machine is an invitation to go looking for a problem
                    // that is not there. The Diagnose page keeps its own
                    // button, so nothing becomes unreachable.
                    if shown_score.is_none_or(|score| score <= 85)
                        || self.diagnose_scan_rx.is_some()
                    {
                        ui.add_space(theme::GAP_TIGHT);
                        if chrome::button(
                            ui,
                            chrome::Weight::Primary,
                            None,
                            if self.diagnose_scan_rx.is_some() {
                                "Show the diagnostic"
                            } else {
                                "Run diagnostic"
                            },
                            accent,
                        )
                        .pointer()
                        .clicked()
                        {
                            self.tab = Tab::Diagnose;
                            if self.diagnose_scan_rx.is_none() {
                                self.start_diagnosis(true);
                            }
                        }
                    }
                });
                // Centred against the row rather than top-aligned with it:
                // the face has no baseline to share with the text, and
                // hanging off the top of a card that grows with its notes
                // left it drifting.
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    health::face(ui, shown_mood, 68.0);
                });
            });
        });
        ui.add_space(theme::GAP_SECTION);
        chrome::section_heading(ui, chrome::icon::CHART, "System at a glance", accent);
        ui.add_space(theme::GAP);

        let cpu = self.cpu_usage;
        let cpu_trend: Vec<f32> = self.history.cpu.values().collect();
        let gpu = self.history.gpu_load.latest();
        let gpu_trend: Vec<f32> = self.history.gpu_load.values().collect();
        let ram_trend: Vec<f32> = self.history.ram.values().collect();
        let self_cpu_threads = self.cpu_monitor.cpus().len();
        let ram_fraction = self.machine.ram_fraction();
        let ram_text = self.machine.ram_summary();
        let disk = self
            .machine
            .disks
            .iter()
            .find(|disk| crate::machine::is_system_drive_mount(&disk.mount))
            .map(|d| {
                (
                    d.mount.clone(),
                    d.used_fraction(),
                    human_bytes(d.available),
                    human_bytes(d.total),
                )
            });

        let warn_now = theme::warn(ui);
        let bad_now = theme::bad(ui);
        ui.columns(4, |columns| {
            let cpu_color = if cpu > 90.0 { warn_now } else { accent };
            reading_card(
                &mut columns[0],
                "Processor",
                &format!("{cpu:.0}%"),
                cpu_color,
                if cpu > 90.0 { "High" } else { "Normal" },
                cpu / 100.0,
                &format!("{self_cpu_threads} threads"),
                Some(&cpu_trend),
            );

            let gpu_value = gpu.map_or_else(|| "—".to_string(), |load| format!("{load:.0}%"));
            let gpu_color = gpu.filter(|load| *load > 90.0).map_or(accent, |_| warn_now);
            reading_card(
                &mut columns[1],
                "Graphics",
                &gpu_value,
                if gpu.is_some() { gpu_color } else { MUTED },
                if gpu.is_none() {
                    "Unavailable"
                } else if gpu.is_some_and(|load| load > 90.0) {
                    "High"
                } else {
                    "Normal"
                },
                gpu.unwrap_or_default() / 100.0,
                if gpu.is_some() {
                    "GPU load"
                } else {
                    "Sensor unavailable"
                },
                Some(&gpu_trend),
            );

            let ram_color = if ram_fraction > 0.9 { bad_now } else { accent };
            reading_card(
                &mut columns[2],
                "Memory",
                &format!("{:.0}%", ram_fraction * 100.0),
                ram_color,
                if ram_fraction > 0.9 { "High" } else { "Normal" },
                ram_fraction,
                &ram_text,
                Some(&ram_trend),
            );

            match &disk {
                // A drive does not move minute to minute, so it keeps the
                // gauge: a flat sparkline would imply a reading that failed.
                Some((mount, fraction, free, total)) => reading_card(
                    &mut columns[3],
                    &format!("Drive {mount}"),
                    &format!("{:.0}%", fraction * 100.0),
                    if *fraction > 0.9 { bad_now } else { accent },
                    if *fraction > 0.9 {
                        "Low space"
                    } else {
                        "Available"
                    },
                    *fraction,
                    &format!("{free} free of {total}"),
                    None,
                ),
                None => reading_card(
                    &mut columns[3],
                    "Drive",
                    "--",
                    MUTED,
                    "Unavailable",
                    0.0,
                    "No drive reported",
                    None,
                ),
            }
        });
    }

    fn page_system_info(&mut self, ui: &mut Ui) {
        let m = &self.machine;
        let show_activation_button =
            is_development_mode() || m.activation_status != ActivationStatus::Active;
        let mut open_activation_settings = false;
        // Two groups, side by side: what Windows is, and what this machine is.
        // They used to share one card, so the identity rows sat under the
        // Windows heading with an unexplained gap above them.
        let (activation_label, activation_color) = match m.activation_status {
            ActivationStatus::Active => ("Active", theme::good(ui)),
            ActivationStatus::NotActive => ("Not active", theme::warn(ui)),
            ActivationStatus::Unknown => ("Unknown", MUTED),
        };
        let era_unsupported = !m.era.supported();
        ui.columns(2, |columns| {
            theme::card(&columns[0]).show(&mut columns[0], |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new("Windows").font(theme::section_title()).strong());
                ui.add_space(theme::GAP_TIGHT);
                kv(ui, "Edition", &m.windows_name);
                kv(ui, "Version", &m.windows_build);
                kv(ui, "Architecture", &m.windows_arch);
                kv(ui, "Generation", m.era.label());
                ui.horizontal(|ui| {
                    ui.add_sized(
                        [124.0, 18.0],
                        Label::new(RichText::new("Activation").small().color(MUTED))
                            .halign(Align::Min),
                    );
                    chrome::pill(ui, activation_label, activation_color);
                });
                if show_activation_button {
                    ui.add_space(theme::GAP_TIGHT);
                    if ghost(ui, "Run activation scripts (downloads and runs MAS)").pointer().clicked() {
                        open_activation_settings = true;
                    }
                }
                if era_unsupported {
                    ui.add_space(theme::GAP_TIGHT);
                    ui.label(
                        RichText::new(
                            "This Windows generation is outside the range Wrenchmaster is tested on.",
                        )
                        .small()
                        .color(theme::warn(ui)),
                    );
                }
            });

            theme::card(&columns[1]).show(&mut columns[1], |ui| {
                ui.set_width(ui.available_width());
                ui.label(
                    RichText::new("This computer")
                        .font(theme::section_title())
                        .strong(),
                );
                ui.add_space(theme::GAP_TIGHT);
                kv(ui, "Name", &m.host_name);
                kv(ui, "Uptime", &m.uptime);
                kv(ui, "Machine GUID", &m.machine_guid);
            });
        });
        ui.add_space(theme::GAP);
        if open_activation_settings {
            self.confirm_activation = true;
        }
    }

    fn page_hardware(&mut self, ui: &mut Ui) {
        let m = &self.machine;
        let accent = theme::accent_on(ui, self.accent);

        chrome::section_heading(ui, chrome::icon::DEVICES, "Processor", accent);
        ui.add_space(theme::GAP);
        theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(&m.cpu).strong());
            ui.label(RichText::new(&m.cpu_cores).small().color(MUTED));
            ui.add_space(theme::GAP);
            chrome::meter(
                ui,
                self.cpu_usage / 100.0,
                if self.cpu_usage > 90.0 {
                    theme::warn(ui)
                } else {
                    accent
                },
                Some(&format!("Overall load  {:.0}%", self.cpu_usage)),
            );
            let core_loads: Vec<f32> = self
                .cpu_monitor
                .cpus()
                .iter()
                .map(|cpu| cpu.cpu_usage().clamp(0.0, 100.0))
                .collect();
            if !core_loads.is_empty() {
                let warn_cores = theme::warn(ui);
                let _ =
                    chrome::accordion(ui, "cpu-per-thread", "Per logical processor", false, |ui| {
                        ui.columns(4, |columns| {
                            for (index, load) in core_loads.iter().enumerate() {
                                let column = &mut columns[index % columns.len()];
                                chrome::meter(
                                    column,
                                    *load / 100.0,
                                    if *load > 90.0 { warn_cores } else { accent },
                                    Some(&format!("Thread {}  {:.0}%", index + 1, load)),
                                );
                            }
                        });
                    });
            }
            let readings = self
                .hardware_telemetry
                .iter()
                .filter(|reading| reading.hardware_type.eq_ignore_ascii_case("Cpu"))
                .collect::<Vec<_>>();
            if self.hardware_sensor_error.is_none() {
                draw_hardware_sensor_rows(
                    ui,
                    &readings,
                    "No CPU temperature, voltage or power sensors were reported by this provider.",
                );
                if self.sensor_temperature("Cpu").is_none() {
                    ui.add_space(theme::GAP_TIGHT);
                    ui.label(RichText::new(self.temperature_hint()).small().color(
                        if self.elevated {
                            MUTED
                        } else {
                            theme::warn(ui)
                        },
                    ));
                }
            } else {
                ui.label(
                    RichText::new(self.hardware_sensor_error.as_deref().unwrap_or_default())
                        .small()
                        .color(MUTED),
                );
            }
        });

        ui.add_space(theme::GAP);
        {
            let cpu = std::mem::take(&mut self.history.cpu);
            let cpu_t = std::mem::take(&mut self.history.cpu_temp);
            let gpu_t = std::mem::take(&mut self.history.gpu_temp);
            let load_color = theme::series(ui, 0);
            let cpu_temp_color = theme::series(ui, 1);
            let gpu_temp_color = theme::series(ui, 2);
            ui.columns(2, |columns| {
                history_chart(
                    &mut columns[0],
                    &mut self.chart_window,
                    "Processor load",
                    &[("CPU %", &cpu, load_color)],
                    "%",
                );
                history_chart(
                    &mut columns[1],
                    &mut self.chart_window,
                    "Temperatures",
                    &[
                        ("CPU \u{00b0}C", &cpu_t, cpu_temp_color),
                        ("GPU \u{00b0}C", &gpu_t, gpu_temp_color),
                    ],
                    "\u{00b0}C",
                );
            });
            self.history.cpu = cpu;
            self.history.cpu_temp = cpu_t;
            self.history.gpu_temp = gpu_t;
        }

        ui.add_space(theme::GAP_SECTION);
        chrome::section_heading(ui, chrome::icon::CHART, "Graphics", accent);
        ui.add_space(theme::GAP);
        if m.gpus.is_empty() {
            theme::card(ui).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new("No graphics adapters were reported.").color(MUTED));
            });
        }
        for (index, gpu) in m.gpus.iter().enumerate() {
            let gpu_reading = self
                .gpu_telemetry
                .iter()
                .find(|reading| {
                    let device = gpu.to_lowercase();
                    let source = reading.name.to_lowercase();
                    device.contains(&source) || source.contains(&device)
                })
                .or_else(|| {
                    (m.gpus.len() == 1 && self.gpu_telemetry.len() == 1)
                        .then(|| &self.gpu_telemetry[0])
                });
            theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(chrome::icon_text(chrome::icon::DIAGNOSTICS, 20.0, accent));
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new(format!("Graphics device {}", index + 1))
                                .small()
                                .color(MUTED),
                        );
                        ui.label(RichText::new(gpu).strong());
                    });
                });
                let device = gpu.to_lowercase();
                let provider_readings = self
                    .hardware_telemetry
                    .iter()
                    .filter(|reading| {
                        reading.hardware_type.starts_with("Gpu")
                            && (reading.hardware_name.to_lowercase().contains(&device)
                                || device.contains(&reading.hardware_name.to_lowercase()))
                    })
                    .collect::<Vec<_>>();
                if !provider_readings.is_empty() {
                    draw_hardware_sensor_rows(
                        ui,
                        &provider_readings,
                        "The provider reported no temperature, fan, power or voltage readings for this graphics device.",
                    );
                } else if let Some(reading) = gpu_reading {
                    ui.add_space(theme::GAP_TIGHT);
                    ui.horizontal_wrapped(|ui| {
                        let temperature = reading
                            .temperature_c
                            .as_deref()
                            .and_then(|v| v.trim().parse::<f32>().ok());
                        chrome::sensor_chip(
                            ui,
                            "Temperature",
                            &reading
                                .temperature_c
                                .as_deref()
                                .map(|v| format!("{v} °C"))
                                .unwrap_or_else(|| "n/a".into()),
                            temperature.map(|c| chrome::temp_color(ui, c)).unwrap_or(MUTED),
                            temperature.map(chrome::temp_fraction),
                        );
                        let fan = reading
                            .fan_percent
                            .as_deref()
                            .and_then(|v| v.trim().parse::<f32>().ok());
                        chrome::sensor_chip(
                            ui,
                            "Fan",
                            &reading
                                .fan_percent
                                .as_deref()
                                .map(|v| format!("{v}%"))
                                .unwrap_or_else(|| "n/a".into()),
                            if fan.is_some() { ACCENT } else { MUTED },
                            fan.map(|v| v / 100.0),
                        );
                        chrome::sensor_chip(
                            ui,
                            "Power",
                            &reading
                                .power_w
                                .as_deref()
                                .map(|v| format!("{v} W"))
                                .unwrap_or_else(|| "n/a".into()),
                            MUTED,
                            None,
                        );
                        chrome::sensor_chip(
                            ui,
                            "Power limit",
                            &reading
                                .power_limit_w
                                .as_deref()
                                .map(|v| format!("{v} W"))
                                .unwrap_or_else(|| "n/a".into()),
                            MUTED,
                            None,
                        );
                    });
                    ui.label(
                        RichText::new(
                            "Voltage: not reported by the NVIDIA command-line sensor source.",
                        )
                        .small()
                        .color(MUTED),
                    );
                } else {
                    let status = self.gpu_sensor_error.as_deref().unwrap_or(
                        self.hardware_sensor_error
                            .as_deref()
                            .unwrap_or("Waiting for a supported graphics sensor reading."),
                    );
                    ui.label(RichText::new(status).small().color(MUTED));
                }
            });
            ui.add_space(theme::GAP);
        }

        ui.add_space(theme::GAP);
        let mut open_memory = false;
        theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(chrome::icon_text(chrome::icon::MEMORY, 20.0, accent));
                ui.vertical(|ui| {
                    ui.label(RichText::new("Memory").strong());
                    ui.label(RichText::new(m.ram_summary()).small().color(MUTED));
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ghost(ui, "Open Memory").pointer().clicked() {
                        open_memory = true;
                    }
                });
            });
            chrome::meter(
                ui,
                m.ram_fraction(),
                if m.ram_fraction() > 0.9 {
                    theme::warn(ui)
                } else {
                    accent
                },
                None,
            );
        });
        if open_memory {
            self.tab = Tab::Memory;
        }

        ui.add_space(theme::GAP);
        ui.horizontal(|ui| {
            ui.label(chrome::icon_text(chrome::icon::DEVICES, 20.0, accent));
            ui.label(
                RichText::new("Motherboard and firmware")
                    .font(theme::section_title())
                    .strong(),
            );
        });
        ui.add_space(theme::GAP);
        theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            kv(ui, "Motherboard", &m.motherboard);
            kv(ui, "Firmware", &m.bios);
            ui.add_space(theme::GAP_TIGHT);
            ui.add_space(theme::GAP_TIGHT);
            ui.label(
                RichText::new("Board temperatures, fans, voltage and power")
                    .small()
                    .strong(),
            );
            let readings = self
                .hardware_telemetry
                .iter()
                .filter(|reading| {
                    reading.hardware_type.eq_ignore_ascii_case("Motherboard")
                        || (reading.hardware_type.eq_ignore_ascii_case("Controller")
                            && reading.parent_type.eq_ignore_ascii_case("Motherboard"))
                })
                .collect::<Vec<_>>();
            if self.hardware_sensor_error.is_none() {
                draw_hardware_sensor_rows(
                    ui,
                    &readings,
                    "No board temperature, fan-header, voltage or power sensors were reported.",
                );
            } else {
                ui.label(
                    RichText::new(self.hardware_sensor_error.as_deref().unwrap_or_default())
                        .small()
                        .color(MUTED),
                );
            }
        });

        ui.add_space(theme::GAP);
        theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("Cooling, power and voltage availability").strong());
            para(ui, "Live CPU, GPU, memory and motherboard sensors come from LibreHardwareMonitor when the bundled provider reports them. Device support varies; motherboard fan readings stay with the board unless their source identifies a specific device.");
        });

        ui.add_space(theme::GAP);
        ui.label(
            RichText::new("Storage volumes")
                .font(theme::section_title())
                .strong(),
        );
        ui.add_space(theme::GAP);
        if m.disks.is_empty() {
            theme::card(ui).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new("No mounted volumes were reported.").color(MUTED));
            });
        }
        for disk in &m.disks {
            let fraction = disk.used_fraction();
            theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(chrome::icon_text(chrome::icon::STORAGE, 20.0, accent));
                    ui.vertical(|ui| {
                        ui.label(RichText::new(format!("{}  {}", disk.mount, disk.name)).strong());
                        ui.label(RichText::new(&disk.fs).small().color(MUTED));
                    });
                });
                ui.add_space(theme::GAP_TIGHT);
                chrome::meter(
                    ui,
                    fraction,
                    if fraction > 0.9 { theme::bad(ui) } else { accent },
                    Some(&format!(
                        "{} free of {}",
                        human_bytes(disk.available),
                        human_bytes(disk.total)
                    )),
                );
                ui.label(RichText::new("Mounted volume capacity; physical drive temperature and health are not available here.").small().color(MUTED));
            });
            ui.add_space(theme::GAP);
        }
        let mut physical_storage = self
            .hardware_telemetry
            .iter()
            .filter(|reading| reading.hardware_type.eq_ignore_ascii_case("Storage"))
            .map(|reading| reading.hardware_name.clone())
            .collect::<Vec<_>>();
        physical_storage.sort();
        physical_storage.dedup();
        if !physical_storage.is_empty() {
            ui.add_space(theme::GAP);
            ui.label(RichText::new("Physical drive sensors").strong().size(15.0));
            for device in physical_storage {
                let readings = self
                    .hardware_telemetry
                    .iter()
                    .filter(|reading| {
                        reading.hardware_type.eq_ignore_ascii_case("Storage")
                            && reading.hardware_name == device
                    })
                    .collect::<Vec<_>>();
                theme::card(ui).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new(&device).strong());
                    draw_hardware_sensor_rows(
                        ui,
                        &readings,
                        "No temperature, fan, voltage or power reading was reported.",
                    );
                });
            }
        }
    }

    fn page_storage(&mut self, ui: &mut Ui) {
        let accent = theme::accent_on(ui, self.accent);
        if self.storage_view == StorageView::AppLeftovers {
            self.page_app_leftovers(ui);
            return;
        }
        if self.storage_view == StorageView::SizeExplorer {
            self.page_size_explorer(ui);
            return;
        }
        if self.storage_view == StorageView::Cleanup {
            ui.horizontal(|ui| {
                ui.label(chrome::icon_text(chrome::icon::CLEANER, 20.0, accent));
                ui.label(RichText::new("Cleanup").heading());
            });
            ui.add_space(theme::GAP);
            self.page_cleaner(ui);
            return;
        }

        let accent = theme::accent_on(ui, self.accent);
        let m = &self.machine;
        theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(chrome::icon_text(chrome::icon::STORAGE, 20.0, accent));
                ui.label(
                    RichText::new("Drives")
                        .font(theme::section_title())
                        .strong(),
                );
            });
            ui.add_space(theme::GAP_TIGHT);
            if m.disks.is_empty() {
                ui.label(RichText::new("No mounted drives were found.").color(MUTED));
            }
            for disk in &m.disks {
                let fraction = disk.used_fraction();
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("{}  {}", disk.mount, disk.name)).strong());
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(RichText::new(&disk.fs).small().color(MUTED))
                    });
                });
                chrome::meter(
                    ui,
                    fraction,
                    if fraction > 0.9 {
                        theme::bad(ui)
                    } else {
                        accent
                    },
                    Some(&format!(
                        "{} free of {}",
                        human_bytes(disk.available),
                        human_bytes(disk.total)
                    )),
                );
                ui.add_space(theme::GAP);
            }
        });

        ui.add_space(theme::GAP_SECTION);
        ui.label(
            RichText::new("Free up space")
                .font(theme::section_title())
                .strong(),
        );
        ui.add_space(theme::GAP);
        let mut go: Option<StorageView> = None;
        ui.columns(3, |columns| {
            if link_card(
                &mut columns[0],
                chrome::icon::CLEANER,
                accent,
                "Cleanup",
                "Temporary files and caches",
            ) {
                go = Some(StorageView::Cleanup);
            }
            if link_card(
                &mut columns[1],
                chrome::icon::CHART,
                accent,
                "Size Explorer",
                "Find what is taking the room",
            ) {
                go = Some(StorageView::SizeExplorer);
            }
            if link_card(
                &mut columns[2],
                chrome::icon::APPS,
                accent,
                "App Leftovers",
                "Folders uninstallers left behind",
            ) {
                go = Some(StorageView::AppLeftovers);
            }
        });
        if let Some(view) = go {
            self.storage_view = view;
            self.storage_expanded = true;
        }
    }

    fn page_app_leftovers(&mut self, ui: &mut Ui) {
        let accent = theme::accent_on(ui, self.accent);
        ui.horizontal(|ui| {
            ui.label(chrome::icon_text(chrome::icon::APPS, 20.0, accent));
            ui.label(RichText::new("App Leftovers").heading());
        });
        ui.label(
            RichText::new(
                "Review app-related folders and uninstall records that may have been left behind.",
            )
            .small()
            .color(MUTED),
        );
        ui.add_space(theme::GAP);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    self.app_residue_rx.is_none(),
                    Button::new("Scan AppData folders"),
                )
                .pointer()
                .clicked()
            {
                self.start_app_residue_scan();
            }
            if ui
                .add_enabled(
                    self.registry_audit_rx.is_none(),
                    Button::new("Review uninstall records"),
                )
                .pointer()
                .clicked()
            {
                self.start_registry_audit();
            }
        });
        ui.add_space(theme::GAP);
        theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("Read-only review").strong());
            para(ui, "These checks only report possible leftovers. AppData folder names and missing uninstall paths can be misleading, so inspect each exact path or registry key. Wrenchmaster will not delete folders or edit registry records from this page.");
        });

        if self.app_residue_rx.is_some() {
            ui.add_space(theme::GAP);
            theme::card(ui).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Checking AppData folders…");
                });
            });
        }
        if let Some(report) = &self.app_residue_result {
            ui.add_space(theme::GAP);
            ui.label(RichText::new("Possible AppData folders").strong());
            theme::card(ui).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(format!(
                    "{} folders reviewed · {} registered apps compared · {} unreadable entries",
                    report.folders_examined, report.registry_apps_seen, report.unreadable
                ));
                if report.limit_reached {
                    ui.label(
                        RichText::new("A scan limit was reached; results may be incomplete.")
                            .color(theme::warn(ui)),
                    );
                }
                if report.findings.is_empty() {
                    ui.label(
                        RichText::new("No possible leftover folders were found.")
                            .color(theme::good(ui)),
                    );
                }
            });
            for finding in &report.findings {
                theme::card(ui).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(chrome::icon_text(chrome::icon::FOLDER, 16.0, accent));
                        ui.vertical(|ui| {
                            ui.label(RichText::new(&finding.name).strong());
                            ui.label(
                                RichText::new(finding.path.display().to_string())
                                    .small()
                                    .monospace(),
                            );
                            ui.label(
                                RichText::new(format!(
                                    "{}{}",
                                    human_bytes(finding.bytes),
                                    if finding.size_complete {
                                        " estimated"
                                    } else {
                                        " estimated (partial)"
                                    }
                                ))
                                .small()
                                .color(MUTED),
                            );
                            if let Some(elapsed) =
                                finding.last_write.and_then(|time| time.elapsed().ok())
                            {
                                ui.label(
                                    RichText::new(format!("Modified about {elapsed:?} ago"))
                                        .small()
                                        .color(MUTED),
                                );
                            }
                            ui.label(RichText::new(finding.caveat).small().color(theme::warn(ui)));
                        });
                    });
                });
                ui.add_space(theme::GAP_TIGHT);
            }
        }

        if self.registry_audit_rx.is_some() {
            ui.add_space(theme::GAP);
            theme::card(ui).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Reviewing uninstall records…");
                });
            });
        }
        if let Some(report) = &self.registry_audit_result {
            ui.add_space(theme::GAP);
            ui.label(RichText::new("Possible stale uninstall records").strong());
            theme::card(ui).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(format!(
                    "{} registry records checked · {} unreadable",
                    report.records_scanned, report.unreadable_records
                ));
                if report.limit_reached {
                    ui.label(
                        RichText::new("A scan limit was reached; results may be incomplete.")
                            .color(theme::warn(ui)),
                    );
                }
                if report.candidates.is_empty() {
                    ui.label(
                        RichText::new("No likely stale uninstall records were found.")
                            .color(theme::good(ui)),
                    );
                }
            });
            for candidate in &report.candidates {
                theme::card(ui).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new(&candidate.display_name).strong());
                    ui.label(RichText::new(&candidate.key).small().monospace());
                    let hive = match candidate.source_hive {
                        registry_audit::RegistryHive::CurrentUser => "Current user",
                        registry_audit::RegistryHive::LocalMachine => "All users",
                    };
                    let view = match candidate.source_view {
                        registry_audit::RegistryView::Default => "default view",
                        registry_audit::RegistryView::View32 => "32-bit view",
                        registry_audit::RegistryView::View64 => "64-bit view",
                    };
                    ui.label(
                        RichText::new(format!("{hive} · {view}"))
                            .small()
                            .color(MUTED),
                    );
                    if let Some(publisher) = &candidate.publisher {
                        ui.label(RichText::new(publisher).small().color(MUTED));
                    }
                    ui.label(
                        RichText::new(&candidate.reason)
                            .small()
                            .color(theme::warn(ui)),
                    );
                    for evidence in &candidate.evidence {
                        ui.label(RichText::new(format!("• {evidence}")).small().color(MUTED));
                    }
                });
                ui.add_space(theme::GAP_TIGHT);
            }
        }
    }

    fn page_startup(&mut self, ui: &mut Ui) {
        let accent = theme::accent_on(ui, self.accent);
        ui.horizontal(|ui| {
            ui.label(chrome::icon_text(chrome::icon::SCHEDULE, 20.0, accent));
            ui.label(RichText::new("Startup inventory").heading());
        });
        para(
            ui,
            "Read common Run/RunOnce entries and Startup-folder shortcuts. Commands are inspected, never executed or changed.",
        );
        ui.add_space(theme::GAP);
        if ui
            .add_enabled(
                self.startup_scan_rx.is_none(),
                Button::new("Scan startup locations"),
            )
            .pointer()
            .clicked()
        {
            self.start_startup_scan();
        }
        if self.startup_scan_rx.is_some() {
            ui.add_space(theme::GAP);
            theme::card(ui).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Reading startup locations…");
                });
            });
        }
        let Some(report) = &self.startup_scan_result else {
            if self.startup_scan_rx.is_none() {
                ui.add_space(theme::GAP);
                theme::card(ui).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label("Review what may start with Windows");
                    ui.label(
                        RichText::new(
                            "Start a scan to inventory common user and machine startup entries.",
                        )
                        .small()
                        .color(MUTED),
                    );
                });
            }
            return;
        };
        ui.add_space(theme::GAP);
        theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(format!(
                "{} entries · {} unreadable locations",
                report.entries.len(),
                report.unreadable_locations
            ));
            if report.limit_reached {
                ui.label(
                    RichText::new("A scan limit was reached; results may be incomplete.")
                        .color(theme::warn(ui)),
                );
            }
            if report.entries.is_empty() {
                ui.label(
                    RichText::new("No entries were found in the checked locations.")
                        .color(theme::good(ui)),
                );
            }
        });
        for entry in &report.entries {
            let source = match &entry.source {
                startup::StartupSource::Registry { hive, key } => format!(
                    "{} · {key}",
                    match hive {
                        startup::RegistryHive::CurrentUser => "Current user",
                        startup::RegistryHive::LocalMachine => "All users",
                    }
                ),
                startup::StartupSource::StartupFolder { scope } => format!(
                    "{} Startup folder",
                    match scope {
                        startup::StartupFolderScope::CurrentUser => "User",
                        startup::StartupFolderScope::Common => "Shared",
                    }
                ),
            };
            let resolved = entry.status == startup::ResolutionStatus::Resolved;
            theme::card(ui).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(chrome::icon_text(
                        if resolved {
                            chrome::icon::ABOUT
                        } else {
                            chrome::icon::INFO
                        },
                        16.0,
                        if resolved {
                            theme::good(ui)
                        } else {
                            theme::warn(ui)
                        },
                    ));
                    ui.vertical(|ui| {
                        ui.label(RichText::new(&entry.display_name).strong());
                        ui.label(RichText::new(source).small().color(MUTED));
                        ui.label(RichText::new(&entry.command_or_path).small().monospace());
                        if !entry.resolution_note.is_empty() {
                            ui.label(
                                RichText::new(&entry.resolution_note)
                                    .small()
                                    .color(theme::warn(ui)),
                            );
                        }
                    });
                });
            });
            ui.add_space(theme::GAP_TIGHT);
        }
    }

    fn page_size_explorer(&mut self, ui: &mut Ui) {
        let accent = theme::accent_on(ui, self.accent);
        ui.horizontal(|ui| {
            ui.label(chrome::icon_text(chrome::icon::CHART, 20.0, accent));
            ui.label(RichText::new("Size Explorer").heading());
        });
        ui.label(
            RichText::new("Find large folders and files before deciding what to remove.")
                .small()
                .color(MUTED),
        );
        ui.add_space(theme::GAP);

        let accent = theme::accent_on(ui, self.accent);
        let busy = self.size_scan.is_some();
        let mut choose_folder = false;
        let mut rescan = false;
        let mut cancel = false;

        // The drives, offered up front. Picking a folder through a file
        // dialog is three clicks and a decision before anything happens, and
        // the answer is almost always "the system drive" -- so the drives are
        // on the page as one-click targets and the dialog stays for the case
        // the chips do not cover.
        let mut scan_drive: Option<PathBuf> = None;
        theme::widget(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 5.0;
                ui.label(
                    RichText::new("DRIVES")
                        .font(theme::eyebrow())
                        .color(theme::muted(ui)),
                );
                ui.add_space(theme::GAP_TIGHT);
                for disk in &self.machine.disks {
                    let selected = self
                        .size_root
                        .as_ref()
                        .is_some_and(|root| root.as_os_str() == disk.mount.as_str());
                    let used = disk.total.saturating_sub(disk.available);
                    let label = format!(
                        "{}  {} of {}",
                        disk.mount.trim_end_matches('\\'),
                        human_bytes(used),
                        human_bytes(disk.total)
                    );
                    if chrome::segment(ui, &label, selected, accent)
                        .pointer()
                        .clicked()
                        && !busy
                    {
                        scan_drive = Some(PathBuf::from(&disk.mount));
                    }
                }
                ui.add_space(theme::GAP_TIGHT);
                if chrome::segment(ui, "Choose folder\u{2026}", false, accent)
                    .pointer()
                    .clicked()
                    && !busy
                {
                    choose_folder = true;
                }
            });

            if self.size_root.is_some() || busy {
                ui.add_space(theme::GAP_TIGHT);
                ui.horizontal(|ui| {
                    if let Some(root) = &self.size_root {
                        ui.add(
                            Label::new(
                                RichText::new(root.display().to_string())
                                    .font(theme::readout())
                                    .color(MUTED),
                            )
                            .truncate(),
                        );
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if busy {
                            if ghost(ui, "Stop scan").pointer().clicked() {
                                cancel = true;
                            }
                        } else if self.size_root.is_some()
                            && ghost(ui, "Scan again").pointer().clicked()
                        {
                            rescan = true;
                        }
                    });
                });
            }
        });
        if let Some(root) = scan_drive {
            self.start_size_scan(root);
        }

        if choose_folder {
            if let Some(root) = rfd::FileDialog::new().pick_folder() {
                self.start_size_scan(root);
            }
        }
        if rescan {
            if let Some(root) = self.size_root.clone() {
                self.start_size_scan(root);
            }
        }
        if cancel {
            if let Some(scan) = &self.size_scan {
                scan.cancel();
            }
        }

        if busy {
            ui.add_space(theme::GAP);
            theme::widget(ui).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.add_space(theme::GAP_TIGHT);
                    ui.label(
                        RichText::new(format!(
                            "{} entries · {} files · {}",
                            self.size_progress.visited_entries,
                            self.size_progress.files,
                            human_bytes(self.size_progress.bytes)
                        ))
                        .font(theme::readout()),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        // The table below is already filling in from partial
                        // results, so this says what it is rather than
                        // leaving the user wondering whether the numbers on
                        // screen are the final ones.
                        ui.label(
                            RichText::new("Results so far · still scanning")
                                .small()
                                .color(MUTED),
                        );
                    });
                });
            });
        }

        let Some(result) = self.size_result.as_ref() else {
            if !busy && self.size_root.is_none() {
                ui.add_space(theme::GAP);
                theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
                    ui.label(
                        RichText::new("Find what's taking up space")
                            .font(theme::section_title())
                            .strong(),
                    );
                    ui.add_space(theme::GAP_TIGHT);
                    para(
                        ui,
                        "Pick a folder and Wrenchmaster lists its largest subfolders and files. It only reads sizes, and never deletes anything.",
                    );
                    ui.add_space(theme::GAP);
                    // The action belongs in the empty state: an empty screen is
                    // an invitation to act, not a notice that nothing is here.
                    if chrome::button(
                        ui,
                        chrome::Weight::Primary,
                        None,
                        "Choose folder",
                        accent,
                    )
                    .pointer().clicked()
                    {
                        choose_folder = true;
                    }
                });
            }
            return;
        };

        ui.add_space(theme::GAP);
        theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                RichText::new(format!("Folder: {}", result.root.display()))
                    .small()
                    .color(MUTED),
            );
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(format!("{} files", result.progress.files)).strong());
                ui.separator();
                ui.label(format!("{} folders", result.progress.folders));
                ui.separator();
                ui.label(format!("{} scanned", human_bytes(result.progress.bytes)));
                if result.progress.inaccessible > 0 {
                    ui.separator();
                    ui.label(
                        RichText::new(format!("{} items skipped", result.progress.inaccessible))
                            .color(theme::warn(ui)),
                    );
                }
            });
            if result.capped || result.depth_limited || result.cancelled {
                ui.add_space(theme::GAP_TIGHT);
                ui.label(
                    RichText::new(if result.cancelled {
                        "This is a partial result because the scan was stopped."
                    } else {
                        "This is a partial result because a scan safety limit was reached."
                    })
                    .small()
                    .color(theme::warn(ui)),
                );
            }
            if result.progress.skipped_symlinks > 0 {
                ui.label(
                    RichText::new(format!(
                        "{} linked folders skipped to avoid scanning outside the chosen folder.",
                        result.progress.skipped_symlinks
                    ))
                    .small()
                    .color(MUTED),
                );
            }
            if !result.errors.is_empty() {
                let _ = chrome::accordion(
                    ui,
                    "treesize-read-errors",
                    "Some locations could not be read",
                    false,
                    |ui| {
                        for error in result.errors.iter().take(8) {
                            ui.label(RichText::new(error).small().color(MUTED));
                        }
                        if result.omitted_errors > 0 {
                            ui.label(
                                RichText::new(format!(
                                    "{} more access errors not shown.",
                                    result.omitted_errors
                                ))
                                .small()
                                .color(MUTED),
                            );
                        }
                    },
                );
            }
        });

        ui.add_space(theme::GAP);
        let mut show_files = self.size_show_files;
        ui.horizontal(|ui| {
            if ui
                .selectable_label(!show_files, "Folders")
                .pointer()
                .clicked()
            {
                show_files = false;
            }
            if ui
                .selectable_label(show_files, "Largest files")
                .pointer()
                .clicked()
            {
                show_files = true;
            }
            ui.label(
                RichText::new("Sorted from largest to smallest")
                    .small()
                    .color(MUTED),
            );
        });
        self.size_show_files = show_files;

        ui.add_space(theme::GAP);
        let mut selected_folder = self.size_selected_folder;
        if selected_folder >= result.folders.len() {
            selected_folder = 0;
        }
        let mut navigate_to: Option<usize> = None;
        theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            if show_files {
                let files: Vec<_> = result
                    .largest_files
                    .iter()
                    .filter(|file| {
                        size_folder_contains(&result.folders, selected_folder, file.parent)
                    })
                    .take(50)
                    .collect();
                if files.is_empty() {
                    ui.label(RichText::new("No large files retained for this folder.").color(MUTED));
                }
                for file in &files {
                    ui.horizontal(|ui| {
                        ui.label(chrome::icon_text(chrome::icon::FOLDER, 16.0, accent));
                        ui.label(
                            RichText::new(
                                file.path.file_name().unwrap_or_default().to_string_lossy(),
                            )
                            .strong(),
                        );
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(human_bytes(file.bytes));
                        });
                    });
                    ui.label(
                        RichText::new(file.path.display().to_string())
                            .small()
                            .color(MUTED),
                    );
                    ui.add_space(theme::GAP_TIGHT);
                }
                ui.label(
                    RichText::new("Showing the largest retained files inside the selected folder (up to 200 total per scan).")
                        .small()
                        .color(MUTED),
                );
            } else {
                if selected_folder >= result.folders.len() {
                    selected_folder = 0;
                }
                let current = &result.folders[selected_folder];
                let mut ancestry = Vec::new();
                let mut ancestor = Some(selected_folder);
                while let Some(index) = ancestor {
                    ancestry.push(index);
                    ancestor = result.folders.get(index).and_then(|folder| folder.parent);
                }
                ancestry.reverse();

                ui.horizontal_wrapped(|ui| {
                    ui.label(chrome::icon_text(chrome::icon::FOLDER, 15.0, accent));
                    for (position, index) in ancestry.iter().enumerate() {
                        if position > 0 {
                            ui.label(RichText::new("›").color(MUTED));
                        }
                        let label = &result.folders[*index].name;
                        if *index == selected_folder {
                            ui.label(RichText::new(label).strong());
                        } else if ghost(ui, label).pointer().clicked() {
                            navigate_to = Some(*index);
                        }
                    }
                });
                ui.label(
                    RichText::new(current.path.display().to_string())
                        .small()
                        .color(MUTED),
                );
                ui.add_space(theme::GAP);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Name").small().color(MUTED));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(RichText::new("Size · share").small().color(MUTED));
                    });
                });
                ui.separator();

                let root_bytes = current.bytes;
                let Some(folder_children) = result.children.get(selected_folder) else {
                    return;
                };
                if folder_children.is_empty() {
                    ui.label(RichText::new("No subfolders in this folder.").color(MUTED));
                } else {
                    for index in folder_children {
                        let folder = &result.folders[*index];
                        let percent = if root_bytes == 0 {
                            0.0
                        } else {
                            (folder.bytes as f64 / root_bytes as f64 * 100.0) as f32
                        };
                        ui.push_id(*index, |ui| {
                            let response = ui.add_sized(
                                [ui.available_width(), 38.0],
                                Button::new(egui::RichText::new("").size(1.0)).frame(false),
                            );
                            let row_rect = response.rect;
                            let painter = ui.painter();
                            painter.text(
                                egui::pos2(row_rect.left() + 4.0, row_rect.center().y),
                                egui::Align2::LEFT_CENTER,
                                chrome::icon::FOLDER,
                                egui::FontId::proportional(16.0),
                                self.accent,
                            );
                            // The size is laid out first so the name knows
                            // how much room is left. Painted blind, a long
                            // folder name ran straight under the figure and
                            // off the end of the row -- the painter clips
                            // nothing, and the row has no idea the text is
                            // there.
                            let size_text = painter.layout_no_wrap(
                                format!(
                                    "{}  \u{00b7}  {:.1}%",
                                    human_bytes(folder.bytes),
                                    percent
                                ),
                                egui::FontId::proportional(13.0),
                                MUTED,
                            );
                            let name_room = (row_rect.width()
                                - 27.0
                                - 6.0
                                - size_text.size().x
                                - 12.0)
                                .max(40.0);
                            let name = painter.layout(
                                folder.name.clone(),
                                egui::FontId::proportional(14.0),
                                theme::text(ui),
                                name_room,
                            );
                            painter.galley(
                                egui::pos2(
                                    row_rect.left() + 27.0,
                                    row_rect.center().y - 6.0 - name.size().y * 0.5,
                                ),
                                name,
                                theme::text(ui),
                            );
                            painter.galley(
                                egui::pos2(
                                    row_rect.right() - 6.0 - size_text.size().x,
                                    row_rect.center().y - 6.0 - size_text.size().y * 0.5,
                                ),
                                size_text,
                                MUTED,
                            );
                            let bar = egui::Rect::from_min_max(
                                egui::pos2(row_rect.left() + 27.0, row_rect.bottom() - 5.0),
                                egui::pos2(row_rect.right() - 6.0, row_rect.bottom() - 2.0),
                            );
                            painter.rect_filled(bar, 2.0, theme::track(ui));
                            let filled = egui::Rect::from_min_max(
                                bar.min,
                                egui::pos2(
                                    bar.left() + bar.width() * (percent / 100.0).clamp(0.0, 1.0),
                                    bar.bottom(),
                                ),
                            );
                            painter.rect_filled(filled, 2.0, self.accent);
                            let should_navigate = response.pointer().clicked() || response.pointer().double_clicked();
                            response.on_hover_text(format!(
                                "{}\n{} files · {} subfolders\nClick to open",
                                folder.path.display(),
                                folder.file_count,
                                folder.folder_count
                            ));
                            if should_navigate {
                                navigate_to = Some(*index);
                            }
                        });
                    }
                }
            }
        });
        if let Some(index) = navigate_to {
            self.size_selected_folder = index;
        } else {
            self.size_selected_folder = selected_folder;
        }
    }

    /// Everything about RAM in one place: how much is in use right now, what is
    /// holding it, and the actions that give it back. These three used to live
    /// on three unrelated pages -- Hardware, Overview's benchmark and Storage > Cleanup --
    /// which meant three hops to answer one question.
    fn page_memory(&mut self, ui: &mut Ui) {
        let accent = theme::accent_on(ui, self.accent);
        let snapshot = self.ram_snapshot;
        let total = snapshot.map_or(self.machine.ram_total, |memory| memory.physical_total_bytes);
        let available = snapshot.map_or_else(
            || self.machine.ram_total.saturating_sub(self.machine.ram_used),
            |memory| {
                memory
                    .physical_available_bytes
                    .min(memory.physical_total_bytes)
            },
        );
        let used = total.saturating_sub(available);
        let used_fraction = if total == 0 {
            0.0
        } else {
            used as f32 / total as f32
        };
        let used_color = if used_fraction > 0.95 {
            theme::bad(ui)
        } else if used_fraction > 0.85 {
            theme::warn(ui)
        } else {
            accent
        };
        let available_color = if available < total / 10 {
            theme::warn(ui)
        } else {
            theme::good(ui)
        };
        let commit_fraction = snapshot.map(|memory| {
            (memory.commit_used_bytes as f32 / memory.commit_limit_bytes.max(1) as f32)
                .clamp(0.0, 1.0)
        });
        let commit_color = if commit_fraction.is_some_and(|fraction| fraction > 0.9) {
            theme::warn(ui)
        } else if commit_fraction.is_some() {
            accent
        } else {
            MUTED
        };
        chrome::section_heading(ui, chrome::icon::MEMORY, "Memory overview", accent);
        ui.add_space(theme::GAP);
        ui.columns(3, |columns| {
            metric_widget(
                &mut columns[0],
                chrome::icon::MEMORY,
                "In use",
                &format!("{:.0}%", used_fraction * 100.0),
                if used_fraction > 0.95 {
                    "Very high"
                } else if used_fraction > 0.85 {
                    "High"
                } else {
                    "Normal"
                },
                &format!("{} of {}", human_bytes(used), human_bytes(total)),
                Some(used_fraction),
                used_color,
            );
            metric_widget(
                &mut columns[1],
                chrome::icon::CHART,
                "Available",
                &human_bytes(available),
                if available < total / 10 {
                    "Limited"
                } else {
                    "Ready"
                },
                "Physical memory Windows can use now",
                None,
                available_color,
            );
            if let Some(memory) = snapshot {
                let fraction = commit_fraction.unwrap_or_default();
                metric_widget(
                    &mut columns[2],
                    chrome::icon::CHART,
                    "Committed",
                    &format!("{:.0}%", fraction * 100.0),
                    if fraction > 0.9 { "High" } else { "Normal" },
                    &format!(
                        "{} of {} · RAM or page file",
                        human_bytes(memory.commit_used_bytes),
                        human_bytes(memory.commit_limit_bytes)
                    ),
                    Some(fraction),
                    commit_color,
                );
            } else {
                metric_widget(
                    &mut columns[2],
                    chrome::icon::CHART,
                    "Committed",
                    "—",
                    "Unavailable",
                    "Windows memory counters are unavailable",
                    None,
                    MUTED,
                );
            }
        });

        let _ = chrome::accordion(
            ui,
            "memory-temp-sensors",
            "Memory temperature sensors",
            false,
            |ui| {
                let readings = self
                    .hardware_telemetry
                    .iter()
                    .filter(|reading| reading.hardware_type.eq_ignore_ascii_case("Memory"))
                    .collect::<Vec<_>>();
                if self.hardware_sensor_error.is_none() {
                    draw_hardware_sensor_rows(
                        ui,
                        &readings,
                        "This provider did not report a memory temperature sensor.",
                    );
                } else {
                    ui.label(
                        RichText::new("Memory temperature requires a compatible sensor provider.")
                            .small()
                            .color(MUTED),
                    );
                }
            },
        );

        ui.add_space(theme::GAP);
        {
            let ram = std::mem::take(&mut self.history.ram);
            history_chart(
                ui,
                &mut self.chart_window,
                "Memory over time",
                &[("RAM %", &ram, theme::series(ui, 0))],
                "%",
            );
            self.history.ram = ram;
        }

        ui.add_space(theme::GAP);
        self.page_ram_diagnostic(ui);
        ui.add_space(theme::GAP);
        self.page_ram_cleanup(ui);
    }

    fn page_ram_diagnostic(&mut self, ui: &mut Ui) {
        let mut refresh = false;
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("What is holding it")
                    .font(theme::section_title())
                    .strong(),
            );
            if ghost(ui, "Refresh").pointer().clicked() {
                refresh = true;
            }
        });
        if refresh {
            self.refresh_ram_data();
        }
        ui.add_space(theme::GAP);

        theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            if let Some(snapshot) = self.ram_snapshot {
                let total = snapshot.physical_total_bytes;
                let available = snapshot.physical_available_bytes.min(total);
                let in_use = total.saturating_sub(available);
                ui.label(RichText::new("Physical memory").strong());
                // Kernel and drivers are part of what is in use but are not
                // something you can close, so they are split out rather than
                // lumped in with the applications. The rest of the bar is
                // what is actually free to hand out.
                let kernel = snapshot
                    .kernel_paged_bytes
                    .saturating_add(snapshot.kernel_nonpaged_bytes)
                    .min(in_use);
                let apps = in_use.saturating_sub(kernel);
                let pressure = if total == 0 {
                    0.0
                } else {
                    in_use as f64 / total as f64
                };
                let apps_color = if pressure > 0.9 {
                    theme::warn(ui)
                } else {
                    theme::series(ui, 0)
                };
                let kernel_color = theme::series(ui, 1);
                let free_color = theme::track(ui);
                ui.add_space(theme::GAP_TIGHT);
                ui.horizontal(|ui| {
                    ui.add(
                        Label::new(
                            RichText::new(format!(
                                "{} in use of {}",
                                human_bytes(in_use),
                                human_bytes(total)
                            ))
                            .font(theme::readout())
                            .color(MUTED),
                        )
                        .truncate(),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(
                            RichText::new(format!("{:.0}%", pressure * 100.0))
                                .font(theme::readout())
                                .color(apps_color),
                        );
                    });
                });
                ui.add_space(theme::GAP_TIGHT);
                chrome::stack(
                    ui,
                    &[
                        (apps as f32, apps_color),
                        (kernel as f32, kernel_color),
                        (available as f32, free_color),
                    ],
                    14.0,
                );
                ui.add_space(theme::GAP_TIGHT);
                chrome::legend(
                    ui,
                    &[
                        ("Applications", apps_color),
                        ("Kernel and drivers", kernel_color),
                        ("Available", free_color),
                    ],
                );

                ui.add_space(theme::GAP);
                ui.horizontal_wrapped(|ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new("Available").small().color(MUTED));
                        ui.label(human_bytes(available));
                    });
                    ui.separator();
                    ui.vertical(|ui| {
                        ui.label(RichText::new("System cache").small().color(MUTED));
                        ui.label(human_bytes(snapshot.system_cache_bytes));
                    });
                    ui.separator();
                    ui.vertical(|ui| {
                        ui.label(RichText::new("Kernel and drivers").small().color(MUTED));
                        ui.label(human_bytes(
                            snapshot
                                .kernel_paged_bytes
                                .saturating_add(snapshot.kernel_nonpaged_bytes),
                        ));
                    });
                    ui.separator();
                    ui.vertical(|ui| {
                        ui.label(RichText::new("Paged pool").small().color(MUTED));
                        ui.label(human_bytes(snapshot.kernel_paged_bytes));
                    });
                    ui.separator();
                    ui.vertical(|ui| {
                        ui.label(RichText::new("Nonpaged pool").small().color(MUTED));
                        ui.label(human_bytes(snapshot.kernel_nonpaged_bytes));
                    });
                });
                ui.add_space(theme::GAP);
                let commit = snapshot.commit_used_bytes;
                let limit = snapshot.commit_limit_bytes.max(1);
                ui.label(RichText::new("Committed memory").strong());
                chrome::meter(
                    ui,
                    (commit as f32 / limit as f32).clamp(0.0, 1.0),
                    self.accent,
                    Some(&format!(
                        "{} of {}",
                        human_bytes(commit),
                        human_bytes(snapshot.commit_limit_bytes)
                    )),
                );
                ui.label(
                    RichText::new(
                        "Committed memory is Windows' memory promise backed by RAM or the page file; it is not the same as RAM in use.",
                    )
                    .small()
                    .color(MUTED),
                );
                ui.label(
                    RichText::new(
                        "System cache and kernel figures overlap the physical-memory total; they are details, not extra slices to add together.",
                    )
                    .small()
                    .color(MUTED),
                );
            } else {
                ui.label(RichText::new("Windows memory counters are unavailable.").strong());
                if let Some(error) = &self.ram_error {
                    ui.label(RichText::new(error).small().color(MUTED));
                }
            }

            ui.add_space(theme::GAP);
            ui.separator();
            ui.label(RichText::new("Apps using the most RAM").strong());
            ui.label(
                RichText::new(
                    "Working-set size is RAM resident right now. Shared pages can appear in more than one app, so these rows do not add up to total RAM use.",
                )
                .small()
                .color(MUTED),
            );
            if self.ram_processes.is_empty() {
                ui.label(RichText::new("No app memory data is available yet.").color(MUTED));
            }
            for process in self.ram_processes.iter().take(6) {
                ui.horizontal(|ui| {
                    ui.label(&process.name);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(human_bytes(process.working_set_bytes));
                    });
                });
            }

            ui.add_space(theme::GAP);
            if ui.button("Test RAM after restart").pointer().clicked() {
                match crate::util::spawn("mdsched.exe", &[]) {
                    Ok(()) => self.say(
                        "Windows Memory Diagnostic opened. The test runs after you restart.",
                    ),
                    Err(error) => self.complain(format!(
                        "Could not open Windows Memory Diagnostic: {error}"
                    )),
                }
            }
        });
    }

    fn page_ram_cleanup(&mut self, ui: &mut Ui) {
        ui.label(
            RichText::new("Free memory")
                .font(theme::section_title())
                .strong(),
        );
        ui.add_space(theme::GAP_TIGHT);
        theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("Trim memory for one app").strong());
            ui.label(
                RichText::new(
                    "Choose an app using unusually high RAM. Windows already reuses cached memory automatically; trimming is temporary and can make the app reload data.",
                )
                .small()
                .color(MUTED),
            );
            ui.add_space(theme::GAP);

            if self.ram_processes.is_empty() {
                ui.label(RichText::new("No app memory data is available yet.").color(MUTED));
            }
            let processes = self.ram_processes.clone();
            let mut selected = None;
            for process in processes.iter().take(10) {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new(&process.name).strong());
                        ui.label(
                            RichText::new(format!("PID {} · working set", process.pid))
                                .small()
                                .color(MUTED),
                        );
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.button("Trim").pointer().clicked() {
                            selected = Some(process.clone());
                        }
                        ui.label(human_bytes(process.working_set_bytes));
                    });
                });
                ui.add_space(theme::GAP_TIGHT);
            }
            if let Some(process) = selected {
                self.pending_ram_trim = Some(process);
            }
        });

        ui.add_space(theme::GAP);
        let actions = [
            ram::RamMapAction::WorkingSets,
            ram::RamMapAction::SystemWorkingSet,
            ram::RamMapAction::ModifiedPageList,
            ram::RamMapAction::StandbyList,
            ram::RamMapAction::PriorityZeroStandbyList,
        ];
        let mut requested_action = None;
        theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("Windows memory-list actions").strong());
            ui.label(
                RichText::new("These are the five actions from RAMMap’s Empty menu. Each applies across Windows and asks for confirmation.")
                    .small()
                    .color(MUTED),
            );
            ui.add_space(theme::GAP);
            for action in actions {
                ui.horizontal(|ui| {
                    if ui.button(action.title()).pointer().clicked() {
                        requested_action = Some(action);
                    }
                    ui.label(RichText::new(action.explanation()).small().color(MUTED));
                });
            }
        });
        if let Some(action) = requested_action {
            self.pending_ram_action = Some(action);
        }
    }

    fn page_cleaner(&mut self, ui: &mut Ui) {
        let accent = theme::accent_on(ui, self.accent);
        let busy = self.job.is_some();
        let total = self.scanned_total();
        let cleaned = self.cleaned_total();
        let mut scan = false;
        let mut clean = false;
        let mut cancel = false;
        let mut selection_changed = false;

        theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.add_enabled_ui(!busy, |ui| {
                    if chrome::button(ui, chrome::Weight::Primary, None, "Scan selected", accent)
                        .on_hover_text("Measures without touching anything.")
                        .pointer()
                        .clicked()
                    {
                        scan = true;
                    }
                    // A dry run deletes nothing, so it is not dressed as a
                    // destructive action; a real clean is.
                    let (label, weight) = if self.dry_run {
                        ("Clean (dry run)", chrome::Weight::Ghost)
                    } else {
                        ("Clean", chrome::Weight::Danger)
                    };
                    if chrome::button(ui, weight, None, label, accent)
                        .pointer()
                        .clicked()
                    {
                        clean = true;
                    }
                });
                if busy
                    && chrome::button(ui, chrome::Weight::Ghost, None, "Cancel", accent)
                        .pointer()
                        .clicked()
                {
                    cancel = true;
                }
                ui.add_space(theme::GAP);
                if chrome::toggle(ui, &mut self.dry_run, accent)
                    .on_hover_text("Report what would be deleted, and delete nothing.")
                    .changed()
                {
                    selection_changed = true;
                }
                ui.label(RichText::new("Dry run").color(MUTED));
                ui.add_space(theme::GAP);
                ui.label(RichText::new(format!("{} selected", self.selected_count())).color(MUTED));
                ui.label(
                    RichText::new(format!(
                        "{} in {} files",
                        human_bytes(total.bytes),
                        total.files
                    ))
                    .font(theme::readout())
                    .color(MUTED),
                );
            });
            if cleaned.files > 0 {
                ui.add_space(theme::GAP);
                ui.label(
                    RichText::new(format!(
                        "Freed {} across {} files this session.",
                        human_bytes(cleaned.bytes),
                        cleaned.files
                    ))
                    .color(theme::good(ui)),
                );
            }
        });

        ui.add_space(theme::GAP);
        ui.horizontal(|ui| {
            if ghost(ui, "Select all").pointer().clicked() {
                for t in &mut self.targets {
                    t.selected = true;
                }
                selection_changed = true;
            }
            if ghost(ui, "Select none").pointer().clicked() {
                for t in &mut self.targets {
                    t.selected = false;
                }
                selection_changed = true;
            }
            if ghost(ui, "Safe set only")
                .on_hover_text("Per-user caches that need no elevation.")
                .pointer()
                .clicked()
            {
                for t in &mut self.targets {
                    t.selected = !t.needs_admin
                        && t.kind != cleaner::Kind::RecycleBin
                        && t.kind != cleaner::Kind::RegistryValues;
                }
                selection_changed = true;
            }
        });

        ui.add_space(theme::GAP);
        let elevated = self.elevated;
        let hairline = theme::line(ui);
        for target in &mut self.targets {
            theme::row(ui).show(ui, |ui| {
                ui.horizontal(|ui| {
                    if ui.checkbox(&mut target.selected, "").changed() {
                        selection_changed = true;
                    }
                    ui.vertical(|ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(
                                RichText::new(target.name)
                                    .font(theme::card_title())
                                    .strong(),
                            );
                            if target.needs_admin && !elevated {
                                chrome::pill(ui, "needs admin", theme::warn(ui));
                            }
                            if let Some(scan) = target.scan {
                                let summary = if target.counts_entries() {
                                    format!("{} entries", scan.files)
                                } else {
                                    format!("{} · {} files", human_bytes(scan.bytes), scan.files)
                                };
                                ui.label(RichText::new(summary).small().color(if scan.files > 0 {
                                    accent
                                } else {
                                    MUTED
                                }));
                            }
                            if let Some(c) = target.cleaned {
                                let done = if target.counts_entries() {
                                    format!("removed {} entries", c.files)
                                } else {
                                    format!("freed {}", human_bytes(c.bytes))
                                };
                                ui.label(RichText::new(done).small().color(theme::good(ui)));
                            }
                            if target.errors > 0 {
                                ui.label(
                                    RichText::new(format!(
                                        "{} skipped (in use or locked)",
                                        target.errors
                                    ))
                                    .small()
                                    .color(MUTED),
                                );
                            }
                        });
                        ui.label(RichText::new(target.what).small().color(MUTED));
                        ui.label(
                            RichText::new(target.roots_label())
                                .monospace()
                                .small()
                                .color(FAINT),
                        );
                    });
                });
            });
            // One hairline between rows instead of a border around each.
            let y = ui.cursor().min.y;
            ui.painter()
                .hline(ui.min_rect().x_range(), y, egui::Stroke::new(1.0, hairline));
            ui.add_space(theme::GAP_TIGHT);
        }

        if selection_changed {
            self.remember();
        }
        if scan {
            self.start_job(false);
        }
        if clean {
            if self.dry_run {
                self.start_job(true);
            } else {
                self.confirm_clean = true;
            }
        }
        if cancel {
            if let Some(job) = &self.job {
                job.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            self.say("Cancelling…");
        }
    }

    fn page_tweaks(&mut self, ui: &mut Ui) {
        let accent = theme::accent_on(ui, self.accent);
        self.elevation_banner(
            ui,
            "Tweaks marked \"admin\" write to machine-wide settings and stay disabled without administrator rights.",
        );
        let elevated = self.elevated;
        let mut message: Option<Result<String, String>> = None;
        let mut reread = false;
        let mut restart_explorer = false;

        theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                if ui.button("Re-read current values").pointer().clicked() {
                    reread = true;
                }
                if ui
                    .button("Restart Explorer")
                    .on_hover_text("Applies the shell tweaks without signing out.")
                    .pointer()
                    .clicked()
                {
                    restart_explorer = true;
                }
                ui.label(
                    RichText::new("A row marked \"custom\" holds a value neither switch owns.")
                        .small()
                        .color(MUTED),
                );
            });
        });
        ui.add_space(theme::GAP);

        for tweak in self
            .tweaks
            .iter_mut()
            .filter(|tweak| tweak.category == tweaks::Category::General)
        {
            theme::card(ui).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let mut on = tweak.state == State::On;
                    let blocked = tweak.needs_admin() && !elevated;
                    let known_state = matches!(tweak.state, State::On | State::Off);
                    // The same switch used everywhere else, rather than a bare
                    // stock checkbox that reads as an empty dot.
                    let response = ui
                        .add_enabled_ui(!blocked && known_state, |ui| {
                            chrome::toggle(ui, &mut on, accent)
                        })
                        .inner;
                    let response = if blocked {
                        response.on_disabled_hover_text(
                            "Administrator access is required for this action.",
                        )
                    } else if !known_state {
                        response.on_disabled_hover_text(
                            "This value is custom or unreadable. Review it before changing it.",
                        )
                    } else {
                        response
                    };
                    if response.changed() {
                        message = Some(match tweak.set(on) {
                            Ok(()) => {
                                let mut text = format!(
                                    "{} turned {}.",
                                    tweak.name,
                                    if on { "on" } else { "off" }
                                );
                                if let Some(note) = tweak.after.note() {
                                    text.push(' ');
                                    text.push_str(note);
                                    text.push('.');
                                }
                                Ok(text)
                            }
                            Err(e) => Err(e),
                        });
                    }

                    ui.vertical(|ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new(tweak.name).strong());
                            let (color, label) = match tweak.state {
                                State::On => (theme::good(ui), "on"),
                                State::Off => (MUTED, "off"),
                                State::Other => (theme::warn(ui), "custom"),
                                State::Unreadable => (theme::bad(ui), "unreadable"),
                            };
                            ui.label(RichText::new(label).small().color(color));
                            if tweak.needs_admin() {
                                ui.label(RichText::new("admin").small().color(theme::warn(ui)));
                            }
                            if let Some(note) = tweak.after.note() {
                                ui.label(RichText::new(note).small().color(MUTED));
                            }
                        });
                        ui.label(RichText::new(tweak.what).small().color(MUTED));
                        ui.label(
                            RichText::new(tweak.location())
                                .monospace()
                                .small()
                                .color(FAINT),
                        );
                    });
                });
            });
            ui.add_space(theme::GAP);
        }

        if reread {
            for t in &mut self.tweaks {
                t.refresh();
            }
            self.say("Registry re-read.");
        }
        if restart_explorer {
            match tweaks::restart_explorer() {
                Ok(()) => self.say("Explorer restarted."),
                Err(e) => self.complain(format!("Could not restart Explorer: {e}")),
            }
        }
        match message {
            Some(Ok(text)) => self.say(text),
            Some(Err(text)) => self.complain(text),
            None => {}
        }
    }

    fn page_privacy(&mut self, ui: &mut Ui) {
        let accent = theme::accent_on(ui, self.accent);
        self.elevation_banner(
            ui,
            "Most controls affect only your account. Diagnostic data, feedback notifications, and cross-device clipboard policies affect the whole computer and need administrator access.",
        );
        ui.label(
            RichText::new(
                "Each setting shows the local registry value Wrenchmaster reads, not necessarily Windows’ effective setting. Group Policy or device management may override it. Changes are individual; there is no one-click privacy preset.",
            )
            .small()
            .color(MUTED),
        );
        ui.add_space(theme::GAP);
        theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            para(
                ui,
                "Unreadable values are locked. A custom value can be replaced explicitly; removing it only deletes the value at the shown path and does not restore an earlier custom value.",
            );
            ui.add_space(theme::GAP_TIGHT);
            ui.label(
                RichText::new("Create a restore point from Backup before broad changes.")
                    .small()
                    .color(theme::warn(ui)),
            );
        });

        let elevated = self.elevated;
        let mut message: Option<Result<String, String>> = None;
        for (recommendation, heading) in [
            (
                tweaks::Recommendation::Recommended,
                "Recommended for most PCs",
            ),
            (
                tweaks::Recommendation::Optional,
                "Optional · changes convenience",
            ),
            (tweaks::Recommendation::Careful, "Review first"),
        ] {
            let count = self
                .tweaks
                .iter()
                .filter(|tweak| {
                    tweak.category == tweaks::Category::Privacy
                        && tweak.recommendation == recommendation
                })
                .count();
            if count == 0 {
                continue;
            }
            ui.add_space(theme::GAP);
            ui.label(RichText::new(heading).font(theme::section_title()).strong());
            ui.add_space(theme::GAP_TIGHT);
            for tweak in self.tweaks.iter_mut().filter(|tweak| {
                tweak.category == tweaks::Category::Privacy
                    && tweak.recommendation == recommendation
            }) {
                let mut requested = None;
                theme::card(ui).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        let mut on = tweak.state == State::On;
                        let blocked = tweak.needs_admin() && !elevated;
                        let known_state = matches!(tweak.state, State::On | State::Off);
                        let response = ui
                            .add_enabled_ui(!blocked && known_state, |ui| {
                                chrome::toggle(ui, &mut on, accent)
                            })
                            .inner;
                        let response = if blocked {
                            response.on_disabled_hover_text("Administrator access is required.")
                        } else if !known_state {
                            response.on_disabled_hover_text(
                                "This value is custom or unreadable. Review it before changing it.",
                            )
                        } else {
                            response
                        };
                        if response.changed() {
                            requested = Some(on);
                        }

                        ui.vertical(|ui| {
                            ui.horizontal_wrapped(|ui| {
                                ui.label(RichText::new(tweak.name).strong());
                                let (color, state_label) = match tweak.state {
                                    State::On => (theme::good(ui), "on"),
                                    State::Off => (MUTED, "off"),
                                    State::Other => (theme::warn(ui), "custom value"),
                                    State::Unreadable => (theme::bad(ui), "unreadable"),
                                };
                                ui.label(RichText::new(state_label).small().color(color));
                                ui.label(
                                    RichText::new(tweak.recommendation.label()).small().color(
                                        if recommendation == tweaks::Recommendation::Recommended {
                                            theme::good(ui)
                                        } else if recommendation == tweaks::Recommendation::Optional
                                        {
                                            MUTED
                                        } else {
                                            theme::warn(ui)
                                        },
                                    ),
                                );
                                if tweak.needs_admin() {
                                    ui.label(RichText::new("admin").small().color(theme::warn(ui)));
                                }
                                if let Some(note) = tweak.after.note() {
                                    ui.label(RichText::new(note).small().color(MUTED));
                                }
                            });
                            ui.label(RichText::new(tweak.what).small().color(MUTED));
                            ui.label(
                                RichText::new(tweak.location())
                                    .monospace()
                                    .small()
                                    .color(FAINT),
                            );
                            if tweak.state == State::Other {
                                ui.horizontal(|ui| {
                                    if ui
                                        .add_enabled(
                                            !blocked,
                                            egui::Button::new("Apply recommendation"),
                                        )
                                        .on_disabled_hover_text("Administrator access is required.")
                                        .pointer()
                                        .clicked()
                                    {
                                        requested = Some(true);
                                    }
                                    if ui
                                        .add_enabled_ui(!blocked, |ui| {
                                            chrome::button(
                                                ui,
                                                chrome::Weight::Danger,
                                                None,
                                                "Remove value",
                                                accent,
                                            )
                                        })
                                        .inner
                                        .on_disabled_hover_text("Administrator access is required.")
                                        .pointer()
                                        .clicked()
                                    {
                                        requested = Some(false);
                                    }
                                });
                            }
                        });
                    });
                });
                if let Some(on) = requested {
                    message = Some(match tweak.set(on) {
                        Ok(()) => {
                            let mut text =
                                format!("{} turned {}.", tweak.name, if on { "on" } else { "off" });
                            if let Some(note) = tweak.after.note() {
                                text.push(' ');
                                text.push_str(note);
                                text.push('.');
                            }
                            Ok(text)
                        }
                        Err(error) => Err(error),
                    });
                }
                ui.add_space(theme::GAP);
            }
        }
        if let Some(message) = message {
            match message {
                Ok(text) => self.say(text),
                Err(error) => self.complain(error),
            }
        }
    }

    fn page_troubleshoot(&mut self, ui: &mut Ui) {
        let actions = self.troubleshooters.clone();
        self.action_list(ui, &actions);

        if self.diagcabs.is_empty() {
            return;
        }
        ui.add_space(theme::GAP_SECTION);
        ui.label(RichText::new("Local diagnostic packages").strong());
        ui.label(
            RichText::new("Found next to the executable, for the ones you still have.")
                .small()
                .color(MUTED),
        );
        ui.add_space(theme::GAP);

        let diagcabs = self.diagcabs.clone();
        let mut launch: Option<PathBuf> = None;
        for path in &diagcabs {
            theme::card(ui).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    if ui.button("Run").pointer().clicked() {
                        launch = Some(path.clone());
                    }
                    ui.label(
                        RichText::new(path.file_name().unwrap_or_default().to_string_lossy())
                            .monospace(),
                    );
                });
            });
            ui.add_space(theme::GAP);
        }
        if let Some(path) = launch {
            let name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            match crate::util::shell_open(&path.to_string_lossy()) {
                Ok(()) => self.say(format!("Launched {name}.")),
                Err(e) => self.complain(format!("{name}: {e}")),
            }
        }
    }

    /// The live stepper: which check is running, which are done, and how long
    /// each one took.
    ///
    /// Every figure here is measured. The bar is finished checks over the nine
    /// in `CheckId::ALL`, not a timer dressed up as progress, and each duration
    /// comes from the worker's own clock around that check. A sweep where DISM
    /// alone takes fifty seconds therefore sits on one row for fifty seconds,
    /// which is the honest picture: a bar that creeps along regardless is the
    /// thing that makes a slow check look like a hang.
    fn diagnose_progress(&mut self, ui: &mut Ui, accent: Color32) {
        let scanning = self.diagnose_scan_rx.is_some();
        // Which run is on screen: the diagnostic once there is one, the
        // background check before that, and failing both an empty row of what
        // a diagnostic would look at. The page should say what it does before
        // it has done anything.
        let probe_only = !scanning && self.diagnose_outcomes.is_empty();
        let probe_ran = !self.probe_outcomes.is_empty();
        // The probe's six results against all eighteen dots would read as a
        // sweep stuck at 6 of 18 that nothing was going to finish.
        let steps: Vec<diagnose::CheckId> = diagnose::CheckId::ALL
            .into_iter()
            .filter(|id| !(probe_only && probe_ran) || id.in_health_probe())
            .collect();
        let (outcomes, durations) = if probe_only {
            (self.probe_outcomes.clone(), self.probe_durations.clone())
        } else {
            (
                self.diagnose_outcomes.clone(),
                self.diagnose_durations.clone(),
            )
        };
        let total = steps.len();
        let done = outcomes.len();
        let running = self.diagnose_running_check;
        // The sum of the checks, not a clock on the sweep: the two differ by
        // the handful of milliseconds spent passing messages, and the figure
        // worth showing is the one that adds up to the steps beside it.
        let total_elapsed: Duration = durations.iter().map(|(_, elapsed)| *elapsed).sum();

        let marks: Vec<chrome::Step> = steps
            .iter()
            .copied()
            .map(|id| {
                let outcome = outcomes.iter().find(|outcome| outcome.id == id);
                let elapsed = durations
                    .iter()
                    .find(|(done_id, _)| *done_id == id)
                    .map(|(_, elapsed)| *elapsed);
                // The tooltip is the whole verdict, because this row replaced
                // the list that used to carry it: name, what it concluded and
                // what it cost, for the one step being pointed at.
                let hover = match outcome {
                    Some(outcome) => format!(
                        "{} · {}\n{}{}",
                        id.label(),
                        outcome.status.label(),
                        outcome.summary,
                        elapsed
                            .map(|elapsed| format!("\n{}", format_elapsed(elapsed)))
                            .unwrap_or_default()
                    ),
                    None if running == Some(id) => {
                        format!("{} · running now\n{}", id.label(), id.explanation())
                    }
                    None => format!("{} · not run yet\n{}", id.label(), id.explanation()),
                };
                chrome::Step {
                    color: outcome.map_or(accent, |outcome| check_color(ui, outcome.status)),
                    done: outcome.is_some(),
                    hover,
                }
            })
            .collect();
        let active = running.and_then(|running| steps.iter().position(|id| *id == running));

        theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(chrome::icon_text(chrome::icon::TROUBLESHOOT, 15.0, accent));
                ui.add_space(theme::GAP_TIGHT);
                ui.label(
                    // The same word the button used. An action that is
                    // started by "Run diagnostic" and reported by "RUNNING
                    // CHECKS" makes the user join two names for one thing.
                    RichText::new(if scanning {
                        "RUNNING DIAGNOSTIC"
                    } else if done == 0 {
                        "NOT RUN YET"
                    } else if probe_only {
                        "BACKGROUND CHECK"
                    } else {
                        "DIAGNOSTIC COMPLETE"
                    })
                    .font(theme::eyebrow())
                    .color(theme::muted(ui)),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(
                        RichText::new(if scanning || done == 0 {
                            format!("{done} of {total}")
                        } else {
                            format!(
                                "{} in {}",
                                plural(total, "check"),
                                format_elapsed(total_elapsed)
                            )
                        })
                        .font(theme::readout())
                        .color(theme::muted(ui)),
                    );
                });
            });
            ui.add_space(theme::GAP);
            chrome::steps(ui, &marks, active, accent, 26.0);
            ui.add_space(theme::GAP_TIGHT);
            // One line under the track, and it is the only thing on this card
            // that changes while the sweep runs: what is being looked at, and
            // why. Seventeen standing labels would not fit across the window
            // and are a tooltip away when they are wanted.
            let caption = match running {
                Some(id) => format!("{} · {}", id.label(), id.explanation()),
                None if scanning => "Starting the next check\u{2026}".to_owned(),
                None => "Point at a step to see what it found.".to_owned(),
            };
            ui.add(Label::new(RichText::new(caption).small().color(theme::muted(ui))).truncate());
        });
    }

    /// One row per machine check: name, verdict pill, and the detail body
    /// behind an accordion.
    ///
    /// Drawn from a clone of the outcomes rather than borrowing them, because
    /// every row needs `&mut self` for the UI and the borrow checker will not
    /// have both. Nine small structs per frame is cheaper than restructuring
    /// the page around interior mutability.
    ///
    /// Every string that came from Windows is wrapped rather than painted, and
    /// was already truncated by the check that produced it: device instance
    /// paths and DISM output are both longer than this column has ever been.
    /// The nine checks as one scannable list.
    ///
    /// A card per check put nine borders and nine paragraphs of standing
    /// explanation on the page, so the thing you actually came to read --
    /// which checks passed -- was buried in chrome and filler. One card of
    /// rows reads top to bottom in a glance, and each check's explanation
    /// moves inside its own details, where it is available to whoever wants
    /// it and silent for everyone else.
    /// The whole diagnosis as one JSON document.
    ///
    /// Every value here is something a check reported or a counter read. There
    /// is no score and no summary verdict in the file: a consumer that wants
    /// one can rank `status` itself, and inventing a grade to export would be
    /// putting a number on the page that no check produced.
    ///
    /// `generated` is UTC, spelled out in the key, because the app carries no
    /// time zone database and a bare local timestamp in a file that gets
    /// emailed somewhere is a lie waiting to happen.
    fn diagnose_report_json(&self) -> serde_json::Value {
        use serde_json::json;

        let checks: Vec<serde_json::Value> = diagnose::CheckId::ALL
            .into_iter()
            .map(|id| {
                let outcome = self
                    .diagnose_outcomes
                    .iter()
                    .find(|outcome| outcome.id == id);
                let seconds = self
                    .diagnose_durations
                    .iter()
                    .find(|(done_id, _)| *done_id == id)
                    .map(|(_, elapsed)| elapsed.as_secs_f64());
                json!({
                    "check": id.label(),
                    "question": id.explanation(),
                    "status": outcome.map(|outcome| outcome.status.label()),
                    "summary": outcome.map(|outcome| outcome.summary.clone()),
                    "details": outcome.map(|outcome| outcome.details.clone()).unwrap_or_default(),
                    "seconds": seconds,
                })
            })
            .collect();

        let events = self.diagnose_event_result.as_ref().map(|result| {
            json!({
                "status": match &result.status {
                    ScanStatus::Complete => "complete".to_owned(),
                    ScanStatus::Partial => "partial".to_owned(),
                    ScanStatus::Unsupported => "unsupported".to_owned(),
                    ScanStatus::Error(error) => format!("error: {error}"),
                },
                "log_errors": result.log_errors.clone(),
                "findings": result
                    .findings
                    .iter()
                    .map(|finding| {
                        json!({
                            "source": finding.source,
                            "event_id": finding.event_id,
                            "time_local": finding.time,
                            "message": finding.message,
                        })
                    })
                    .collect::<Vec<_>>(),
            })
        });

        json!({
            "tool": "Windows Wrenchmaster",
            "version": env!("CARGO_PKG_VERSION"),
            "generated_utc": tools::timestamp(),
            "elevated": crate::util::is_elevated(),
            "machine": {
                "windows": self.machine.windows_name,
                "build": self.machine.windows_build,
                "architecture": self.machine.windows_arch,
                "cpu": self.machine.cpu,
                "cpu_cores": self.machine.cpu_cores,
                "gpus": self.machine.gpus,
                "motherboard": self.machine.motherboard,
                "bios": self.machine.bios,
                "uptime": self.machine.uptime,
                "uptime_seconds": self.machine.uptime_seconds,
                "ram_total_bytes": self.machine.ram_total,
                "ram_used_bytes": self.machine.ram_used,
                "disks": self
                    .machine
                    .disks
                    .iter()
                    .map(|disk| {
                        json!({
                            "mount": disk.mount,
                            "total_bytes": disk.total,
                            "available_bytes": disk.available,
                        })
                    })
                    .collect::<Vec<_>>(),
            },
            "checks": checks,
            "event_log": events,
        })
    }

    /// Write the diagnosis next to wherever the user points the dialog.
    ///
    /// The file dialog is modal and does block this thread, which is the one
    /// place in the app that is allowed: it is the user's own deliberate
    /// action, nothing is running behind it, and every other save in the app
    /// already works this way.
    fn export_diagnosis(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .set_title("Where should the diagnosis go?")
            .set_file_name(format!(
                "wrenchmaster-diagnosis-{}.json",
                tools::timestamp()
            ))
            .add_filter("JSON", &["json"])
            .save_file()
        else {
            return;
        };
        let report = self.diagnose_report_json();
        match serde_json::to_string_pretty(&report)
            .map_err(|error| error.to_string())
            .and_then(|text| std::fs::write(&path, text).map_err(|error| error.to_string()))
        {
            Ok(()) => self.say(format!("Saved the diagnosis to {}.", path.display())),
            Err(error) => self.complain(format!("Could not save the diagnosis: {error}")),
        }
    }

    /// The report header: what the diagnosis concluded, in the order it
    /// matters, with the figures it is based on beside it.
    ///
    /// Everything here is derived from a check that ran or a reading that was
    /// taken. There is no score, no grade and no invented confidence: the lead
    /// line is the worst finding's own words, and the tiles are counts of
    /// things that either happened or did not.
    /// The short answer: the worst few findings, in plain sentences, and
    /// nothing else.
    ///
    /// Most people opening a diagnosis want to know whether anything is wrong
    /// and what to do about it, in that order, and then to stop reading. The
    /// full page answers a different question -- what was examined and what
    /// each check said -- which is the right page for whoever is actually
    /// fixing the machine and the wrong one for everybody else.
    ///
    /// Nothing here is a summary of a summary: these are the same findings,
    /// ranked the same way, with the long tail cut off rather than rewritten.
    /// Three is the cut because a list you can act on is a list you can hold
    /// in your head, and the count says plainly how many were left out.
    fn diagnose_simple_view(&mut self, ui: &mut Ui, findings: &[Finding]) {
        let shown = findings.iter().take(3).collect::<Vec<_>>();
        theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            if shown.is_empty() {
                ui.horizontal(|ui| {
                    ui.label(chrome::icon_text(chrome::icon::HEART, 18.0, theme::good(ui)));
                    ui.add_space(theme::GAP_TIGHT);
                    ui.add(
                        Label::new(RichText::new(if self.diagnose_outcomes.is_empty() {
                            "Nothing has been checked yet. Run the checks and the results land here."
                        } else {
                            "Nothing needs attention. Every check passed and no reading is out of range."
                        }))
                        .wrap(),
                    );
                });
                return;
            }
            for (index, finding) in shown.iter().enumerate() {
                if index > 0 {
                    ui.add_space(theme::GAP);
                }
                ui.horizontal_top(|ui| {
                    ui.label(
                        RichText::new("\u{25cf}")
                            .color(finding.severity.color(ui)),
                    );
                    ui.vertical(|ui| {
                        ui.add(
                            Label::new(
                                RichText::new(&finding.title)
                                    .font(theme::card_title())
                                    .strong(),
                            )
                            .wrap(),
                        );
                        ui.add(
                            Label::new(RichText::new(&finding.detail).color(theme::muted(ui)))
                                .wrap(),
                        );
                    });
                });
            }
            if findings.len() > shown.len() {
                ui.add_space(theme::GAP);
                ui.add(
                    Label::new(
                        RichText::new(format!(
                            "{} more, in the full view.",
                            findings.len() - shown.len()
                        ))
                        .small()
                        .color(theme::muted(ui)),
                    )
                    .wrap(),
                );
            }
        });
    }

    fn diagnose_findings(&mut self, ui: &mut Ui, accent: Color32, health: &Health) {
        let mut findings: Vec<Finding> = Vec::new();
        for outcome in &self.diagnose_outcomes {
            // A pass, a skip and a check that could not finish are all
            // "nothing to act on" -- the last two are still visible in the
            // check list and red in the stepper, where the fact that they did
            // not answer is the point, but there is no action to rank here.
            if !outcome.status.is_finding() {
                continue;
            }
            let severity = if outcome.status == diagnose::CheckStatus::Problem {
                health::Severity::Bad
            } else {
                health::Severity::Warn
            };
            findings.push(Finding {
                severity,
                title: outcome.id.label().to_owned(),
                detail: outcome.summary.clone(),
                source: "Machine check",
            });
        }
        let checked = !self.diagnose_outcomes.is_empty();
        for note in &health.notes {
            if note.severity == health::Severity::Info {
                continue;
            }
            // A check already said this, in its own words, with a tool button
            // beside it. Listing both put the same drive on the page twice.
            if checked && note.also_a_check {
                continue;
            }
            findings.push(Finding {
                severity: note.severity,
                title: note.text.clone(),
                detail: note.recommendation.to_owned(),
                source: "Live reading",
            });
        }
        findings.sort_by_key(|finding| match finding.severity {
            health::Severity::Bad => 0,
            health::Severity::Warn => 1,
            health::Severity::Info => 2,
        });

        // Counted from the same ranked list the table below is built from,
        // not from the check outcomes alone. Counting only checks meant the
        // tile could read "0 problems" with a red row directly underneath it,
        // because a live reading -- a nearly full Windows drive -- is a
        // finding that no check produced. A figure heading a list has to
        // count that list.
        let problems = findings
            .iter()
            .filter(|finding| finding.severity == health::Severity::Bad)
            .count();
        let warnings = findings
            .iter()
            .filter(|finding| finding.severity == health::Severity::Warn)
            .count();
        let unreadable = self
            .diagnose_outcomes
            .iter()
            .filter(|outcome| {
                matches!(
                    outcome.status,
                    diagnose::CheckStatus::Error | diagnose::CheckStatus::Skipped
                )
            })
            .count();
        let events = self.reliability_event_count();

        // Four figures and a coloured edge, and nothing else. It used to
        // carry the worst finding as a sentence across the top, which was the
        // same text as the first row of the list directly below it -- the
        // page saying the same thing twice, in its largest type, and the
        // reason the block read as cluttered.
        //
        // Shown in both views. In the summary it heads the three findings; in
        // the full report it stands in for the stepper, so the page opens on
        // what was found rather than on how long each check took.
        {
            // No accent bar down the left any more, and the height is the
            // reason as much as the look. The bar was allocated at
            // `available_height`, which inside a card is whatever is left of
            // the page -- so the card stretched itself to several times the
            // height of its own contents and left a field of empty surface
            // under four small figures. With the sentence gone the bar had
            // nothing to mark the start of either.
            theme::widget(ui)
                .inner_margin(Margin::symmetric(18, 12))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    {
                        {
                            // Resolved before the columns close over `ui`: every theme
                            // colour is a function of the live visuals, and reading one
                            // inside the closure borrows what the closure already holds.
                            let problem_color = if problems > 0 { theme::bad(ui) } else { MUTED };
                            let warning_color = if warnings > 0 { theme::warn(ui) } else { MUTED };
                            let event_tile_color = if events.is_some_and(|count| count > 0) {
                                theme::warn(ui)
                            } else {
                                MUTED
                            };
                            let event_tile_value = events
                                .map_or_else(|| "\u{2014}".to_owned(), |count| count.to_string());
                            ui.columns(4, |columns| {
                                stat(
                                    &mut columns[0],
                                    "PROBLEMS",
                                    &problems.to_string(),
                                    problem_color,
                                );
                                stat(
                                    &mut columns[1],
                                    "WARNINGS",
                                    &warnings.to_string(),
                                    warning_color,
                                );
                                stat(
                                    &mut columns[2],
                                    "NOT CHECKED",
                                    &unreadable.to_string(),
                                    MUTED,
                                );
                                stat(
                                    &mut columns[3],
                                    "EVENTS, 24 H",
                                    &event_tile_value,
                                    event_tile_color,
                                );
                            });
                        }
                    }
                });
        }

        // The short view takes the same ranked list and stops after three.
        if self.diagnose_simple {
            ui.add_space(theme::GAP);
            self.diagnose_simple_view(ui, &findings);
            return;
        }

        if findings.is_empty() {
            return;
        }

        ui.add_space(theme::GAP);
        let worst = findings
            .iter()
            .filter(|finding| finding.severity == health::Severity::Bad)
            .count();
        let headline = if worst > 0 {
            format!("{worst} serious, {} in total", findings.len())
        } else {
            format!("{} to look at", findings.len())
        };
        // Open by default, and the only section on the page that is. It is the
        // answer to the question the user came with; the others are evidence
        // for it, and the group rule shuts this one the moment they go looking
        // at any of them.
        let _ = chrome::accordion_panel(
            ui,
            "diagnose-what-went-wrong",
            chrome::icon::DIAGNOSTICS,
            "What went wrong",
            &headline,
            // Red from the closed header when something serious is inside.
            // A section that folds away has one line to say whether it is
            // worth opening, and "3 serious" in the same grey as "none" is
            // that line wasted.
            (worst > 0).then(|| theme::bad(ui)),
            accent,
            true,
            |ui| {
                for (index, finding) in findings.iter().enumerate() {
                    if index > 0 {
                        let (rect, _) = ui.allocate_exact_size(
                            egui::vec2(ui.available_width(), 1.0),
                            egui::Sense::hover(),
                        );
                        ui.painter().hline(
                            rect.x_range(),
                            rect.center().y,
                            egui::Stroke::new(1.0, theme::line(ui)),
                        );
                    }
                    theme::row(ui).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal_top(|ui| {
                            ui.label(
                                RichText::new("\u{25cf}")
                                    .small()
                                    .color(finding.severity.color(ui)),
                            );
                            ui.vertical(|ui| {
                                ui.horizontal(|ui| {
                                    // The provenance is laid out first: the title
                                    // beside it wraps, and a wrapping label left
                                    // to itself takes the whole row.
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        ui.label(
                                            RichText::new(finding.source)
                                                .small()
                                                .color(theme::faint(ui)),
                                        );
                                        ui.with_layout(
                                            Layout::left_to_right(Align::Center),
                                            |ui| {
                                                ui.add(
                                                    Label::new(
                                                        RichText::new(&finding.title).strong(),
                                                    )
                                                    .truncate(),
                                                );
                                            },
                                        );
                                    });
                                });
                                ui.add(
                                    Label::new(
                                        RichText::new(&finding.detail)
                                            .small()
                                            .color(theme::muted(ui)),
                                    )
                                    .wrap(),
                                );
                            });
                        });
                    });
                }
            },
        );
    }

    fn diagnose_check_list(&mut self, ui: &mut Ui, accent: Color32) {
        let outcomes = self.diagnose_outcomes.clone();
        let running = self.diagnose_running_check;
        let scanning = self.diagnose_scan_rx.is_some();
        let done = outcomes.len();
        let total = diagnose::CheckId::ALL.len();

        // The verdict goes in the accordion header. A closed section labelled
        // only "Machine checks" makes every reader open nine rows to find out
        // whether any of them matters; the tally answers that from outside.
        let tally = |wanted: &[diagnose::CheckStatus]| {
            outcomes
                .iter()
                .filter(|outcome| wanted.contains(&outcome.status))
                .count()
        };
        let problems = tally(&[diagnose::CheckStatus::Problem]);
        let warnings = tally(&[
            diagnose::CheckStatus::Warning,
            diagnose::CheckStatus::NeedsAdmin,
        ]);
        let passes = tally(&[diagnose::CheckStatus::Pass]);
        let unknown = tally(&[diagnose::CheckStatus::Skipped, diagnose::CheckStatus::Error]);
        let summary = if scanning {
            format!("{done} of {total} run")
        } else if outcomes.is_empty() {
            "not run yet".to_owned()
        } else {
            let mut parts = Vec::new();
            if problems > 0 {
                parts.push(plural(problems, "problem"));
            }
            if warnings > 0 {
                parts.push(plural(warnings, "warning"));
            }
            if unknown > 0 {
                parts.push(format!("{unknown} not checked"));
            }
            if passes > 0 {
                parts.push(format!("{passes} pass"));
            }
            parts.join(" \u{b7} ")
        };

        // Set by a row and acted on after the closure returns: the rows are
        // drawn from a clone, so `self` is not reachable from inside them.
        let mut requested: Option<Action> = None;
        let mut go_to_cleanup = false;

        let _ = chrome::accordion_panel(
            ui,
            "diagnose-machine-checks",
            chrome::icon::TROUBLESHOOT,
            "Machine checks",
            &summary,
            if problems > 0 {
                Some(theme::bad(ui))
            } else if warnings > 0 {
                Some(theme::warn(ui))
            } else {
                None
            },
            accent,
            false,
            |ui| {
                ui.set_width(ui.available_width());
                for (index, id) in diagnose::CheckId::ALL.into_iter().enumerate() {
                    if index > 0 {
                        // A hairline rather than a gap: the rows are one
                        // list and have to keep reading as one.
                        let (rect, _) = ui.allocate_exact_size(
                            egui::vec2(ui.available_width(), 1.0),
                            egui::Sense::hover(),
                        );
                        ui.painter().hline(
                            rect.x_range(),
                            rect.center().y,
                            egui::Stroke::new(1.0, theme::line(ui)),
                        );
                    }
                    let outcome = outcomes.iter().find(|outcome| outcome.id == id);
                    let verdict = outcome.map(|outcome| check_color(ui, outcome.status));
                    // The verdict is a bar down the left edge rather than a
                    // pill on the right. Eighteen pills spelling Pass, Pass,
                    // Pass down a column is one word repeated, read once and
                    // ignored thereafter; a colour at the start of the row is
                    // taken in by the same glance that finds the row. The
                    // status is still written out in the summary line under
                    // it and in the stepper's tooltip, so nothing is lost but
                    // the repetition.
                    let row = theme::row(ui).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        // The indent past the colour bar is applied once, to
                        // the whole row, rather than to the title alone --
                        // which is what left the summary and the buttons
                        // starting a step to the left of the name they
                        // belong to.
                        ui.horizontal_top(|ui| {
                            ui.add_space(theme::GAP);
                            ui.vertical(|ui| {
                                ui.horizontal(|ui| {
                                    ui.add(
                                        Label::new(RichText::new(id.label()).strong()).truncate(),
                                    );
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        if outcome.is_none() && running == Some(id) {
                                            ui.spinner();
                                        }
                                    });
                                });

                                let Some(outcome) = outcome else {
                                    return;
                                };
                                ui.add(
                                    Label::new(
                                        RichText::new(&outcome.summary)
                                            .small()
                                            .color(theme::muted(ui)),
                                    )
                                    .wrap(),
                                );
                                // The verdict and the tool that acts on it sit
                                // together, because a row reading "a restart is
                                // pending" with no way to go and look at it is
                                // only half an answer.
                                //
                                // Only on a row that found something, though. Seven
                                // "Open Device Manager" buttons down a list of
                                // passes is seven invitations to go and look at
                                // nothing, and it buries the one row that meant it.
                                let worth_opening = !matches!(
                                    outcome.status,
                                    diagnose::CheckStatus::Pass | diagnose::CheckStatus::Skipped
                                );
                                // Right-aligned, on one line. Left-aligned under the
                                // summary they read as another sentence; against the
                                // right edge they read as the row's actions, and two
                                // of them sit together instead of stacking.
                                let tool = id.tool().filter(|_| worth_opening);
                                let cleanup =
                                    worth_opening && id == diagnose::CheckId::StorageSpace;
                                if tool.is_some() || cleanup {
                                    ui.add_space(theme::GAP_TIGHT);
                                    ui.horizontal(|ui| {
                                        ui.with_layout(
                                            Layout::right_to_left(Align::Center),
                                            |ui| {
                                                if let Some((label, target)) = tool {
                                                    if ghost(ui, label).pointer().clicked() {
                                                        requested = Some(tools::shell_action(
                                                            label, target,
                                                        ));
                                                    }
                                                }
                                                if cleanup
                                                    && ghost(ui, "Free up space")
                                                        .pointer()
                                                        .clicked()
                                                {
                                                    go_to_cleanup = true;
                                                }
                                            },
                                        );
                                    });
                                }
                                if !outcome.details.is_empty() {
                                    let _ = chrome::accordion(
                                        ui,
                                        ("diagnose-check", id.label()),
                                        "Details",
                                        false,
                                        |ui| {
                                            for detail in &outcome.details {
                                                ui.add(
                                                    Label::new(RichText::new(detail).small())
                                                        .wrap(),
                                                );
                                            }
                                            ui.add_space(theme::GAP_TIGHT);
                                            ui.add(
                                                Label::new(
                                                    RichText::new(id.explanation())
                                                        .small()
                                                        .color(theme::muted(ui)),
                                                )
                                                .wrap(),
                                            );
                                        },
                                    );
                                }
                            });
                        });
                    });
                    // Painted after the body, because only then is the row's
                    // height known: a check with its details open is several
                    // times the height of one without, and a bar drawn to a
                    // guessed height is the thing that looks broken.
                    if let Some(color) = verdict {
                        let rect = row.response.rect;
                        ui.painter().rect_filled(
                            egui::Rect::from_min_max(
                                egui::pos2(rect.left(), rect.top() + 3.0),
                                egui::pos2(rect.left() + 3.0, rect.bottom() - 3.0),
                            ),
                            egui::CornerRadius::same(2),
                            color,
                        );
                    }
                }

                if !scanning && outcomes.is_empty() {
                    ui.add(
                        Label::new(
                            RichText::new(
                                "Not run yet. Every check only reads: none repairs, \
                                     schedules or restarts anything.",
                            )
                            .small()
                            .color(theme::muted(ui)),
                        )
                        .wrap(),
                    );
                }
            },
        );

        if let Some(action) = requested {
            self.launch_action(action);
        }
        if go_to_cleanup {
            self.tab = Tab::Storage;
            self.storage_view = StorageView::Cleanup;
            self.expand_group(NavGroup::Storage);
        }
    }

    fn full_benchmark_panel(&mut self, ui: &mut Ui) {
        let accent = theme::accent_on(ui, self.accent);
        let scanned = self.scanned_total();
        let junk = self
            .targets
            .iter()
            .any(|target| target.scan.is_some())
            .then_some(scanned.bytes);
        let health = Health::assess(health::Inputs {
            machine: &self.machine,
            junk_bytes: junk,
            cpu_load_average: self.history.cpu.recent_average(30, 15),
            gpu_load_average: self.history.gpu_load.recent_average(30, 15),
            cpu_temperature_average: self.history.cpu_temp.recent_average(30, 15),
            gpu_temperature_average: self.history.gpu_temp.recent_average(30, 15),
            reliability_events: self.reliability_event_count(),
            checks: self.scored_checks(),
        });
        self.cache_health(&health);

        let benchmark_running = self.diagnose_scan_rx.is_some();
        let have_outcomes = !self.diagnose_outcomes.is_empty();
        // The run button alone on its line. The description that used to sit
        // beside it repeated the tab blurb two lines above, and an unwrapped
        // label inside a horizontal row grows the row until the right-aligned
        // button is pushed off the edge of the page.
        let mut export = false;
        ui.horizontal(|ui| {
            // The view switch sits on the left, where it is read before the
            // page under it rather than discovered after scrolling it.
            for (label, simple) in [("Summary", true), ("Full report", false)] {
                if chrome::segment(ui, label, self.diagnose_simple == simple, accent)
                    .pointer()
                    .clicked()
                {
                    self.diagnose_simple = simple;
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if chrome::button(
                    ui,
                    chrome::Weight::Primary,
                    None,
                    if benchmark_running {
                        "Running diagnostic…"
                    } else if have_outcomes {
                        "Run diagnostic again"
                    } else {
                        "Run diagnostic"
                    },
                    accent,
                )
                .pointer()
                .clicked()
                    && !benchmark_running
                {
                    self.start_diagnosis(true);
                }
                // Offered only once there is a diagnosis to export. A button
                // that writes an empty report is a button that produces a file
                // the user then has to work out is empty.
                if have_outcomes && !benchmark_running {
                    ui.add_space(theme::GAP_TIGHT);
                    export = ghost(ui, "Export JSON").pointer().clicked();
                }
            });
        });
        if export {
            self.export_diagnosis();
        }
        ui.add_space(theme::GAP_SECTION);

        // The stepper belongs to the summary, where it is the record of what
        // was looked at and what each check cost. The full report opens on the
        // counts instead and goes straight into the sections: a reader who
        // chose the evidence does not need eighteen dots telling them it was
        // gathered.
        //
        // The exception is a run in progress, in either view. A diagnostic
        // with no visible progress is the thing that makes people click the
        // button again.
        if benchmark_running || self.diagnose_simple {
            self.diagnose_progress(ui, accent);
            ui.add_space(theme::GAP_SECTION);
        }

        // The processor, memory and Windows-drive tiles that used to stand
        // here are gone, and so has the Windows reliability band that replaced
        // them. All of those figures are read live on the Overview and on the
        // System pages, and the reliability count is already one of the four
        // figures in the summary callout below -- a band above it repeating
        // that number was the page using a sixth of the screen to say the
        // same thing twice.
        self.diagnose_findings(ui, accent, &health);

        // Everything below is the evidence behind the summary above, which is
        // exactly what the short view exists not to show.
        if self.diagnose_simple {
            return;
        }

        ui.add_space(theme::GAP_SECTION);
        self.diagnose_check_list(ui, accent);

        ui.add_space(theme::GAP);
        // Live figures, which is why they sit behind a disclosure rather than
        // on the page. The header says what they currently read, so the
        // section can stay shut without hiding the number that would matter.
        let performance_summary = format!(
            "CPU {:.0}% · RAM {:.0}%",
            self.cpu_usage,
            self.machine.ram_fraction() * 100.0
        );
        let _ = chrome::accordion_panel(
            ui,
            "detailed-performance",
            chrome::icon::CHART,
            "Detailed performance readings",
            &performance_summary,
            None,
            accent,
            false,
            |ui| {
                ui.set_width(ui.available_width());
                chrome::meter(
                    ui,
                    self.cpu_usage / 100.0,
                    if self.cpu_usage > 90.0 {
                        theme::warn(ui)
                    } else {
                        self.accent
                    },
                    Some(&format!("CPU load now  {:.0}%", self.cpu_usage)),
                );
                if self.cpu_usage > 90.0 {
                    ui.label(
                        RichText::new("CPU load is currently high; check active processes below.")
                            .color(theme::warn(ui)),
                    );
                }
                let mut cpu_processes: Vec<_> = self
                    .cpu_monitor
                    .processes()
                    .iter()
                    .filter(|(pid, _)| pid.as_u32() > 4 && pid.as_u32() != std::process::id())
                    .map(|(pid, process)| {
                        (
                            process.name().to_string_lossy().into_owned(),
                            pid.as_u32(),
                            process.cpu_usage(),
                        )
                    })
                    .collect();
                cpu_processes.sort_by(|a, b| b.2.total_cmp(&a.2));
                if !cpu_processes.is_empty() {
                    let _ = chrome::accordion(
                        ui,
                        "largest-cpu-users",
                        "Largest current CPU users",
                        false,
                        |ui| {
                            for (name, pid, usage) in cpu_processes.iter().take(5) {
                                ui.label(format!("{name}  ·  {usage:.0}%  ·  PID {pid}"));
                            }
                            ui.label(
                        RichText::new(
                            "Process CPU readings update with Windows process sampling and can briefly lag behind total CPU load.",
                        )
                        .small()
                        .color(MUTED),
                    );
                        },
                    );
                }
                // A failed read still has to be said, because the Memory tile at
                // the top of the page falls back to a different counter and will
                // happily show a figure when this one is unavailable.
                if self.ram_snapshot.is_none() {
                    if let Some(error) = &self.ram_error {
                        ui.label(
                            RichText::new(format!("Memory counters unavailable: {error}"))
                                .small()
                                .color(MUTED),
                        );
                    }
                }
                let system_drive = self
                    .machine
                    .disks
                    .iter()
                    .find(|disk| crate::machine::is_system_drive_mount(&disk.mount));
                match system_drive {
                    Some(disk) if disk.used_fraction() >= 0.90 => ui.label(
                        RichText::new(format!(
                            "Low free space: Windows drive {} is {:.0}% full ({} free)",
                            disk.mount.trim_end_matches('\\'),
                            disk.used_fraction() * 100.0,
                            human_bytes(disk.available)
                        ))
                        .color(theme::warn(ui)),
                    ),
                    Some(_) => ui.label(
                        RichText::new("Windows drive: no low-space warning.")
                            .small()
                            .color(MUTED),
                    ),
                    None => ui.label(
                        RichText::new("Windows system drive could not be identified.")
                            .small()
                            .color(MUTED),
                    ),
                };
                if !self.ram_processes.is_empty() {
                    let _ = chrome::accordion(
                        ui,
                        "largest-memory-users",
                        "Largest process memory use",
                        false,
                        |ui| {
                            for process in self.ram_processes.iter().take(5) {
                                ui.label(format!(
                                    "{}  ·  {}",
                                    process.name,
                                    human_bytes(process.working_set_bytes)
                                ));
                            }
                            ui.label(
                        RichText::new(
                            "Shared memory means these values do not add up to total RAM use.",
                        )
                        .small()
                        .color(MUTED),
                    );
                        },
                    );
                }
                ui.add_space(theme::GAP);
                ui.label(
                    RichText::new("Reported device temperatures")
                        .small()
                        .strong(),
                );
                let temperatures: Vec<_> = self
                    .hardware_telemetry
                    .iter()
                    .filter(|reading| reading.sensor_type.eq_ignore_ascii_case("Temperature"))
                    .collect();
                if temperatures.is_empty() {
                    let explanation = self
                        .hardware_sensor_error
                        .as_deref()
                        .unwrap_or_else(|| self.temperature_hint());
                    ui.label(RichText::new(explanation).small().color(MUTED));
                } else {
                    for reading in temperatures.iter().take(12) {
                        // Celsius is written here rather than taken from the
                        // provider, whose unit text does not survive decoding.
                        // Readings arrive as 51.40625; one decimal is plenty.
                        let celsius = reading.value.trim().parse::<f32>().ok();
                        let shown = match celsius {
                            Some(v) => format!("{v:.1} °C"),
                            None => reading.value.clone(),
                        };
                        let color = celsius.map_or(MUTED, |c| chrome::temp_color(ui, c));
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(format!(
                                    "{} · {}",
                                    reading.hardware_name, reading.sensor_name
                                ))
                                .small()
                                .color(MUTED),
                            );
                            ui.label(RichText::new(shown).font(theme::readout()).color(color));
                        });
                    }
                }
            },
        );

        ui.add_space(theme::GAP_SECTION);
        let event_summary = if self.diagnose_events_pending() {
            "Checking…".to_string()
        } else if let Some(result) = &self.diagnose_event_result {
            match &result.status {
                ScanStatus::Complete if result.findings.is_empty() => "No errors found".into(),
                ScanStatus::Complete => format!("{} events to review", result.findings.len()),
                ScanStatus::Partial => "Partial check".into(),
                ScanStatus::Unsupported => "Unavailable".into(),
                ScanStatus::Error(_) => "Could not check".into(),
            }
        } else {
            "Not checked yet".to_string()
        };
        let _ = chrome::accordion_panel(
            ui,
            "windows-event-log",
            chrome::icon::EVENTS,
            "Windows event log",
            &event_summary,
            // Amber only when there is something in the log. The count is a
            // finding, not a fault: events get investigated, not fixed.
            self.reliability_event_count()
                .is_some_and(|count| count > 0)
                .then(|| theme::warn(ui)),
            accent,
            false,
            |ui| {
                ui.set_width(ui.available_width());
                ui.label(
                    RichText::new(
                        "System and Application logs · Critical and Error events · past 24 hours",
                    )
                    .small()
                    .color(MUTED),
                );
                if self.diagnose_events_pending() {
                    ui.add_space(theme::GAP);
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Reading Windows events…");
                    });
                } else if let Some(result) = &self.diagnose_event_result {
                    match &result.status {
                        ScanStatus::Complete => ui.label(format!(
                            "Checked both logs. {} found.",
                            plural(result.findings.len(), "critical or error event")
                        )),
                        ScanStatus::Partial => ui.label(
                            RichText::new(format!(
                                "Partial event check. {} found; some log data could not be read.",
                                plural(result.findings.len(), "event")
                            ))
                            .color(theme::warn(ui)),
                        ),
                        ScanStatus::Unsupported => ui.label(
                            RichText::new("Windows event checks are available on Windows only.")
                                .color(MUTED),
                        ),
                        ScanStatus::Error(error) => ui.label(
                            RichText::new(format!("Could not check Windows events: {error}"))
                                .color(theme::warn(ui)),
                        ),
                    };
                    for error in &result.log_errors {
                        ui.label(RichText::new(error).small().color(theme::warn(ui)));
                    }
                    if result.findings.is_empty() && matches!(&result.status, ScanStatus::Complete)
                    {
                        ui.add_space(theme::GAP_TIGHT);
                        ui.label(RichText::new("No Critical or Error events were recorded in the selected logs during this period.").color(theme::good(ui)));
                    } else if !result.findings.is_empty() {
                        ui.add_space(theme::GAP);
                        ui.label(RichText::new("These entries are clues to investigate; a log event alone does not prove the cause of a slowdown.").small().color(MUTED));
                        ui.label(RichText::new("Next: compare repeated event sources and timestamps with the freezes, crashes, or slowdowns you noticed.").small().color(MUTED));
                        egui::ScrollArea::vertical().max_height(280.0).show(ui, |ui| {
                        for (index, finding) in result.findings.iter().take(30).enumerate() {
                            draw_diagnostic_event(ui, index, finding);
                        }
                        if result.findings.len() > 30 {
                            ui.label(RichText::new(format!("Showing 30 of {} events (up to 100 per log are checked).", result.findings.len())).small().color(MUTED));
                        }
                    });
                    }
                }
            },
        );
    }

    fn page_security(&mut self, ui: &mut Ui) {
        let accent = theme::accent_on(ui, self.accent);
        theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(chrome::icon_text(chrome::icon::DEVICES, 22.0, accent));
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new("Windows file integrity and Explorer processes").strong(),
                    );
                    para(ui, "Checks the Windows Explorer signature and lists running Explorer instances with their paths, sessions, and signatures. Multiple instances alone are not treated as suspicious.");
                });
            });
        });
        ui.add_space(theme::GAP);
        let checking_integrity = self.explorer_integrity_rx.is_some();
        ui.horizontal(|ui| {
            if ui.add_enabled(!checking_integrity, Button::new("Check Explorer integrity")).pointer().clicked() {
                self.explorer_integrity_result = None;
                self.explorer_integrity_rx = Some(security_integrity::start_scan());
            }
            if ui.button("Verify Windows copy with SFC").on_hover_text("Runs the Windows verify-only check in a console; it does not repair files.").pointer().clicked() {
                match security_integrity::launch_system_explorer_verify() {
                    Ok(()) => self.say("Started SFC verify-only check for the Windows Explorer file. Review its console output."),
                    Err(error) => self.complain(format!("Could not start SFC: {error}")),
                }
            }
        });
        if checking_integrity {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new("Checking signatures and process paths…").color(MUTED));
            });
        }
        if let Some(result) = &self.explorer_integrity_result {
            let scan_complete = matches!(&result.status, security_integrity::ScanStatus::Complete);
            let signature_valid = result.system_signature_status.eq_ignore_ascii_case("Valid");
            let status_color = if scan_complete && signature_valid {
                theme::good(ui)
            } else {
                theme::warn(ui)
            };
            theme::widget(ui).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(chrome::icon_text(chrome::icon::SHIELD, 22.0, status_color));
                    ui.vertical(|ui| {
                        ui.label(RichText::new("Windows Explorer integrity").strong());
                        ui.label(plural(result.processes.len(), "running instance"));
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        chrome::pill(
                            ui,
                            if scan_complete && signature_valid {
                                "Verified"
                            } else {
                                "Review"
                            },
                            status_color,
                        );
                    });
                });
            });
            let _ = chrome::accordion(
                ui,
                "explorer-exe-details",
                "Executable path and process details",
                false,
                |ui| {
                    ui.label(
                        RichText::new(&result.system_explorer_path)
                            .small()
                            .monospace(),
                    );
                    ui.label(
                        RichText::new(format!(
                            "Authenticode: {}{}",
                            result.system_signature_status,
                            if result.system_signer.is_empty() {
                                String::new()
                            } else {
                                format!(" · {}", result.system_signer)
                            }
                        ))
                        .small()
                        .color(status_color),
                    );
                    if let security_integrity::ScanStatus::Error(error) = &result.status {
                        ui.label(RichText::new(error).small().color(theme::warn(ui)));
                    }
                    for process in &result.processes {
                        let expected_path = process
                            .image_path
                            .eq_ignore_ascii_case(&result.system_explorer_path);
                        let signature_valid =
                            process.signature_status.eq_ignore_ascii_case("Valid");
                        let suspicious_context = !expected_path || !signature_valid;
                        theme::row(ui).show(ui, |ui| {
                            ui.label(
                                RichText::new(format!(
                                    "PID {} · parent {} · session {}",
                                    process.pid, process.parent_pid, process.session_id
                                ))
                                .strong(),
                            );
                            ui.label(
                                RichText::new(&process.image_path)
                                    .small()
                                    .monospace()
                                    .color(if suspicious_context {
                                        theme::warn(ui)
                                    } else {
                                        MUTED
                                    }),
                            );
                            ui.label(
                                RichText::new(format!(
                                    "Authenticode: {}{}{}",
                                    process.signature_status,
                                    if process.signer.is_empty() {
                                        String::new()
                                    } else {
                                        format!(" · {}", process.signer)
                                    },
                                    if !expected_path {
                                        " · unexpected image path"
                                    } else {
                                        ""
                                    }
                                ))
                                .small()
                                .color(if suspicious_context {
                                    theme::warn(ui)
                                } else {
                                    theme::good(ui)
                                }),
                            );
                        });
                    }
                    if !result.notes.is_empty() {
                        let _ = chrome::accordion(
                            ui,
                            "explorer-scanner-notes",
                            "Scanner notes",
                            false,
                            |ui| {
                                for note in &result.notes {
                                    ui.label(RichText::new(note).small().color(MUTED));
                                }
                            },
                        );
                    }
                },
            );
        }

        ui.add_space(theme::GAP_SECTION);
        theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(chrome::icon_text(chrome::icon::DEVICES, 21.0, accent));
                ui.vertical(|ui| {
                    ui.label(RichText::new("Network threat review").strong());
                    para(ui, "Takes a read-only snapshot of TCP listeners and active connections, grouped into inbound and outbound activity. It checks process path, signature status, and unusual ports for review. This does not inspect packet contents or block traffic.");
                });
            });
        });
        ui.add_space(theme::GAP);
        let scanning_network = self.network_scan_rx.is_some();
        if ui
            .add_enabled(!scanning_network, Button::new("Refresh network activity"))
            .pointer()
            .clicked()
        {
            self.start_network_scan();
        }
        if scanning_network {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new("Reading Windows TCP connection metadata…").color(MUTED));
            });
        }
        if let Some(result) = &self.network_scan_result {
            let status = match &result.status {
                network_watch::ScanStatus::Complete => "Snapshot complete",
                network_watch::ScanStatus::Partial => "Snapshot partial",
                network_watch::ScanStatus::Unsupported => "Unsupported system",
                network_watch::ScanStatus::Error(_) => "Snapshot failed",
            };
            ui.label(RichText::new(status).color(
                if matches!(&result.status, network_watch::ScanStatus::Complete) {
                    theme::good(ui)
                } else {
                    theme::warn(ui)
                },
            ));
            if let network_watch::ScanStatus::Error(error) = &result.status {
                ui.label(RichText::new(error).small().color(theme::warn(ui)));
            }
            for note in &result.notes {
                ui.label(RichText::new(note).small().color(MUTED));
            }
            for (direction, title, description) in [
                (
                    NetworkDirection::IncomingListener,
                    "Inbound listeners",
                    "Programs waiting for incoming TCP connections; a listener does not mean a remote connection was accepted.",
                ),
                (
                    NetworkDirection::IncomingEstablished,
                    "Inbound connections",
                    "Established connections associated with a local listening port.",
                ),
                (
                    NetworkDirection::Outgoing,
                    "Outbound connections",
                    "Established TCP connections initiated by local programs.",
                ),
            ] {
                let connections = result
                    .connections
                    .iter()
                    .filter(|connection| connection.direction == direction)
                    .collect::<Vec<_>>();
                let _ = chrome::accordion(
                    ui,
                    ("network-connections", title),
                    &format!("{title} · {}", connections.len()),
                    !connections.is_empty(),
                    |ui| {
                        ui.label(RichText::new(description).small().color(MUTED));
                        if connections.is_empty() {
                            ui.label(
                                RichText::new("Nothing found in this category.")
                                    .small()
                                    .color(MUTED),
                            );
                        }
                        for connection in connections.iter().take(20) {
                            let (severity, color) = match connection.severity {
                                NetworkSeverity::Elevated => ("Check", theme::bad(ui)),
                                NetworkSeverity::Review => ("Review", theme::warn(ui)),
                                NetworkSeverity::Info => ("Info", MUTED),
                            };
                            theme::card(ui).show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.horizontal(|ui| {
                                    ui.label(chrome::icon_text(chrome::icon::SHIELD, 16.0, color));
                                    ui.label(RichText::new(severity).strong().color(color));
                                    ui.label(
                                        RichText::new(format!(
                                            "PID {} · {}",
                                            connection.process_id, connection.process_name
                                        ))
                                        .strong(),
                                    );
                                });
                                if connection.direction == NetworkDirection::IncomingListener {
                                    ui.label(
                                        RichText::new(format!(
                                            "Listening on {}:{}",
                                            connection.local_address, connection.local_port
                                        ))
                                        .small()
                                        .monospace(),
                                    );
                                } else {
                                    ui.label(
                                        RichText::new(format!(
                                            "{}:{}  →  {}:{}",
                                            connection.local_address,
                                            connection.local_port,
                                            connection.remote_address,
                                            connection.remote_port
                                        ))
                                        .small()
                                        .monospace(),
                                    );
                                }
                                if !connection.executable_path.is_empty() {
                                    ui.label(
                                        RichText::new(&connection.executable_path)
                                            .small()
                                            .monospace()
                                            .color(MUTED),
                                    );
                                }
                                ui.label(
                                    RichText::new(format!(
                                        "Signature: {} · {}",
                                        connection.signature, connection.explanation
                                    ))
                                    .small()
                                    .color(color),
                                );
                            });
                        }
                        if connections.len() > 20 {
                            ui.label(
                                RichText::new(format!(
                                    "Showing 20 of {} entries in this category.",
                                    connections.len()
                                ))
                                .small()
                                .color(MUTED),
                            );
                        }
                    },
                );
            }
            para(
                ui,
                "A connection or unusual port is not proof of malware. Use Defender for a full system scan. This view is a snapshot, not continuous packet monitoring.",
            );
        }

        theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(chrome::icon_text(chrome::icon::SHIELD, 24.0, accent));
                ui.vertical(|ui| {
                    ui.label(RichText::new("Microsoft Defender scans").strong());
                    ui.label(
                        RichText::new(
                            "Quick and full scans use Defender's installed security intelligence to check for known threats. Review scan status and detections in Windows Security.",
                        )
                        .small()
                        .color(MUTED),
                    );
                });
            });
        });
        ui.add_space(theme::GAP);
        self.elevation_banner(ui, "Starting Defender scans requires administrator rights.");
        let actions = self.security_actions.clone();
        self.action_list(ui, &actions);

        ui.add_space(theme::GAP_SECTION);
        ui.horizontal(|ui| {
            ui.label(chrome::icon_text(chrome::icon::SHIELD, 20.0, accent));
            ui.label(RichText::new("Online malware hash lookup").strong());
        });
        para(
            ui,
            "MalwareBazaar checks a selected file's SHA-256 against its malware database. The file is hashed locally; only its hash is sent. This is opt-in and requires an Auth-Key. No file contents are uploaded or samples downloaded.",
        );
        ui.add_space(theme::GAP_TIGHT);
        ui.hyperlink_to(
            "MalwareBazaar API and key information",
            "https://bazaar.abuse.ch/api/",
        );
        ui.horizontal(|ui| {
            if ui.button("Choose file…").pointer().clicked() {
                if let Some(path) = rfd::FileDialog::new().pick_file() {
                    self.malware_lookup_path = Some(path);
                    self.malware_lookup_result = None;
                }
            }
            ui.label(
                RichText::new(
                    self.malware_lookup_path
                        .as_ref()
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|| "No file selected".into()),
                )
                .small()
                .monospace()
                .color(MUTED),
            );
        });
        ui.horizontal(|ui| {
            ui.label("Auth-Key");
            ui.add(
                egui::TextEdit::singleline(&mut self.malware_lookup_key)
                    .password(true)
                    .desired_width(280.0)
                    .hint_text("used once; not saved"),
            );
            let busy = self.malware_lookup_rx.is_some();
            if ui
                .add_enabled(!busy, Button::new("Check hash online"))
                .pointer()
                .clicked()
            {
                self.start_malware_lookup();
            }
            if busy {
                ui.spinner();
            }
        });
        if let Some(result) = &self.malware_lookup_result {
            theme::card(ui).show(ui, |ui| match result {
                Ok(lookup) => {
                    let (label, color) = match &lookup.status {
                        LookupStatus::Match => ("Database match — review this file", theme::bad(ui)),
                        LookupStatus::NotFound => ("No MalwareBazaar record found", MUTED),
                        LookupStatus::ApiIssue => ("Lookup could not be completed", theme::warn(ui)),
                    };
                    ui.label(RichText::new(label).strong().color(color));
                    if let Some(hash) = &lookup.sha256 { ui.label(RichText::new(format!("SHA-256: {hash}")).small().monospace()); }
                    ui.label(RichText::new(&lookup.sanitized_json).small().monospace());
                    para(ui, "A database miss does not prove a file is safe; a match needs review and is not an automatic quarantine decision.");
                }
                Err(error) => { ui.label(RichText::new(error).color(theme::warn(ui))); }
            });
        }

        ui.add_space(theme::GAP);
        ui.label(RichText::new("Recent MalwareBazaar feed scan").strong());
        ui.label(RichText::new(format!("Fetches metadata for the latest 100 MalwareBazaar records, then compares local SHA-256 hashes for common executable and script files in one selected folder. Maximum {} files, {} each, 60 seconds. This recent feed is limited coverage, not a full antivirus scan.", malware_lookup::RECENT_SCAN_MAX_FILES, human_bytes(malware_lookup::RECENT_SCAN_MAX_FILE_BYTES))).small().color(MUTED));
        ui.horizontal(|ui| {
            if ui.button("Choose folder…").pointer().clicked() {
                if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                    self.malware_feed_scan_folder = Some(folder);
                    self.malware_feed_scan_result = None;
                }
            }
            ui.label(
                RichText::new(
                    self.malware_feed_scan_folder
                        .as_ref()
                        .map(|path| path.display().to_string())
                        .unwrap_or_else(|| "No folder selected".into()),
                )
                .small()
                .monospace()
                .color(MUTED),
            );
        });
        ui.horizontal(|ui| {
            ui.label("Auth-Key");
            ui.add(
                egui::TextEdit::singleline(&mut self.malware_lookup_key)
                    .password(true)
                    .desired_width(280.0)
                    .hint_text("used once; not saved"),
            );
            let busy = self.malware_feed_scan_rx.is_some();
            if ui
                .add_enabled(!busy, Button::new("Scan folder against recent feed"))
                .pointer()
                .clicked()
            {
                self.start_malware_feed_scan();
            }
            if busy {
                ui.spinner();
            }
        });
        if self.malware_feed_scan_rx.is_some() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(
                    RichText::new("Fetching hashes and scanning selected files locally…")
                        .color(MUTED),
                );
            });
        }
        if let Some(result) = &self.malware_feed_scan_result {
            theme::card(ui).show(ui, |ui| match result {
                Ok(report) => {
                    if report.matches.is_empty() {
                        ui.label(
                            RichText::new("No hash matches in the recent feed.")
                                .strong()
                                .color(MUTED),
                        );
                    } else {
                        ui.label(
                            RichText::new(format!(
                                "{} recent-feed match(es) — review these files",
                                report.matches.len()
                            ))
                            .strong()
                            .color(theme::bad(ui)),
                        );
                    }
                    ui.label(format!(
                        "{} files hashed · {} skipped · {} seconds",
                        report.files_examined, report.files_skipped, report.elapsed_seconds
                    ));
                    if report.truncated_by_file_limit || report.truncated_by_time_limit {
                        ui.label(
                            RichText::new(
                                "The scan reached a safety limit, so results are partial.",
                            )
                            .color(theme::warn(ui)),
                        );
                    }
                    for finding in &report.matches {
                        ui.label(
                            RichText::new(&finding.path)
                                .small()
                                .monospace()
                                .color(theme::warn(ui)),
                        );
                        ui.label(
                            RichText::new(format!("SHA-256: {}", finding.sha256))
                                .small()
                                .monospace()
                                .color(MUTED),
                        );
                    }
                    ui.label(RichText::new(report.coverage_notice).small().color(MUTED));
                }
                Err(error) => {
                    ui.label(RichText::new(error).color(theme::warn(ui)));
                }
            });
        }

        ui.add_space(theme::GAP);
        ui.label(RichText::new("Common token-grabber check").strong());
        ui.label(
            RichText::new(
                "Checks JavaScript, PowerShell, Python, batch, VBScript, and HTML files in Startup, Downloads, Discord/BetterDiscord, Chrome and Edge extensions, and Temp. Browser credential databases are skipped; nothing is deleted or uploaded.",
            )
            .small()
            .color(MUTED),
        );
        ui.add_space(theme::GAP);

        let scanning = self.token_scan_rx.is_some();
        if ui
            .add_enabled(
                !scanning,
                Button::new("Scan common locations for token grabbers"),
            )
            .pointer()
            .clicked()
        {
            self.token_scan_report = None;
            self.token_scan_rx = Some(security_scan::start_token_scan());
            self.say("Scanning selected user folders for suspicious script patterns…");
        }
        if scanning {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(
                    RichText::new("Scanning selected scripts… (up to 30 seconds)").color(MUTED),
                );
            });
        }

        if let Some(report) = &self.token_scan_report {
            ui.add_space(theme::GAP);
            theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
                ui.label(RichText::new("Local script scan results").strong());
                ui.label(format!(
                    "{} files checked across {} existing locations; {} unreadable and {} oversized files skipped.",
                    report.files_scanned,
                    report.locations_checked,
                    report.unreadable,
                    report.skipped_large
                ));
                if report.limit_reached {
                    ui.label(RichText::new("A safety limit was reached, so not every location may have been scanned.").color(theme::warn(ui)));
                }
                if report.findings.is_empty() {
                    ui.add_space(theme::GAP_TIGHT);
                    ui.label(RichText::new("No suspicious script patterns matched. This does not guarantee the computer is clean.").color(MUTED));
                } else {
                    ui.add_space(theme::GAP);
                    ui.label(RichText::new("Possible matches only; review before taking action.").color(theme::warn(ui)));
                }
            });

            let mut open_folder = None;
            for finding in &report.findings {
                theme::card(ui).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(RichText::new(finding.path.display().to_string()).monospace());
                            ui.label(RichText::new(&finding.reason).small().color(MUTED));
                        });
                        if ui.button("Show folder").pointer().clicked() {
                            open_folder = finding.path.parent().map(PathBuf::from);
                        }
                    });
                });
                ui.add_space(theme::GAP);
            }
            if let Some(path) = open_folder {
                match crate::util::shell_open(&path.to_string_lossy()) {
                    Ok(()) => self.say("Opened the matching file's folder."),
                    Err(error) => self.complain(format!("Could not open folder: {error}")),
                }
            }
        }
    }

    /// Banner shown on pages whose contents mostly need an elevated token,
    /// with the button that actually gets you one.
    fn elevation_banner(&mut self, ui: &mut Ui, why: &str) {
        if self.elevated {
            return;
        }
        let mut elevate = false;
        theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(why).color(theme::warn(ui)));
            ui.add_space(theme::GAP);
            if ui
                .button("Restart as administrator")
                .on_hover_text("Closes this window and reopens it with an administrator token.")
                .pointer()
                .clicked()
            {
                elevate = true;
            }
        });
        ui.add_space(theme::GAP);
        if elevate {
            self.elevate_now(ui.ctx());
        }
    }

    /// Hand off to an elevated copy of ourselves and close this one. Settings
    /// are flushed first so the new process picks up the current state.
    fn elevate_now(&mut self, ctx: &egui::Context) {
        self.remember();
        match relaunch_elevated() {
            Ok(()) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Err(error) => self.complain(format!("Could not elevate: {error}")),
        }
    }

    fn page_repair(&mut self, ui: &mut Ui) {
        self.elevation_banner(
            ui,
            "Most repairs need administrator rights, and are disabled without them.",
        );
        let actions = self.repairs.clone();
        self.action_list(ui, &actions);
    }

    fn page_tools(&mut self, ui: &mut Ui) {
        let accent = theme::accent_on(ui, self.accent);
        let actions = self.utilities.clone();
        let mut launch: Option<Action> = None;
        ui.columns(3, |columns| {
            for (index, action) in actions.iter().enumerate() {
                let column = index % columns.len();
                let target = &mut columns[column];
                let glyph = tool_icon(&action.name);
                let (clicked, _) = chrome::card_button(target, ("tool", index), |ui| {
                    // Fixed tiles, so the grid lines up however long the
                    // descriptions run.
                    ui.set_min_height(148.0);
                    ui.set_max_height(148.0);
                    ui.horizontal(|ui| {
                        ui.label(chrome::icon_text(glyph, 24.0, accent));
                        if action.needs_admin {
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                chrome::pill(ui, "admin", theme::warn(ui));
                            });
                        }
                    });
                    ui.add_space(theme::GAP_TIGHT);
                    ui.add(Label::new(RichText::new(&action.name).strong()).truncate());
                    // ui.columns hands down a *justified* layout, which stretches
                    // a label that wraps -- that is what spaced the words of the
                    // longer descriptions out across the line.
                    ui.with_layout(Layout::top_down(Align::Min), |ui| {
                        ui.label(RichText::new(&action.what).small().color(MUTED));
                    });
                });
                if clicked.pointer().clicked() {
                    launch = Some(action.clone());
                }
                target.add_space(10.0);
            }
        });
        if let Some(action) = launch {
            self.launch_action(action);
        }
    }

    fn action_list(&mut self, ui: &mut Ui, actions: &[Action]) {
        let accent = theme::accent_on(ui, self.accent);
        let elevated = self.elevated;
        let mut launch: Option<Action> = None;

        for action in actions {
            theme::card(ui).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let blocked = action.needs_admin && !elevated;
                    let button_label = if action.name.starts_with("Microsoft Defender") {
                        "Start"
                    } else {
                        "Open"
                    };
                    // Name first, action on the right. The button used to lead
                    // the row, so it sat wedged against the text it belonged
                    // to and read as part of the title.
                    ui.vertical(|ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new(&action.name).strong());
                            if action.needs_admin {
                                chrome::pill(ui, "admin", theme::warn(ui));
                            }
                        });
                        ui.label(RichText::new(&action.what).small().color(MUTED));
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .add_enabled_ui(!blocked, |ui| {
                                chrome::button(
                                    ui,
                                    chrome::Weight::Ghost,
                                    None,
                                    button_label,
                                    accent,
                                )
                            })
                            .inner
                            .on_disabled_hover_text("Needs administrator rights.")
                            .pointer()
                            .clicked()
                        {
                            launch = Some(action.clone());
                        }
                    });
                });
            });
            ui.add_space(theme::GAP);
        }

        if let Some(action) = launch {
            self.launch_action(action);
        }
    }

    fn page_backup(&mut self, ui: &mut Ui) {
        self.elevation_banner(ui, "Restore points need administrator rights.");
        let mut export = false;
        let mut restore_point = false;
        let mut protection = false;

        let running = self.backup_job.as_ref().map(|(job, _)| *job);
        if let Some(job) = running {
            theme::widget(ui).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.add_space(theme::GAP_TIGHT);
                    ui.label(RichText::new(job.running_text()).font(theme::readout()));
                });
            });
            ui.add_space(theme::GAP);
        }
        let busy = running.is_some();

        theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("Registry export").strong());
            ui.label(
                RichText::new(
                    "Exports every hive to .reg files in a timestamped folder you choose. \
                     Some HKLM subtrees are protected and get skipped even as administrator.",
                )
                .small()
                .color(MUTED),
            );
            ui.add_space(theme::GAP);
            if ui
                .add_enabled(
                    !busy,
                    Button::new(RichText::new("Choose folder and export").strong()),
                )
                .on_disabled_hover_text("A backup task is already running.")
                .pointer()
                .clicked()
            {
                export = true;
            }
            if !self.backups.is_empty() {
                ui.add_space(theme::GAP);
                for file in &self.backups {
                    ui.label(
                        RichText::new(file.display().to_string())
                            .monospace()
                            .small()
                            .color(MUTED),
                    );
                }
            }
        });

        ui.add_space(theme::GAP);
        theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("System restore point").strong());
            ui.label(
                RichText::new(
                    "The backup that actually undoes a bad tweak. Needs System Protection \
                     enabled on the system drive, and administrator rights.",
                )
                .small()
                .color(MUTED),
            );
            ui.add_space(theme::GAP);
            if ui
                .add_enabled(self.elevated && !busy, Button::new("Create restore point"))
                .on_disabled_hover_text(if busy {
                    "A backup task is already running."
                } else {
                    "Needs administrator rights."
                })
                .pointer()
                .clicked()
            {
                restore_point = true;
            }
            ui.add_space(theme::GAP_TIGHT);
            if ghost(ui, "Open System Protection settings")
                .pointer()
                .clicked()
            {
                protection = true;
            }
        });

        if export {
            let mut dialog = rfd::FileDialog::new().set_title("Where should the backup go?");
            if let Some(previous) = self.settings.last_backup_dir.clone() {
                if previous.is_dir() {
                    dialog = dialog.set_directory(previous);
                }
            }
            if let Some(dir) = dialog.pick_folder() {
                self.settings.remember_backup_dir(&dir);
                self.say(format!(
                    "Exporting the registry to {}\u{2026}",
                    dir.display()
                ));
                self.start_backup_job(BackupJob::Export, Some(dir));
            }
        }
        if restore_point {
            self.say("Creating a restore point. This can take a minute.");
            self.start_backup_job(BackupJob::RestorePoint, None);
        }
        if protection {
            match crate::util::spawn("SystemPropertiesProtection.exe", &[]) {
                Ok(()) => self.say("Opened System Protection."),
                Err(e) => self.complain(format!("Could not open it: {e}")),
            }
        }
    }

    /// The AI connection controls, as a section of the Settings page.
    ///
    /// This was a top-level tab until it became clear what it actually is: a
    /// preference about which local MCP server the app talks to. It sat in
    /// the sidebar between Backup and System Info, among pages that change
    /// the machine, and so read as something far weightier than two text
    /// fields and a list of tool names.
    fn ai_connections_section(&mut self, ui: &mut Ui) {
        let mut events = Vec::new();
        if let Some(connection) = &self.mcp_connection {
            while let Ok(event) = connection.events.try_recv() {
                events.push(event);
            }
        }
        let mut failed = false;
        for event in events {
            match event {
                crate::mcp_client::Event::Connected {
                    server_name,
                    server_version,
                    tools,
                } => {
                    self.mcp_status = format!("Connected · {} tools", tools.len());
                    self.mcp_server = Some((server_name, server_version));
                    self.mcp_tools = tools;
                }
                crate::mcp_client::Event::Disconnected => {
                    self.mcp_status = "Disconnected".into();
                    self.mcp_server = None;
                    self.mcp_tools.clear();
                }
                crate::mcp_client::Event::Failed(error) => {
                    self.mcp_status = format!("Connection failed: {error}");
                    self.mcp_server = None;
                    self.mcp_tools.clear();
                    failed = true;
                }
            }
        }
        if failed {
            self.mcp_connection = None;
        }

        // Discovery hits the disk, so it happens on the first frame this
        // section is drawn and never again. Doing it in `App::new` charged
        // every launch for a section most people never scroll to; doing it
        // while painting read two files at the refresh rate.
        if self.ai_discovered.is_none() {
            self.ai_discovered = Some(crate::ai_config::discover());
        }
        // Moved out rather than borrowed, because the rows below are drawn
        // from this list and clicking one writes to `self`. Taking it for the
        // length of the frame is what lets both happen without cloning every
        // string each frame; it goes back at the end.
        let discovered = self.ai_discovered.take().unwrap_or_default();

        let accent = theme::accent_on(ui, self.accent);
        chrome::section_heading(ui, chrome::icon::TOOLS, "AI connections", accent);
        ui.add_space(theme::GAP_TIGHT);
        ui.add(
            Label::new(
                RichText::new(
                    "Connect a local Model Context Protocol server. Claude and OpenAI here are \
                     the two programs on this PC that already keep a list of them; nothing is \
                     sent to either company.",
                )
                .small()
                .color(MUTED),
            )
            .wrap(),
        );
        ui.add_space(theme::GAP);

        ui.horizontal(|ui| {
            for source in [AiSource::Claude, AiSource::OpenAi, AiSource::Custom] {
                let selected = self.ai_source == source;
                if chrome::segment(ui, source.label(), selected, accent)
                    .pointer()
                    .clicked()
                {
                    self.ai_source = source;
                }
            }
        });
        ui.add_space(theme::GAP);

        // Both a discovered row and the Connect button want the same thing to
        // happen, and neither can touch `self` where the decision is made, so
        // the choice is carried out of the card and acted on below it.
        let mut connect_to: Option<(String, Vec<String>)> = None;
        theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            match self.ai_source.discovery() {
                None => {
                    ui.label(RichText::new("Server program").strong());
                    ui.add(
                        egui::TextEdit::singleline(&mut self.mcp_program)
                            .hint_text(
                                r"Full path or executable name, e.g. C:\Tools\my-mcp-server.exe",
                            )
                            .desired_width(f32::INFINITY),
                    );
                    ui.add_space(theme::GAP);
                    ui.label(RichText::new("Arguments · one per line").strong());
                    ui.add(
                        egui::TextEdit::multiline(&mut self.mcp_args)
                            .hint_text("--stdio\n--config\nC:\\path with spaces\\server.json")
                            .desired_width(f32::INFINITY)
                            .desired_rows(3),
                    );
                }
                Some(wanted) => {
                    let mut shown = 0usize;
                    for (index, entry) in discovered
                        .iter()
                        .enumerate()
                        .filter(|(_, entry)| entry.source == wanted)
                    {
                        if shown > 0 {
                            ui.add_space(theme::GAP_TIGHT);
                        }
                        shown += 1;
                        let live =
                            self.mcp_program == entry.command && self.mcp_connection.is_some();
                        let (response, ()) = chrome::card_button(ui, ("ai-server", index), |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                // The pill is laid out first because a
                                // truncating label claims whatever width is
                                // left, and a server name long enough to
                                // truncate would otherwise push the status
                                // clean off the right edge of the card.
                                if live {
                                    chrome::pill(ui, "Connected", theme::good(ui));
                                }
                                ui.add(
                                    Label::new(
                                        RichText::new(&entry.name)
                                            .font(theme::card_title())
                                            .strong(),
                                    )
                                    .truncate(),
                                );
                            });
                            ui.add(
                                Label::new(
                                    RichText::new(entry.command_line()).small().color(MUTED),
                                )
                                .truncate(),
                            );
                        });
                        if response.pointer().clicked() {
                            connect_to = Some((entry.command.clone(), entry.args.clone()));
                        }
                    }
                    if shown == 0 {
                        // An empty box under a heading reads as a loading
                        // state, so say outright that there is nothing and
                        // name the way forward.
                        ui.add(
                            Label::new(
                                RichText::new(format!(
                                    "No MCP servers are configured for {}.",
                                    wanted.label()
                                ))
                                .strong(),
                            )
                            .wrap(),
                        );
                        ui.add_space(theme::GAP_TIGHT);
                        ui.add(
                            Label::new(
                                RichText::new(format!(
                                    "Nothing was found in {}. Choose Custom above to point at a \
                                     server yourself.",
                                    wanted.origin()
                                ))
                                .small()
                                .color(MUTED),
                            )
                            .wrap(),
                        );
                    }
                }
            }

            // The warning stays in all three modes. A discovered row is read
            // out of a file this app does not own, and clicking it still
            // launches a program, which is exactly the risk a hand-typed path
            // carries, so hiding the warning there would have it backwards.
            ui.add_space(theme::GAP);
            ui.add(
                Label::new(
                    RichText::new(
                        "Only connect to a server you trust. Connecting starts that program on \
                         this PC with these arguments. Its stdout is reserved for MCP messages.",
                    )
                    .small()
                    .color(theme::warn(ui)),
                )
                .wrap(),
            );
            ui.add_space(theme::GAP);

            ui.horizontal(|ui| {
                let connected = self.mcp_connection.is_some();
                let can_connect = !connected && !self.mcp_program.trim().is_empty();
                let connect = ui
                    .add_enabled_ui(can_connect, |ui| {
                        chrome::button(ui, chrome::Weight::Primary, None, "Connect", accent)
                    })
                    .inner;
                if connect.pointer().clicked() && can_connect {
                    let args = self
                        .mcp_args
                        .lines()
                        .map(str::trim)
                        .filter(|line| !line.is_empty())
                        .map(str::to_string)
                        .collect();
                    connect_to = Some((self.mcp_program.trim().to_string(), args));
                }
                let disconnect = ui
                    .add_enabled_ui(connected, |ui| {
                        chrome::button(ui, chrome::Weight::Ghost, None, "Disconnect", accent)
                    })
                    .inner;
                if disconnect.pointer().clicked() && connected {
                    if let Some(mut connection) = self.mcp_connection.take() {
                        connection.disconnect();
                    }
                    self.mcp_status = "Disconnected".into();
                    self.mcp_server = None;
                    self.mcp_tools.clear();
                }
                let status_color = if self.mcp_status.starts_with("Connection failed") {
                    theme::bad(ui)
                } else if self.mcp_status.starts_with("Connected") {
                    theme::good(ui)
                } else {
                    MUTED
                };
                ui.add(Label::new(RichText::new(&self.mcp_status).color(status_color)).truncate());
            });
        });

        if let Some((program, args)) = connect_to {
            // Picking a discovered row fills the Custom fields too, so
            // switching to Custom shows what is actually running rather than
            // whatever happened to be typed there last.
            self.mcp_program = program.clone();
            self.mcp_args = args.join("\n");
            if let Some(mut previous) = self.mcp_connection.take() {
                previous.disconnect();
            }
            self.mcp_connection = Some(crate::mcp_client::Connection::start(program, args));
            self.mcp_status = "Starting server and requesting its tools…".into();
            self.mcp_server = None;
            self.mcp_tools.clear();
        }

        if let Some((name, version)) = &self.mcp_server {
            ui.add_space(theme::GAP);
            theme::card(ui).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.add(
                    Label::new(RichText::new(name).font(theme::card_title()).strong()).truncate(),
                );
                ui.add(
                    Label::new(
                        RichText::new(format!("Version {version}"))
                            .small()
                            .color(MUTED),
                    )
                    .truncate(),
                );
                ui.add_space(theme::GAP);
                ui.label(RichText::new("Available tools").strong());
                if self.mcp_tools.is_empty() {
                    ui.label(
                        RichText::new("This server did not advertise any tools.").color(MUTED),
                    );
                }
                for tool in &self.mcp_tools {
                    ui.separator();
                    ui.add(Label::new(RichText::new(&tool.name).strong()).truncate());
                    if !tool.description.is_empty() {
                        ui.add(
                            Label::new(RichText::new(&tool.description).small().color(MUTED))
                                .wrap(),
                        );
                    }
                }
                if self.mcp_tools.len() == 256 {
                    ui.label(
                        RichText::new("Showing the first 256 tools.")
                            .small()
                            .color(MUTED),
                    );
                }
            });
        }

        self.ai_discovered = Some(discovered);

        if self.mcp_status.starts_with("Starting") {
            ui.ctx().request_repaint_after(Duration::from_millis(100));
        }
    }

    fn page_settings(&mut self, ui: &mut Ui) {
        let accent = theme::accent_on(ui, self.accent);
        // Three unrelated subjects stacked down one scroll was the previous
        // shape, and the separator between them was doing more work than a
        // rule can. Switching between them costs a click and makes each one
        // a page that fits on screen.
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 5.0;
            for view in SettingsView::ALL {
                if chrome::segment(ui, view.label(), self.settings_view == view, accent)
                    .pointer()
                    .clicked()
                {
                    self.settings_view = view;
                }
            }
        });
        ui.add_space(theme::GAP_SECTION);

        match self.settings_view {
            SettingsView::Appearance => {
                let changed = self
                    .appearance
                    .page(ui, &mut self.settings, &mut self.accent);
                if changed {
                    self.settings.save();
                    self.appearance.apply(ui.ctx(), &self.settings, self.accent);
                }
            }
            SettingsView::Connections => self.ai_connections_section(ui),
            SettingsView::About => self.about_section(ui),
        }
    }

    fn about_section(&mut self, ui: &mut Ui) {
        let accent = theme::accent_on(ui, self.accent);
        theme::widget(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("Windows Wrenchmaster")
                        .font(theme::section_title())
                        .strong(),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    chrome::pill(ui, &format!("v{}", env!("CARGO_PKG_VERSION")), accent);
                });
            });
            ui.label(
                RichText::new("Offline Windows maintenance. Rust and egui, no network.")
                    .small()
                    .color(MUTED),
            );
        });

        ui.add_space(theme::GAP);
        theme::widget(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                RichText::new("CHANGELOG")
                    .font(theme::eyebrow())
                    .color(theme::muted(ui)),
            );
            ui.add_space(theme::GAP_TIGHT);
            // One line each. The long-form history of the 2017 VB.NET build
            // that this replaces was interesting to write and is of no use to
            // anyone reading the About page to find out what version they are
            // running.
            for (version, note) in [
                (
                    "2.0",
                    "Rewritten in Rust. Offline, reversible tweaks, worker-thread cleaner.",
                ),
                ("0.9", "Last VB.NET build."),
                ("0.1", "First version."),
            ] {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(version)
                            .font(theme::readout())
                            .strong()
                            .color(accent),
                    );
                    ui.add(Label::new(RichText::new(note).small().color(MUTED)).truncate());
                });
            }
        });

        ui.add_space(theme::GAP);
        theme::widget(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(chrome::icon_text(chrome::icon::HEART, 15.0, accent));
                ui.vertical(|ui| {
                    ui.label(RichText::new("Support the project").font(theme::card_title()));
                    ui.label(
                        RichText::new("Free and offline. Nothing to unlock.")
                            .small()
                            .color(MUTED),
                    );
                });
                // The only outbound link in the app, and it opens in the
                // browser rather than fetching anything. A click the user
                // made is not the app phoning home, which is the promise the
                // rest of the build keeps.
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if chrome::button(ui, chrome::Weight::Ghost, None, "Patreon", accent)
                        .pointer()
                        .clicked()
                    {
                        match crate::util::shell_open(PATREON_URL) {
                            Ok(()) => self.say("Opened Patreon in your browser."),
                            Err(error) => {
                                self.complain(format!("Could not open the browser: {error}"))
                            }
                        }
                    }
                });
            });
        });

        ui.add_space(theme::GAP);
        ui.label(
            RichText::new("Deletes are permanent. Create a restore point before broad changes.")
                .small()
                .color(theme::warn(ui)),
        );
    }
}

fn read_nvidia_sensors() -> Result<Vec<GpuTelemetry>, String> {
    let mut command = Command::new("nvidia-smi");
    command.args([
        "--query-gpu=name,temperature.gpu,fan.speed,power.draw,power.limit",
        "--format=csv,noheader,nounits",
    ]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    let output = command.output().map_err(|error| {
        format!("NVIDIA sensor source unavailable (nvidia-smi could not start: {error}).")
    })?;
    if !output.status.success() {
        return Err("NVIDIA sensor source unavailable; the driver query did not succeed.".into());
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let readings = stdout
        .lines()
        .filter_map(|line| {
            let fields: Vec<_> = line.split(',').map(str::trim).collect();
            if fields.len() < 5 || fields[0].is_empty() {
                return None;
            }
            let available = |value: &str| {
                let normalized = value.trim();
                (!normalized.is_empty() && !normalized.eq_ignore_ascii_case("n/a"))
                    .then(|| normalized.to_string())
            };
            Some(GpuTelemetry {
                name: fields[0].to_string(),
                temperature_c: available(fields[1]),
                fan_percent: available(fields[2]),
                power_w: available(fields[3]),
                power_limit_w: available(fields[4]),
            })
        })
        .collect::<Vec<_>>();
    if readings.is_empty() {
        return Err("NVIDIA driver did not report graphics sensor readings.".into());
    }
    Ok(readings)
}

fn read_hardware_sensor_provider() -> Result<Vec<HardwareSensorReading>, String> {
    let executable = std::env::current_exe()
        .map_err(|error| format!("Could not locate Wrenchmaster: {error}"))?
        .parent()
        .ok_or_else(|| "Could not locate Wrenchmaster's output folder.".to_string())?
        .join("sensors")
        .join("wrenchmaster-sensors.exe");
    if !executable.is_file() {
        return Err(
            "LibreHardwareMonitor sensor helper is not present beside Wrenchmaster.".into(),
        );
    }
    let mut command = Command::new(executable);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    let output = command
        .output()
        .map_err(|error| format!("Could not start the hardware sensor helper: {error}"))?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if detail.is_empty() {
            "Hardware sensor helper could not read sensors. Try launching Wrenchmaster as administrator.".into()
        } else {
            format!("Hardware sensor helper failed: {detail}")
        });
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let readings = stdout
        .lines()
        .filter_map(|line| {
            let fields: Vec<_> = line.split('\t').collect();
            (fields.len() == 8).then(|| HardwareSensorReading {
                hardware_type: fields[0].to_string(),
                hardware_name: fields[1].to_string(),
                parent_type: fields[2].to_string(),
                parent_name: fields[3].to_string(),
                sensor_type: fields[4].to_string(),
                sensor_name: fields[5].to_string(),
                value: fields[6].to_string(),
                unit: fields[7].to_string(),
            })
        })
        .collect::<Vec<_>>();
    Ok(readings)
}

fn draw_hardware_sensor_rows(ui: &mut Ui, readings: &[&HardwareSensorReading], empty_text: &str) {
    let mut supported = readings
        .iter()
        .copied()
        .filter(|reading| {
            let name = reading.sensor_name.to_ascii_lowercase();
            match reading.sensor_type.as_str() {
                "Temperature" | "Fan" => true,
                "Voltage" => !name.contains("vid") && !name.starts_with("core #"),
                "Power" => {
                    name.contains("package")
                        || name.contains("total")
                        || name.contains("board")
                        || name.contains("gpu")
                }
                _ => false,
            }
        })
        .collect::<Vec<_>>();
    supported.sort_by(|left, right| {
        left.sensor_type
            .cmp(&right.sensor_type)
            .then_with(|| left.sensor_name.cmp(&right.sensor_name))
    });
    if supported.is_empty() {
        ui.label(RichText::new(empty_text).small().color(MUTED));
        return;
    }
    ui.add_space(theme::GAP_TIGHT);
    // A grid, not a wrapped row: in a horizontal layout each chip is placed
    // against the growing row height, so they drift downwards as the row
    // fills. A grid puts them on fixed rails.
    let salt = supported
        .first()
        .map(|r| r.hardware_name.clone())
        .unwrap_or_else(|| "sensors".to_string());
    egui::Grid::new(salt)
        .num_columns(3)
        .spacing([18.0, 6.0])
        .show(ui, |ui| {
            for (index, reading) in supported.into_iter().enumerate() {
                let number = reading.value.trim().parse::<f32>().ok();
                let units = match reading.sensor_type.as_str() {
                    "Temperature" => " °C".to_string(),
                    _ if reading.unit.is_empty() => String::new(),
                    // Anything the provider sent that did not survive decoding is
                    // dropped rather than drawn as a box.
                    _ if reading.unit.contains('�') => String::new(),
                    _ => format!(" {}", reading.unit),
                };

                // Temperature is the reading worth colouring: it is the one that
                // says whether the machine is in trouble. Fans and load get the
                // accent and a bar; volts and watts have no meaningful ceiling to
                // draw a bar against, so they stay plain.
                let (color, fraction) = match reading.sensor_type.as_str() {
                    "Temperature" => match number {
                        Some(c) => (chrome::temp_color(ui, c), Some(chrome::temp_fraction(c))),
                        None => (MUTED, None),
                    },
                    "Fan" | "Load" => (
                        ACCENT,
                        number.filter(|_| reading.unit == "%").map(|v| v / 100.0),
                    ),
                    _ => (MUTED, None),
                };

                let shown = match number {
                    Some(v) if v.fract() == 0.0 => format!("{v:.0}"),
                    Some(v) => format!("{v:.1}"),
                    None => reading.value.clone(),
                };
                chrome::sensor_chip(
                    ui,
                    &reading.sensor_name,
                    &format!("{shown}{units}"),
                    color,
                    fraction,
                )
                .on_hover_text(format!(
                    "{} · {} · {} (owned by {} · {})",
                    reading.sensor_type,
                    reading.hardware_type,
                    reading.hardware_name,
                    reading.parent_type,
                    reading.parent_name
                ));
                if (index + 1) % 3 == 0 {
                    ui.end_row();
                }
            }
        });
}

fn size_folder_contains(
    folders: &[treesize::FolderSize],
    ancestor: usize,
    mut folder: usize,
) -> bool {
    loop {
        if folder == ancestor {
            return true;
        }
        let Some(parent) = folders.get(folder).and_then(|item| item.parent) else {
            return false;
        };
        folder = parent;
    }
}

fn draw_diagnostic_event(ui: &mut Ui, index: usize, finding: &EventFinding) {
    // The position in the list is what makes this unique. Windows really does
    // log the same source, event id and timestamp more than once, and keying
    // only on those made two rows share an id -- which egui reports by painting
    // "First/Second use of widget ID" over the page.
    let _ = chrome::accordion(
        ui,
        (
            "diagnostic-event",
            index,
            &finding.source,
            finding.event_id,
            &finding.time,
        ),
        &format!(
            "{} · Event {} · {}",
            finding.source, finding.event_id, finding.time
        ),
        false,
        |ui| {
            ui.label(RichText::new(&finding.message).small());
        },
    );
}

fn tool_icon(name: &str) -> &'static str {
    match name {
        "System information" => chrome::icon::SYSTEM_INFO,
        "DirectX diagnostics" => chrome::icon::DIAGNOSTICS,
        "Resource monitor" | "Performance monitor" => chrome::icon::CHART,
        "Reliability monitor" => chrome::icon::HEART,
        "Event viewer" => chrome::icon::EVENTS,
        "Services" => chrome::icon::SETTINGS,
        "Device manager" => chrome::icon::DEVICES,
        "Disk management" | "Storage sense" => chrome::icon::STORAGE,
        "Task scheduler" => chrome::icon::SCHEDULE,
        "Registry editor" => chrome::icon::REGISTRY,
        "Installed apps" | "Startup apps" => chrome::icon::APPS,
        "Windows security" => chrome::icon::SHIELD,
        "MAS" => chrome::icon::SETTINGS,
        "About Windows" => chrome::icon::INFO,
        _ => chrome::icon::TOOLS,
    }
}

fn kv(ui: &mut Ui, key: &str, value: &str) {
    ui.horizontal_top(|ui| {
        ui.add_sized(
            [124.0, 18.0],
            Label::new(RichText::new(key).small().color(MUTED)).halign(Align::Min),
        );
        // Wrap instead of running on. In a horizontal layout a plain label does
        // not wrap, so a long value overflowed its column and the card beside
        // it was painted over the overflow.
        ui.add(Label::new(value).wrap());
    });
}

fn shorten(text: &str, max: usize) -> String {
    let count = text.chars().count();
    if count <= max {
        return text.to_string();
    }
    let tail: String = text.chars().skip(count - max + 1).collect();
    format!("…{tail}")
}

/// One live reading: name, figure, trace, caption.
///
/// Every card in the Overview row is built from this, which is why they line
/// up -- they are the same shape, rather than four different shapes pinned to
/// the same height. Change the shape here and the row stays aligned.
///
/// The order is deliberate. The name is an eyebrow, small and grey; the
/// figure is the headline; the chart takes the width underneath it. A card
/// whose title is the biggest thing on it is a card you have to read before
/// you can use, and these are meant to be glanced at.
fn reading_card(
    ui: &mut Ui,
    name: &str,
    value: &str,
    color: Color32,
    status: &str,
    fraction: f32,
    caption: &str,
    trend: Option<&[f32]>,
) {
    theme::widget(ui).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.set_min_height(162.0);
        ui.horizontal(|ui| {
            let icon = match name {
                "Processor" => chrome::icon::DEVICES,
                "Graphics" => chrome::icon::CHART,
                "Memory" => chrome::icon::MEMORY,
                _ => chrome::icon::STORAGE,
            };
            ui.label(chrome::icon_text(icon, 13.0, color));
            ui.add(
                Label::new(
                    RichText::new(name)
                        .font(theme::eyebrow())
                        .color(theme::muted(ui)),
                )
                .truncate(),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                chrome::pill(ui, status, color);
            });
        });
        ui.add_space(theme::GAP_TIGHT);
        ui.label(
            RichText::new(value)
                .font(theme::readout_large())
                .color(color),
        );
        ui.add_space(theme::GAP_TIGHT);
        match trend {
            // Where there is history, the shape of the last few minutes says
            // more than the bar: it separates "busy" from "spiking".
            Some(values) if values.len() >= 2 => {
                chrome::sparkline(ui, values, color, 100.0);
                ui.add_space(theme::GAP_TIGHT);
                ui.add(Label::new(RichText::new(caption).small().color(MUTED)).truncate());
            }
            // A card that is going to carry a chart keeps the chart's space
            // from the first frame. Falling back to the bar meant the three
            // live cards opened as meters and swapped to sparklines a second
            // or two later, which read as a glitch rather than as data
            // arriving.
            Some(_) => {
                chrome::sparkline(ui, &[], color, 100.0);
                ui.add_space(theme::GAP_TIGHT);
                ui.add(
                    Label::new(
                        RichText::new("Collecting\u{2026}")
                            .small()
                            .color(theme::faint(ui)),
                    )
                    .truncate(),
                );
            }
            // The caption goes under the bar in the small muted type, not
            // into the meter's own label line. Passed to the meter it was set
            // in the reading font on the same row as the percentage, so the
            // drive card read as two competing figures while the three cards
            // beside it kept their caption quietly underneath.
            //
            // The bar is then padded out to the height a sparkline would have
            // taken, so the caption lands on the same line as the captions on
            // the three cards beside it. A 10px bar where the others have a
            // 46px chart left this card's last line floating a finger's width
            // above theirs, which is the kind of near-miss that makes a row of
            // cards look accidental.
            None => {
                let trend_height = ui.available_height().clamp(30.0, 46.0);
                let before = ui.cursor().top();
                chrome::meter(ui, fraction, color, None);
                let used = ui.cursor().top() - before;
                ui.add_space((trend_height - used).max(0.0));
                ui.add_space(theme::GAP_TIGHT);
                ui.add(Label::new(RichText::new(caption).small().color(MUTED)).truncate());
            }
        }
    });
}

/// A compact status tile for a measured part of the system.
///
/// The figure carries the state; the short caption explains what it means.
/// This keeps the first pass over a health check readable while leaving the
/// detailed readings available below.
///
/// Same hierarchy as `reading_card`: eyebrow, figure, scale, caption. The two
/// appear on different pages but within sight of each other in the same
/// session, so they are built to the same proportions.
fn metric_widget(
    ui: &mut Ui,
    icon: &str,
    title: &str,
    value: &str,
    status: &str,
    detail: &str,
    fraction: Option<f32>,
    color: Color32,
) {
    theme::widget(ui).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.set_min_height(162.0);
        ui.horizontal(|ui| {
            ui.label(chrome::icon_text(icon, 13.0, color));
            ui.add(
                Label::new(
                    RichText::new(title)
                        .font(theme::eyebrow())
                        .color(theme::muted(ui)),
                )
                .truncate(),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                chrome::pill(ui, status, color);
            });
        });
        ui.add_space(theme::GAP_TIGHT);
        ui.label(
            RichText::new(value)
                .font(theme::readout_large())
                .color(color),
        );
        if let Some(fraction) = fraction {
            ui.add_space(theme::GAP_TIGHT);
            chrome::meter(ui, fraction.clamp(0.0, 1.0), color, None);
        }
        ui.add_space(theme::GAP_TIGHT);
        ui.add(Label::new(RichText::new(detail).small().color(MUTED)).wrap());
    });
}

/// One ranked finding on the Diagnose report, whatever produced it.
///
/// The machine checks and the health assessment both name things that are
/// wrong, and the page used to print them as two lists with no way to tell
/// which mattered more. Ranking them against each other needs one shape, and
/// `source` keeps the provenance visible rather than blending them into an
/// anonymous list the user cannot verify.
struct Finding {
    severity: health::Severity,
    title: String,
    detail: String,
    source: &'static str,
}

/// A sidebar entry that owns sub-entries. Named rather than passed as four
/// booleans so that opening one and closing the rest is a single statement
/// that cannot be half-written.
#[derive(Clone, Copy, PartialEq, Eq)]
enum NavGroup {
    Hardware,
    Storage,
    Security,
    Tools,
}

/// The colour a check's verdict is drawn in, wherever it is drawn.
///
/// Not `Severity::color`, because the severity vocabulary has no word for "the
/// check itself failed". It maps `Error` to `None` along with `Skipped`, on
/// the reasoning that neither is a statement about the machine -- which is
/// true, and which left a check that blew up looking exactly like one that was
/// never meant to run. A check that could not complete is a failure of the
/// diagnosis and is drawn red, so a sweep with a hole in it cannot be mistaken
/// for a clean one.
///
/// `Skipped` stays grey. That one really is "nothing to say here": a Windows
/// check on a machine that has no such feature has not failed at anything.
fn check_color(ui: &Ui, status: diagnose::CheckStatus) -> Color32 {
    match status {
        diagnose::CheckStatus::Pass => theme::good(ui),
        diagnose::CheckStatus::Warning | diagnose::CheckStatus::NeedsAdmin => theme::warn(ui),
        diagnose::CheckStatus::Problem | diagnose::CheckStatus::Error => theme::bad(ui),
        diagnose::CheckStatus::Skipped => MUTED,
    }
}

/// One figure under a short label, for the row of counts on the report.
///
/// Not `metric_widget`: that draws a card, and four cards inside the callout
/// would be four borders inside a border. Here the callout is the container
/// and the figures are its contents.
fn stat(ui: &mut Ui, label: &str, value: &str, color: Color32) {
    ui.vertical(|ui| {
        ui.add(
            Label::new(
                RichText::new(label)
                    .font(theme::eyebrow())
                    .color(theme::muted(ui)),
            )
            .truncate(),
        );
        ui.label(
            RichText::new(value)
                .font(theme::readout_large())
                .color(color),
        );
    });
}

/// How long a check took, written the way a stopwatch would read it.
///
/// Sub-second checks still get a figure rather than a rounded `0 s`: three of
/// the nine are registry reads that genuinely finish in milliseconds, and a
/// column of zeroes reads as a column of failures.
fn format_elapsed(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs_f32();
    if seconds < 1.0 {
        format!("{} ms", elapsed.as_millis())
    } else if seconds < 60.0 {
        format!("{seconds:.1} s")
    } else {
        format!(
            "{} min {:02} s",
            elapsed.as_secs() / 60,
            elapsed.as_secs() % 60
        )
    }
}

/// A secondary button. Ghost weight takes its colours from the widget visuals,
/// so it needs no accent passed in -- which is what makes it usable as a
/// drop-in wherever a cramped `small_button` used to be.
fn ghost(ui: &mut Ui, label: &str) -> egui::Response {
    chrome::button(ui, chrome::Weight::Ghost, None, label, theme::ACCENT)
}

/// A card that behaves as a link: the whole surface is the target, because a
/// small button sitting inside a large card is the worst of both.
fn link_card(ui: &mut Ui, glyph: &str, accent: Color32, title: &str, detail: &str) -> bool {
    let card = theme::widget(ui)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_min_height(78.0);
            ui.horizontal(|ui| {
                ui.label(chrome::icon_text(glyph, 22.0, accent));
                ui.add_space(theme::GAP_TIGHT);
                ui.vertical(|ui| {
                    ui.label(RichText::new(title).font(theme::card_title()).strong());
                    ui.label(RichText::new(detail).small().color(MUTED));
                });
            });
        })
        .response
        .interact(egui::Sense::click());
    if card.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    card.pointer().clicked()
}

/// A paragraph of explanation, held to a readable measure. Body text set to
/// the full width of a 1080px window runs well past 100 characters a line,
/// which is beyond comfortable reading.
fn para(ui: &mut Ui, text: &str) {
    ui.scope(|ui| {
        ui.set_max_width(760.0);
        ui.label(RichText::new(text).small().color(MUTED));
    });
}
