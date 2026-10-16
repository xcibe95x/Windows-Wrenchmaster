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

use crate::chrome;
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
use crate::settings::Settings;
use crate::theme::{self, ACCENT, FAINT, MUTED};
use crate::tools::{self, Action};
use crate::treesize::{self, ScanHandle, ScanMessage, ScanProgress, ScanResult};
use crate::tweaks::{self, State, Tweak};
use crate::util::{
    human_bytes, is_development_mode, is_elevated, run_activation_scripts,
    relaunch_elevated,
};
use crate::{app_residue, registry_audit, startup};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Overview,
    SystemInfo,
    Hardware,
    Memory,
    Storage,
    Tweaks,
    Privacy,
    Troubleshoot,
    Diagnose,
    Security,
    Repair,
    Startup,
    Tools,
    Backup,
    AiConnections,
    About,
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

impl Tab {
    const ALL: [Tab; 13] = [
        Tab::Overview,
        Tab::SystemInfo,
        Tab::Hardware,
        Tab::Memory,
        Tab::Storage,
        Tab::Tweaks,
        Tab::Privacy,
        Tab::Diagnose,
        Tab::Security,
        Tab::Tools,
        Tab::Backup,
        Tab::AiConnections,
        Tab::About,
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
            Tab::SystemInfo => "System Info",
            Tab::Hardware => "Hardware",
            Tab::Memory => "Memory",
            Tab::Storage => "Storage",
            Tab::Tweaks => "Tweaks",
            Tab::Privacy => "Privacy",
            Tab::Troubleshoot => "Troubleshoot",
            Tab::Diagnose => "Diagnose",
            Tab::Security => "Security",
            Tab::Repair => "Repair",
            Tab::Startup => "Startup",
            Tab::Tools => "Tools",
            Tab::Backup => "Backup",
            Tab::AiConnections => "AI Connections",
            Tab::About => "About",
        }
    }

    fn blurb(self) -> &'static str {
        match self {
            Tab::Overview => "A quick health check and the next useful action.",
            Tab::SystemInfo => "Windows edition, activation, computer identity and uptime.",
            Tab::Hardware => "Processor, graphics and board, with live sensors where available.",
            Tab::Memory => "Everything about RAM: live use, what is holding it, and how to free it.",
            Tab::Storage => "Drives, cleanup, folder sizes and app leftovers.",
            Tab::Tweaks => "Registry switches, each one reversible from the same row.",
            Tab::Privacy => "Review privacy and Windows sharing controls one setting at a time.",
            Tab::Troubleshoot => "Windows diagnostics and guided troubleshooting options.",
            Tab::Diagnose => "A health check of memory, drive space, uptime and cleanup.",
            Tab::Security => "Scan for threats and review Windows protection.",
            Tab::Repair => "Commands that change the system. Each one says what it does first.",
            Tab::Startup => "Review common startup entries and whether their commands resolve.",
            Tab::Tools => "Read-only places to look when something is wrong.",
            Tab::Backup => "Export the registry and drop a restore point before you tweak.",
            Tab::AiConnections => {
                "Connect a trusted local MCP server and inspect its available tools."
            }
            Tab::About => "Where this came from, and what changed.",
        }
    }
}

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
    storage_expanded: bool,
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
    tweaks: Vec<Tweak>,
    troubleshooters: Vec<Action>,
    security_actions: Vec<Action>,
    repairs: Vec<Action>,
    utilities: Vec<Action>,
    diagcabs: Vec<PathBuf>,
    token_scan_rx: Option<Receiver<ScanReport>>,
    token_scan_report: Option<ScanReport>,
    diagnose_event_rx: Option<Receiver<EventScanResult>>,
    diagnose_event_result: Option<EventScanResult>,
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
    accent: Color32,
    toast: Option<Toast>,
    backups: Vec<PathBuf>,
    mcp_program: String,
    mcp_args: String,
    mcp_connection: Option<crate::mcp_client::Connection>,
    mcp_status: String,
    mcp_server: Option<(String, String)>,
    mcp_tools: Vec<crate::mcp_client::ToolInfo>,
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
            settings.light = true;
        }
        chrome::install_fonts(&cc.egui_ctx);
        let [r, g, b] = machine.era.accent();
        let accent = Color32::from_rgb(r, g, b);
        theme::apply(&cc.egui_ctx, settings.light, accent);

        // Restore the target ticks the user left behind last time.
        let mut targets = cleaner::targets();
        for target in &mut targets {
            if settings.deselected.contains(target.name) {
                target.selected = false;
            }
        }

        Self {
            tab: Tab::from_env().unwrap_or(Tab::Overview),
            storage_view: StorageView::from_env(),
            storage_expanded: false,
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
            tweaks: tweaks::all(),
            troubleshooters: tools::troubleshooters(),
            security_actions: tools::security_actions(),
            repairs: tools::repairs(),
            utilities: tools::utilities(),
            diagcabs: tools::local_diagcabs(),
            token_scan_rx: None,
            token_scan_report: None,
            diagnose_event_rx: None,
            diagnose_event_result: None,
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
            accent,
            toast: None,
            backups: Vec::new(),
            mcp_program: String::new(),
            mcp_args: String::new(),
            mcp_connection: None,
            mcp_status: "Not connected".into(),
            mcp_server: None,
            mcp_tools: Vec::new(),
        }
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

    fn poll_diagnose_events(&mut self, ctx: &egui::Context) {
        let received = self.diagnose_event_rx.as_ref().map(Receiver::try_recv);
        match received {
            Some(Ok(result)) => {
                self.diagnose_event_rx = None;
                self.diagnose_event_result = Some(result);
            }
            Some(Err(TryRecvError::Empty)) => {
                ctx.request_repaint_after(Duration::from_millis(200));
            }
            Some(Err(TryRecvError::Disconnected)) => {
                self.diagnose_event_rx = None;
                self.diagnose_event_result = Some(EventScanResult {
                    status: ScanStatus::Error(
                        "The Windows event check stopped unexpectedly.".into(),
                    ),
                    findings: Vec::new(),
                    log_errors: Vec::new(),
                });
            }
            None => {}
        }
    }

    fn start_diagnosis(&mut self) {
        self.refresh_ram_data();
        self.diagnose_event_result = None;
        self.diagnose_event_rx = Some(diagnose::start_scan());
        self.say("Checking current resource pressure and recent Windows errors…");
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
                    "AppData review finished: {count} possible leftover folder(s)."
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
                    "Registry review finished: {count} possible stale record(s). No changes made."
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
fn history_chart(
    ui: &mut Ui,
    window: &mut usize,
    title: &str,
    series: &[(&str, &crate::history::Series, Color32)],
    unit: &str,
) {
    {
        theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(title).font(theme::section_title()).strong());
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(RichText::new(unit).small().color(MUTED));
                    for (index, (label, _)) in History::WINDOWS.iter().enumerate().rev() {
                        let selected = *window == index;
                        if ui.selectable_label(selected, *label).clicked() {
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

            egui_plot::Plot::new(format!("chart-{title}"))
                .height(170.0)
                .allow_drag(false)
                .allow_zoom(false)
                .allow_scroll(false)
                .allow_boxed_zoom(false)
                .show_axes([true, true])
                // Let the card show through. The plot default paints
                // extreme_bg_color, which is sized for a thin gauge track and
                // reads as a heavy grey slab at chart size -- obvious on light.
                .show_background(false)
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
                .legend(egui_plot::Legend::default().position(egui_plot::Corner::LeftTop))
                .show(ui, |plot| {
                    for (name, data, color) in series {
                        let points = data.points(window_secs);
                        if points.len() >= 2 {
                            plot.line(
                                egui_plot::Line::new(*name, egui_plot::PlotPoints::from(points))
                                    .color(*color)
                                    .width(1.4),
                            );
                        }
                    }
                });
        });
    }
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
            if dark { "wordmark-dark" } else { "wordmark-light" },
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
            if let Some(c) = self.sensor_temperature("Cpu") {
                self.history.cpu_temp.push(c);
            }
            if let Some(c) = self.sensor_temperature("Gpu") {
                self.history.gpu_temp.push(c);
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

        let close = Panel::top(Id::new("title-bar"))
            .exact_size(chrome::TITLE_BAR_HEIGHT)
            .resizable(false)
            .frame(theme::chrome(ui))
            .show(ui, |ui| {
                chrome::title_bar(ui, self.tab.title()).close_requested
            })
            .inner;
        if close {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }

        self.sidebar(ui);
        self.status_bar(ui);
        self.body(ui);
        self.confirm_dialog(&ctx);
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
                        .clicked()
                        {
                            if self.tab == Tab::Storage && self.storage_expanded {
                                self.storage_expanded = false;
                            } else {
                                self.tab = Tab::Storage;
                                self.storage_view = StorageView::Summary;
                                self.storage_expanded = true;
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
                                                && self.storage_view == StorageView::Cleanup,
                                            chrome::icon::CLEANER,
                                            "Cleanup",
                                            accent,
                                            32.0 * expansion,
                                            expansion > 0.95,
                                        )
                                        .clicked()
                                    })
                                    .inner
                                })
                                .inner;
                            if clicked {
                                self.tab = Tab::Storage;
                                self.storage_view = StorageView::Cleanup;
                                self.storage_expanded = true;
                            }

                            let clicked = ui
                                .scope(|ui| {
                                    ui.set_opacity(expansion);
                                    ui.horizontal(|ui| {
                                        ui.add_space(32.0);
                                        chrome::nav_item_with_height(
                                            ui,
                                            self.tab == Tab::Storage
                                                && self.storage_view == StorageView::SizeExplorer,
                                            chrome::icon::CHART,
                                            "Size Explorer",
                                            accent,
                                            32.0 * expansion,
                                            expansion > 0.95,
                                        )
                                        .clicked()
                                    })
                                    .inner
                                })
                                .inner;
                            if clicked {
                                self.tab = Tab::Storage;
                                self.storage_view = StorageView::SizeExplorer;
                                self.storage_expanded = true;
                            }

                            let clicked = ui
                                .scope(|ui| {
                                    ui.set_opacity(expansion);
                                    ui.horizontal(|ui| {
                                        ui.add_space(32.0);
                                        chrome::nav_item_with_height(
                                            ui,
                                            self.tab == Tab::Storage
                                                && self.storage_view == StorageView::AppLeftovers,
                                            chrome::icon::APPS,
                                            "App Leftovers",
                                            accent,
                                            32.0 * expansion,
                                            expansion > 0.95,
                                        )
                                        .clicked()
                                    })
                                    .inner
                                })
                                .inner;
                            if clicked {
                                self.tab = Tab::Storage;
                                self.storage_view = StorageView::AppLeftovers;
                                self.storage_expanded = true;
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
                        .clicked()
                        {
                            if self.tab == Tab::Tools && self.tools_expanded {
                                self.tools_expanded = false;
                            } else {
                                self.tab = Tab::Tools;
                                self.tools_expanded = true;
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
                                self.tools_expanded = true;
                            }
                        }
                        continue;
                    }

                    let glyph = match tab {
                        Tab::Overview => chrome::icon::OVERVIEW,
                        Tab::SystemInfo => chrome::icon::SYSTEM_INFO,
                        Tab::Hardware => chrome::icon::DEVICES,
                        Tab::Memory => chrome::icon::MEMORY,
                        Tab::Storage => unreachable!(),
                        Tab::Tweaks => chrome::icon::TWEAKS,
                        Tab::Privacy => chrome::icon::SHIELD,
                        Tab::Troubleshoot | Tab::Repair | Tab::Startup => unreachable!(),
                        Tab::Diagnose => chrome::icon::DIAGNOSTICS,
                        Tab::Security => chrome::icon::SHIELD,
                        Tab::Tools => chrome::icon::TOOLS,
                        Tab::Backup => chrome::icon::BACKUP,
                        Tab::AiConnections => chrome::icon::TOOLS,
                        Tab::About => chrome::icon::ABOUT,
                    };
                    if chrome::nav_item(ui, selected, glyph, tab.title(), accent).clicked() {
                        self.tab = tab;
                    }
                }

                });

                ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
                    // Lift the switch off the window edge.
                    ui.add_space(theme::GAP);
                    let mut light = self.settings.light;
                    ui.horizontal(|ui| {
                        let glyph = if light {
                            chrome::icon::SUN
                        } else {
                            chrome::icon::MOON
                        };
                        ui.label(chrome::icon_text(glyph, 14.0, MUTED));
                        if chrome::toggle(ui, &mut light, accent).changed() {
                            self.settings.light = light;
                            self.settings.save();
                            theme::apply(ui.ctx(), light, accent);
                        }
                        ui.label(
                            RichText::new(if light { "Light" } else { "Dark" })
                                .small()
                                .color(MUTED),
                        );
                    });
                });
            });
    }

    fn status_bar(&mut self, ui: &mut Ui) {
        let accent = theme::accent_on(ui, self.accent);
        let mut elevate = false;
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
                        ui.label(
                            RichText::new(format!(
                                "{}  ·  {}  ·  up {}",
                                self.machine.host_name,
                                self.machine.windows_name,
                                self.machine.uptime
                            ))
                            .color(MUTED),
                        );
                    }

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(
                            RichText::new(format!(
                                "RAM {}  ·  {}",
                                human_bytes(self.machine.ram_used),
                                self.machine.app_arch
                            ))
                            .font(theme::readout())
                            .color(MUTED),
                        );
                        ui.label(RichText::new("·").small().color(MUTED));
                        if self.elevated {
                            ui.label(RichText::new("Administrator").small().color(theme::good(ui)));
                        } else if ui
                            .add(
                                egui::Label::new(
                                    RichText::new("Standard user").small().color(theme::warn(ui)),
                                )
                                .sense(egui::Sense::click()),
                            )
                            .on_hover_text("Click to restart Wrenchmaster as administrator.")
                            .clicked()
                        {
                            elevate = true;
                        }
                    });
                });
            });
        if elevate {
            let ctx = ui.ctx().clone();
            self.elevate_now(&ctx);
        }
    }

    fn body(&mut self, ui: &mut Ui) {
        let frame = egui::Frame::new()
            .fill(ui.visuals().panel_fill)
            .inner_margin(Margin::symmetric(20, 18));
        CentralPanel::default().frame(frame).show(ui, |ui| {
            ui.label(RichText::new(self.tab.title()).heading());
            ui.label(RichText::new(self.tab.blurb()).color(MUTED));
            ui.add_space(theme::GAP_SECTION);

            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| match self.tab {
                    Tab::Overview => self.page_overview(ui),
                    Tab::SystemInfo => self.page_system_info(ui),
                    Tab::Hardware => self.page_hardware(ui),
                    Tab::Memory => self.page_memory(ui),
                    Tab::Storage => self.page_storage(ui),
                    Tab::Tweaks => self.page_tweaks(ui),
                    Tab::Privacy => self.page_privacy(ui),
                    Tab::Troubleshoot => self.page_troubleshoot(ui),
                    Tab::Diagnose => self.page_diagnose(ui),
                    Tab::Security => self.page_security(ui),
                    Tab::Repair => self.page_repair(ui),
                    Tab::Tools => self.page_tools(ui),
                    Tab::Startup => self.page_startup(ui),
                    Tab::Backup => self.page_backup(ui),
                    Tab::AiConnections => self.page_ai_connections(ui),
                    Tab::About => self.page_about(ui),
                });
        });
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
            ui.label(RichText::new("Files are deleted, not moved to the Recycle Bin.").color(theme::warn(ui)));
            ui.add_space(theme::GAP_SECTION);
            ui.horizontal(|ui| {
                if ui
                    .add(|ui: &mut Ui| {
                        chrome::button(ui, chrome::Weight::Danger, None, "Delete now", accent)
                    })
                    .clicked()
                {
                    go = true;
                }
                if ui.button("Cancel").clicked() {
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
                if ui.button(RichText::new("Trim working set").strong()).clicked() {
                    trim = true;
                }
                if ui.button("Cancel").clicked() {
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
                if ui.button(RichText::new("Run memory action").strong()).clicked() {
                    run = true;
                }
                if ui.button("Cancel").clicked() {
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
        let health = Health::assess(&self.machine, junk);
        let mut scan = false;

        theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                health::face(ui, health.mood, 76.0);
                ui.add_space(theme::GAP);
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(health.score.to_string())
                                .font(theme::readout_large())
                                .color(health.mood.color(ui)),
                        );
                        ui.label(RichText::new("/ 100").color(MUTED));
                    });
                    ui.label(
                        RichText::new(health.mood.headline())
                            .strong()
                            .color(health.mood.color(ui)),
                    );
                    ui.add_space(theme::GAP_TIGHT);
                    for note in &health.notes {
                        ui.label(
                            RichText::new(format!("• {}", note.text))
                                .small()
                                .color(note.severity.color(ui)),
                        );
                    }
                    if junk.is_none() {
                        ui.add_space(theme::GAP_TIGHT);
                        if chrome::button(
                            ui,
                            chrome::Weight::Primary,
                            None,
                            "Measure cleanup space",
                            accent,
                        )
                        .clicked()
                        {
                            scan = true;
                        }
                    }
                });
            });
        });
        if scan {
            self.tab = Tab::Storage;
            self.storage_view = StorageView::Cleanup;
            self.storage_expanded = true;
            self.start_job(false);
        }

        ui.add_space(theme::GAP_SECTION);
        ui.label(
            RichText::new("Right now")
                .font(theme::section_title())
                .strong(),
        );
        ui.add_space(theme::GAP);

        let cpu = self.cpu_usage;
        let cpu_trend: Vec<f32> = self.history.cpu.values().collect();
        let ram_trend: Vec<f32> = self.history.ram.values().collect();
        let self_cpu_threads = self.cpu_monitor.cpus().len();
        let ram_fraction = self.machine.ram_fraction();
        let ram_text = self.machine.ram_summary();
        let disk = self
            .machine
            .disks
            .iter()
            .find(|d| d.mount.starts_with("C:"))
            .or_else(|| self.machine.disks.first())
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
        ui.columns(3, |columns| {
            let cpu_color = if cpu > 90.0 { warn_now } else { accent };
            reading_card(
                &mut columns[0],
                "Processor",
                &format!("{cpu:.0}%"),
                cpu_color,
                cpu / 100.0,
                &format!("{self_cpu_threads} threads"),
                Some(&cpu_trend),
            );

            let ram_color = if ram_fraction > 0.9 { bad_now } else { accent };
            reading_card(
                &mut columns[1],
                "Memory",
                &format!("{:.0}%", ram_fraction * 100.0),
                ram_color,
                ram_fraction,
                &ram_text,
                Some(&ram_trend),
            );

            match &disk {
                // A drive does not move minute to minute, so it keeps the
                // gauge: a flat sparkline would imply a reading that failed.
                Some((mount, fraction, free, total)) => reading_card(
                    &mut columns[2],
                    &format!("Drive {mount}"),
                    &format!("{:.0}%", fraction * 100.0),
                    if *fraction > 0.9 { bad_now } else { accent },
                    *fraction,
                    &format!("{free} free of {total}"),
                    None,
                ),
                None => reading_card(
                    &mut columns[2],
                    "Drive",
                    "--",
                    MUTED,
                    0.0,
                    "No drive reported",
                    None,
                ),
            }
        });

        ui.add_space(theme::GAP_SECTION);
        ui.label(RichText::new("Go to").font(theme::section_title()).strong());
        ui.add_space(theme::GAP);
        ui.columns(3, |columns| {
            for (index, (title, detail, tab, glyph)) in [
                (
                    "System Info",
                    "Windows and hardware",
                    Tab::SystemInfo,
                    chrome::icon::SYSTEM_INFO,
                ),
                (
                    "Storage",
                    "Memory and drive space",
                    Tab::Storage,
                    chrome::icon::STORAGE,
                ),
                (
                    "Diagnose",
                    "Review system health",
                    Tab::Diagnose,
                    chrome::icon::DIAGNOSTICS,
                ),
            ]
            .into_iter()
            .enumerate()
            {
                // The whole card is the target. A 60x22 "Open" button inside a
                // 92px card was a small thing to hit sitting in a large empty
                // one, which is the worst of both.
                let card = theme::card(&columns[index])
                    .show(&mut columns[index], |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.label(chrome::icon_text(glyph, 22.0, accent));
                            ui.add_space(theme::GAP_TIGHT);
                            ui.vertical(|ui| {
                                ui.label(
                                    RichText::new(title).font(theme::card_title()).strong(),
                                );
                                ui.label(RichText::new(detail).small().color(MUTED));
                            });
                        });
                    })
                    .response
                    .interact(egui::Sense::click());
                if card.hovered() {
                    columns[index].ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                if card.clicked() {
                    self.tab = tab;
                }
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
                    if ghost(ui, "Run activation scripts (downloads and runs MAS)").clicked() {
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
            match run_activation_scripts() {
                Ok(()) => self.say("Started the Microsoft Activation Scripts."),
                Err(error) => self.complain(format!("Could not start the activation scripts: {error}")),
            }
        }
        if ghost(ui, "Refresh system information").clicked() {
            self.machine = Machine::collect();
            self.machine_refreshed = Instant::now();
            self.say("System information refreshed.");
        }
    }

    fn page_hardware(&mut self, ui: &mut Ui) {
        let m = &self.machine;
        let accent = theme::accent_on(ui, self.accent);

        ui.horizontal(|ui| {
            ui.label(chrome::icon_text(chrome::icon::DEVICES, 20.0, accent));
            ui.label(RichText::new("Processor").font(theme::section_title()).strong());
        });
        ui.add_space(theme::GAP);
        theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(&m.cpu).strong());
            ui.label(RichText::new(&m.cpu_cores).small().color(MUTED));
            ui.add_space(theme::GAP);
            chrome::meter(
                ui,
                self.cpu_usage / 100.0,
                if self.cpu_usage > 90.0 { theme::warn(ui) } else { accent },
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
                ui.collapsing("Per logical processor", |ui| {
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
                    ui.label(
                        RichText::new(self.temperature_hint())
                            .small()
                            .color(if self.elevated { MUTED } else { theme::warn(ui) }),
                    );
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
            history_chart(
                ui,
                &mut self.chart_window,
                "Load and temperature",
                &[
                    ("CPU load %", &cpu, theme::ACCENT),
                    ("CPU \u{00b0}C", &cpu_t, theme::warn(ui)),
                    ("GPU \u{00b0}C", &gpu_t, theme::good(ui)),
                ],
                "% and \u{00b0}C",
            );
            self.history.cpu = cpu;
            self.history.cpu_temp = cpu_t;
            self.history.gpu_temp = gpu_t;
        }

        ui.add_space(theme::GAP_SECTION);
        ui.label(RichText::new("Graphics").font(theme::section_title()).strong());
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
                    ui.label(
                        RichText::new(m.ram_summary())
                            .small()
                            .color(MUTED),
                    );
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ghost(ui, "Open Memory").clicked() {
                        open_memory = true;
                    }
                });
            });
            chrome::meter(
                ui,
                m.ram_fraction(),
                if m.ram_fraction() > 0.9 { theme::warn(ui) } else { accent },
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
        ui.label(RichText::new("Storage volumes").font(theme::section_title()).strong());
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
                chrome::gauge(
                    ui,
                    fraction,
                    if fraction > 0.9 { theme::bad(ui) } else { accent },
                    Some(&format!(
                        "{} free of {}",
                        human_bytes(disk.available),
                        human_bytes(disk.total)
                    )),
                    0.9,
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
                ui.label(RichText::new("Drives").font(theme::section_title()).strong());
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
                chrome::gauge(
                    ui,
                    fraction,
                    if fraction > 0.9 { theme::bad(ui) } else { accent },
                    Some(&format!(
                        "{} free of {}",
                        human_bytes(disk.available),
                        human_bytes(disk.total)
                    )),
                    0.9,
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
                .clicked()
            {
                self.start_app_residue_scan();
            }
            if ui
                .add_enabled(
                    self.registry_audit_rx.is_none(),
                    Button::new("Review uninstall records"),
                )
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
                    ui.label(RichText::new("No possible leftover folders were found.").color(theme::good(ui)));
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
                        RichText::new("No likely stale uninstall records were found.").color(theme::good(ui)),
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
                    ui.label(RichText::new(&candidate.reason).small().color(theme::warn(ui)));
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
        para(ui, "Read common Run/RunOnce entries and Startup-folder shortcuts. Commands are inspected, never executed or changed.");
        ui.add_space(theme::GAP);
        if ui
            .add_enabled(
                self.startup_scan_rx.is_none(),
                Button::new("Scan startup locations"),
            )
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
                    RichText::new("No entries were found in the checked locations.").color(theme::good(ui)),
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
                        if resolved { theme::good(ui) } else { theme::warn(ui) },
                    ));
                    ui.vertical(|ui| {
                        ui.label(RichText::new(&entry.display_name).strong());
                        ui.label(RichText::new(source).small().color(MUTED));
                        ui.label(RichText::new(&entry.command_or_path).small().monospace());
                        if !entry.resolution_note.is_empty() {
                            ui.label(RichText::new(&entry.resolution_note).small().color(theme::warn(ui)));
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

        let started = busy || self.size_root.is_some();
        ui.horizontal(|ui| {
            if started
                && ui
                    .add_enabled(!busy, Button::new("Choose folder"))
                    .clicked()
            {
                choose_folder = true;
            }
            if let Some(root) = &self.size_root {
                ui.label(
                    RichText::new(root.display().to_string())
                        .small()
                        .color(MUTED),
                );
            }
            if busy {
                if ui.button("Stop scan").clicked() {
                    cancel = true;
                }
            } else if self.size_root.is_some() && ui.button("Scan again").clicked() {
                rescan = true;
            }
        });

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
            theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(format!(
                        "Reading {} entries · {} files · {}",
                        self.size_progress.visited_entries,
                        self.size_progress.files,
                        human_bytes(self.size_progress.bytes)
                    ));
                });
                ui.add(egui::ProgressBar::new(0.0).animate(true));
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
                    .clicked()
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
                ui.collapsing("Some locations could not be read", |ui| {
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
                });
            }
        });

        ui.add_space(theme::GAP);
        let mut show_files = self.size_show_files;
        ui.horizontal(|ui| {
            if ui.selectable_label(!show_files, "Folders").clicked() {
                show_files = false;
            }
            if ui.selectable_label(show_files, "Largest files").clicked() {
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
                        } else if ghost(ui, label).clicked() {
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
                            painter.text(
                                egui::pos2(row_rect.left() + 27.0, row_rect.center().y - 6.0),
                                egui::Align2::LEFT_CENTER,
                                &folder.name,
                                egui::FontId::proportional(14.0),
                                theme::text(ui),
                            );
                            painter.text(
                                egui::pos2(row_rect.right() - 6.0, row_rect.center().y - 6.0),
                                egui::Align2::RIGHT_CENTER,
                                format!("{}  ·  {:.1}%", human_bytes(folder.bytes), percent),
                                egui::FontId::proportional(13.0),
                                MUTED,
                            );
                            let bar = egui::Rect::from_min_max(
                                egui::pos2(row_rect.left() + 27.0, row_rect.bottom() - 5.0),
                                egui::pos2(row_rect.right() - 6.0, row_rect.bottom() - 2.0),
                            );
                            painter.rect_filled(bar, 2.0, ui.visuals().faint_bg_color);
                            let filled = egui::Rect::from_min_max(
                                bar.min,
                                egui::pos2(
                                    bar.left() + bar.width() * (percent / 100.0).clamp(0.0, 1.0),
                                    bar.bottom(),
                                ),
                            );
                            painter.rect_filled(filled, 2.0, self.accent);
                            let should_navigate = response.clicked() || response.double_clicked();
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
    /// on three unrelated pages -- Hardware, Diagnose and Storage > Cleanup --
    /// which meant three hops to answer one question.
    fn page_memory(&mut self, ui: &mut Ui) {
        let accent = theme::accent_on(ui, self.accent);
        {
            let m = &self.machine;
            theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(chrome::icon_text(chrome::icon::MEMORY, 20.0, accent));
                    ui.label(RichText::new("In use now").font(theme::section_title()).strong());
                });
                ui.add_space(theme::GAP_TIGHT);
                chrome::gauge(
                    ui,
                    m.ram_fraction(),
                    if m.ram_fraction() > 0.9 { theme::bad(ui) } else { accent },
                    Some(&m.ram_summary()),
                    0.9,
                );
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
            });
        }

        ui.add_space(theme::GAP);
        {
            let ram = std::mem::take(&mut self.history.ram);
            history_chart(
                ui,
                &mut self.chart_window,
                "Memory over time",
                &[("RAM %", &ram, theme::ACCENT)],
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
            ui.label(RichText::new("What is holding it").font(theme::section_title()).strong());
            if ghost(ui, "Refresh").clicked() {
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
                let in_use_fraction = if total == 0 {
                    0.0
                } else {
                    in_use as f32 / total as f32
                };
                chrome::meter(
                    ui,
                    in_use_fraction,
                    if total > 0 && in_use as f64 / total as f64 > 0.9 {
                        theme::warn(ui)
                    } else {
                        self.accent
                    },
                    Some(&format!(
                        "{} in use of {}",
                        human_bytes(in_use),
                        human_bytes(total)
                    )),
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
            if ui.button("Test RAM after restart").clicked() {
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
        ui.label(RichText::new("Free memory").font(theme::section_title()).strong());
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
                        if ui.button("Trim").clicked() {
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
                    if ui.button(action.title()).clicked() {
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
                    if chrome::button(
                        ui,
                        chrome::Weight::Primary,
                        None,
                        "Scan selected",
                        accent,
                    )
                    .on_hover_text("Measures without touching anything.")
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
                    if chrome::button(ui, weight, None, label, accent).clicked() {
                        clean = true;
                    }
                });
                if busy
                    && chrome::button(ui, chrome::Weight::Ghost, None, "Cancel", accent).clicked()
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
                ui.label(
                    RichText::new(format!("{} selected", self.selected_count())).color(MUTED),
                );
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
            if ghost(ui, "Select all").clicked() {
                for t in &mut self.targets {
                    t.selected = true;
                }
                selection_changed = true;
            }
            if ghost(ui, "Select none").clicked() {
                for t in &mut self.targets {
                    t.selected = false;
                }
                selection_changed = true;
            }
            if ghost(ui, "Safe set only")
                .on_hover_text("Per-user caches that need no elevation.")
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
            ui.painter().hline(
                ui.min_rect().x_range(),
                y,
                egui::Stroke::new(1.0, hairline),
            );
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
                if ui.button("Re-read current values").clicked() {
                    reread = true;
                }
                if ui
                    .button("Restart Explorer")
                    .on_hover_text("Applies the shell tweaks without signing out.")
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
                                        .on_disabled_hover_text(
                                            "Administrator access is required.",
                                        )
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
                    if ui.button("Run").clicked() {
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

    fn page_diagnose(&mut self, ui: &mut Ui) {
        let accent = theme::accent_on(ui, self.accent);
        if self.diagnose_event_rx.is_none() && self.diagnose_event_result.is_none() {
            self.start_diagnosis();
        }
        let scanned = self.scanned_total();
        let junk = self
            .targets
            .iter()
            .any(|target| target.scan.is_some())
            .then_some(scanned.bytes);
        let health = Health::assess(&self.machine, junk);

        theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                health::face(ui, health.mood, 64.0);
                ui.add_space(theme::GAP);
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(health.score.to_string())
                                .font(theme::readout_large())
                                .color(health.mood.color(ui)),
                        );
                        ui.label(RichText::new("/ 100 quick health snapshot").color(MUTED));
                    });
                    ui.label(
                        RichText::new(health.mood.headline())
                            .strong()
                            .color(health.mood.color(ui)),
                    );
                });
            });
            ui.add_space(theme::GAP);
            ui.label(
                RichText::new(
                    "Diagnosis checks live CPU, memory and drive pressure, and automatically reviews recent Windows System and Application errors.",
                )
                .small()
                .color(MUTED),
            );
        });

        ui.add_space(theme::GAP);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Current performance").strong());
            if ui
                .add_enabled(
                    self.diagnose_event_rx.is_none(),
                    Button::new(if self.diagnose_event_result.is_some() {
                        "Run diagnosis again"
                    } else {
                        "Run diagnosis"
                    }),
                )
                .clicked()
            {
                self.start_diagnosis();
            }
        });
        ui.add_space(theme::GAP);
        theme::card(ui).show(ui, |ui| {
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
                ui.collapsing("Largest current CPU users", |ui| {
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
                });
            }
            if let Some(memory) = &self.ram_snapshot {
                let used = memory
                    .physical_total_bytes
                    .saturating_sub(memory.physical_available_bytes);
                let fraction = if memory.physical_total_bytes == 0 {
                    0.0
                } else {
                    used as f32 / memory.physical_total_bytes as f32
                };
                chrome::meter(
                    ui,
                    fraction,
                    if fraction > 0.90 { theme::warn(ui) } else { self.accent },
                    Some(&format!(
                        "RAM in use  {} of {}",
                        human_bytes(used),
                        human_bytes(memory.physical_total_bytes)
                    )),
                );
                if fraction > 0.90 {
                    ui.label(
                        RichText::new("Available physical memory is low right now.").color(theme::warn(ui)),
                    );
                }
            } else if let Some(error) = &self.ram_error {
                ui.label(
                    RichText::new(format!("Memory counters unavailable: {error}"))
                        .small()
                        .color(MUTED),
                );
            }
            let low_space: Vec<_> = self
                .machine
                .disks
                .iter()
                .filter(|disk| disk.used_fraction() >= 0.90)
                .collect();
            if low_space.is_empty() {
                ui.label(
                    RichText::new("Mounted drives: no drive is above 90% full.")
                        .small()
                        .color(MUTED),
                );
            } else {
                for disk in low_space {
                    ui.label(
                        RichText::new(format!(
                            "Low free space: {} is {:.0}% full ({} free)",
                            disk.mount.trim_end_matches('\\'),
                            disk.used_fraction() * 100.0,
                            human_bytes(disk.available)
                        ))
                        .color(theme::warn(ui)),
                    );
                }
            }
            if !self.ram_processes.is_empty() {
                ui.collapsing("Largest process memory use", |ui| {
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
                });
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
        });

        ui.add_space(theme::GAP);
        ui.label(RichText::new("Recent Windows errors").strong());
        ui.add_space(theme::GAP);
        theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("System and Application logs · Critical and Error events · past 24 hours").small().color(MUTED));
            if self.diagnose_event_rx.is_some() {
                ui.add_space(theme::GAP);
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Reading Windows events…");
                });
            } else if let Some(result) = &self.diagnose_event_result {
                match &result.status {
                    ScanStatus::Complete => ui.label(format!("Checked both logs. {} critical/error event(s) found.", result.findings.len())),
                    ScanStatus::Partial => ui.label(RichText::new(format!("Partial event check. {} event(s) found; some log data could not be read.", result.findings.len())).color(theme::warn(ui))),
                    ScanStatus::Unsupported => ui.label(RichText::new("Windows event checks are available on Windows only.").color(MUTED)),
                    ScanStatus::Error(error) => ui.label(RichText::new(format!("Could not check Windows events: {error}")).color(theme::warn(ui))),
                };
                for error in &result.log_errors {
                    ui.label(RichText::new(error).small().color(theme::warn(ui)));
                }
                if result.findings.is_empty() && matches!(&result.status, ScanStatus::Complete) {
                    ui.add_space(theme::GAP_TIGHT);
                    ui.label(RichText::new("No Critical or Error events were recorded in the selected logs during this period.").color(theme::good(ui)));
                } else if !result.findings.is_empty() {
                    ui.add_space(theme::GAP);
                    ui.label(RichText::new("These entries are clues to investigate; a log event alone does not prove the cause of a slowdown.").small().color(MUTED));
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
        });

        ui.add_space(theme::GAP);
        ui.label(RichText::new("Other health checks").strong());
        ui.add_space(theme::GAP);
        theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            for note in &health.notes {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("●").color(note.severity.color(ui)));
                    ui.label(&note.text);
                });
                ui.add_space(theme::GAP_TIGHT);
            }
        });

        ui.add_space(theme::GAP);
        let mut open_memory = false;
        theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(chrome::icon_text(chrome::icon::MEMORY, 20.0, accent));
                ui.vertical(|ui| {
                    ui.label(RichText::new("Memory").strong());
                    ui.label(
                        RichText::new(self.machine.ram_summary())
                            .small()
                            .color(MUTED),
                    );
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ghost(ui, "Open Memory").clicked() {
                        open_memory = true;
                    }
                });
            });
        });
        if open_memory {
            self.tab = Tab::Memory;
        }

        ui.add_space(theme::GAP);
        ui.label(RichText::new("Next steps").strong());
        ui.add_space(theme::GAP);
        ui.horizontal(|ui| {
            if ui.button("Open Storage").clicked() {
                self.tab = Tab::Storage;
                self.storage_view = StorageView::Summary;
                self.storage_expanded = true;
            }
            if ui.button("Open Cleanup").clicked() {
                self.tab = Tab::Storage;
                self.storage_view = StorageView::Cleanup;
                self.storage_expanded = true;
            }
            if ui.button("RAM cleanup").clicked() {
                self.tab = Tab::Storage;
                self.storage_view = StorageView::Cleanup;
                self.storage_expanded = true;
            }
            if ui.button("Open Troubleshoot").clicked() {
                self.tab = Tab::Troubleshoot;
            }
        });
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
            if ui.add_enabled(!checking_integrity, Button::new("Check Explorer integrity")).clicked() {
                self.explorer_integrity_result = None;
                self.explorer_integrity_rx = Some(security_integrity::start_scan());
            }
            if ui.button("Verify Windows copy with SFC").on_hover_text("Runs the Windows verify-only check in a console; it does not repair files.").clicked() {
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
            theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
                ui.label(RichText::new("Windows Explorer executable").strong());
                let status = match &result.status {
                    security_integrity::ScanStatus::Complete => "Inventory complete",
                    security_integrity::ScanStatus::Partial => "Inventory partial",
                    security_integrity::ScanStatus::Unsupported => "Unsupported system",
                    security_integrity::ScanStatus::Error(_) => "Check failed",
                };
                ui.label(RichText::new(status).color(
                    if matches!(&result.status, security_integrity::ScanStatus::Complete) {
                        theme::good(ui)
                    } else {
                        theme::warn(ui)
                    },
                ));
                if let security_integrity::ScanStatus::Error(error) = &result.status {
                    ui.label(RichText::new(error).small().color(theme::warn(ui)));
                }
                ui.label(
                    RichText::new(&result.system_explorer_path)
                        .small()
                        .monospace(),
                );
                let system_signature_valid =
                    result.system_signature_status.eq_ignore_ascii_case("Valid");
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
                    .color(if system_signature_valid { theme::good(ui) } else { theme::warn(ui) }),
                );
                ui.add_space(theme::GAP);
                ui.label(
                    RichText::new(format!(
                        "Running explorer.exe instances: {}",
                        result.processes.len()
                    ))
                    .strong(),
                );
                for process in &result.processes {
                    let expected_path = process
                        .image_path
                        .eq_ignore_ascii_case(&result.system_explorer_path);
                    let signature_valid = process.signature_status.eq_ignore_ascii_case("Valid");
                    let suspicious_context = !expected_path || !signature_valid;
                    ui.label(
                        RichText::new(format!(
                            "PID {} · parent {} · session {} · {}",
                            process.pid, process.parent_pid, process.session_id, process.image_path
                        ))
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
                }
                for note in &result.notes {
                    ui.label(RichText::new(note).small().color(MUTED));
                }
            });
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
                egui::CollapsingHeader::new(format!("{title} · {}", connections.len()))
                    .default_open(!connections.is_empty())
                    .show(ui, |ui| {
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
                    });
            }
            para(ui, "A connection or unusual port is not proof of malware. Use Defender for a full system scan. This view is a snapshot, not continuous packet monitoring.");
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
        para(ui, "MalwareBazaar checks a selected file's SHA-256 against its malware database. The file is hashed locally; only its hash is sent. This is opt-in and requires an Auth-Key. No file contents are uploaded or samples downloaded.");
        ui.add_space(theme::GAP_TIGHT);
        ui.hyperlink_to(
            "MalwareBazaar API and key information",
            "https://bazaar.abuse.ch/api/",
        );
        ui.horizontal(|ui| {
            if ui.button("Choose file…").clicked() {
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
            if ui.button("Choose folder…").clicked() {
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
                        ui.label(RichText::new(&finding.path).small().monospace().color(theme::warn(ui)));
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
                        if ui.button("Show folder").clicked() {
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
                theme::card(target).show(target, |ui| {
                    // Fixed tiles, so the grid lines up and the Open buttons
                    // share a baseline.
                    ui.set_min_height(148.0);
                    ui.set_max_height(148.0);
                    ui.label(chrome::icon_text(glyph, 24.0, accent));
                    ui.add_space(theme::GAP_TIGHT);
                    ui.label(RichText::new(&action.name).strong());
                    // ui.columns hands down a *justified* layout, which stretches
                    // a label that wraps -- that is what spaced the words of the
                    // longer descriptions out across the line.
                    ui.with_layout(Layout::top_down(Align::Min), |ui| {
                        ui.label(RichText::new(&action.what).small().color(MUTED));
                    });
                    if action.needs_admin {
                        ui.label(RichText::new("Administrator required").small().color(theme::warn(ui)));
                    }
                    ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
                        if ui.button("Open").clicked() {
                            launch = Some(action.clone());
                        }
                    });
                });
                target.add_space(10.0);
            }
        });
        if let Some(action) = launch {
            match action.launch() {
                Ok(()) => self.say(format!("Launched {}.", action.name)),
                Err(e) => self.complain(format!("{}: {e}", action.name)),
            }
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
            match action.launch() {
                Ok(()) => self.say(format!("Launched {}.", action.name)),
                Err(e) => self.complain(format!("{}: {e}", action.name)),
            }
        }
    }

    fn page_backup(&mut self, ui: &mut Ui) {
        self.elevation_banner(ui, "Restore points need administrator rights.");
        let mut export = false;
        let mut restore_point = false;
        let mut protection = false;

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
                .button(RichText::new("Choose folder and export").strong())
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
                .add_enabled(self.elevated, Button::new("Create restore point"))
                .on_disabled_hover_text("Needs administrator rights.")
                .clicked()
            {
                restore_point = true;
            }
            ui.add_space(theme::GAP_TIGHT);
            if ghost(ui, "Open System Protection settings").clicked() {
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
                match tools::export_registry(&dir) {
                    Ok(files) => {
                        let count = files.len();
                        self.backups = files;
                        self.settings.remember_backup_dir(&dir);
                        self.say(format!("Exported {count} hives to {}.", dir.display()));
                    }
                    Err(e) => self.complain(e),
                }
            }
        }
        if restore_point {
            match tools::create_restore_point() {
                Ok(()) => self.say("Restore point created."),
                Err(e) => self.complain(e),
            }
        }
        if protection {
            match crate::util::spawn("SystemPropertiesProtection.exe", &[]) {
                Ok(()) => self.say("Opened System Protection."),
                Err(e) => self.complain(format!("Could not open it: {e}")),
            }
        }
    }

    fn page_ai_connections(&mut self, ui: &mut Ui) {
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

        ui.label(RichText::new("MCP server connection").font(theme::section_title()).strong());
        ui.label(
            RichText::new(
                "Connect a local Model Context Protocol server. This page only connects and lists tools; it does not run them or send data to Claude/OpenAI.",
            )
            .small()
            .color(MUTED),
        );
        ui.add_space(theme::GAP);

        theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("Server program").strong());
            ui.add(
                egui::TextEdit::singleline(&mut self.mcp_program)
                    .hint_text(r"Full path or executable name, e.g. C:\Tools\my-mcp-server.exe")
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
            ui.add_space(theme::GAP);
            ui.label(
                RichText::new(
                    "Only connect to a server you trust. Connecting starts the program above with these arguments. Its stdout is reserved for MCP messages.",
                )
                .small()
                .color(theme::warn(ui)),
            );
            ui.add_space(theme::GAP);
            ui.horizontal(|ui| {
                let connected = self.mcp_connection.is_some();
                if ui
                    .add_enabled(
                        !connected && !self.mcp_program.trim().is_empty(),
                        Button::new("Connect"),
                    )
                    .clicked()
                {
                    let args = self
                        .mcp_args
                        .lines()
                        .map(str::trim)
                        .filter(|line| !line.is_empty())
                        .map(str::to_string)
                        .collect();
                    self.mcp_connection = Some(crate::mcp_client::Connection::start(
                        self.mcp_program.trim().to_string(),
                        args,
                    ));
                    self.mcp_status = "Starting server and requesting its tools…".into();
                    self.mcp_server = None;
                    self.mcp_tools.clear();
                }
                if ui
                    .add_enabled(connected, Button::new("Disconnect"))
                    .clicked()
                {
                    if let Some(mut connection) = self.mcp_connection.take() {
                        connection.disconnect();
                    }
                    self.mcp_status = "Disconnected".into();
                    self.mcp_server = None;
                    self.mcp_tools.clear();
                }
                ui.label(RichText::new(&self.mcp_status).color(if self.mcp_status.starts_with("Connection failed") {
                    theme::bad(ui)
                } else if self.mcp_status.starts_with("Connected") {
                    theme::good(ui)
                } else {
                    MUTED
                }));
            });
        });

        if let Some((name, version)) = &self.mcp_server {
            ui.add_space(theme::GAP);
            theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
                ui.label(RichText::new(name).font(theme::section_title()).strong());
                ui.label(
                    RichText::new(format!("Version {version}"))
                        .small()
                        .color(MUTED),
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
                    ui.label(RichText::new(&tool.name).strong());
                    if !tool.description.is_empty() {
                        ui.label(RichText::new(&tool.description).small().color(MUTED));
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

        if self.mcp_status.starts_with("Starting") {
            ui.ctx().request_repaint_after(Duration::from_millis(100));
        }
    }

    fn page_about(&mut self, ui: &mut Ui) {
        let accent = theme::accent_on(ui, self.accent);
        theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("Windows Wrenchmaster").font(theme::section_title()).strong());
            ui.label(
                RichText::new(format!(
                    "Version {} — offline build, Rust + egui",
                    env!("CARGO_PKG_VERSION")
                ))
                .color(MUTED),
            );
            ui.add_space(theme::GAP);
            ui.label(
                "A rebuild of a VB.NET WinForms tool from 2017: a splash screen that phoned a \
                 license server, a tabbed window of mostly unimplemented buttons, and a Clean \
                 button that walked C:\\ deleting every .log file it could reach.",
            );
            ui.add_space(theme::GAP);
            ui.label(RichText::new("What changed").strong());
            ui.add_space(theme::GAP_TIGHT);
            for line in [
                "No network at all: no license server, no avatar fetch, no update check.",
                "The cleaner works from a fixed list of known caches and always measures first.",
                "Dry run is the default, and a real delete asks once before it runs.",
                "Scanning and deleting run on a worker thread, with a cancel button.",
                "Every tweak reads its registry value; removing it clears the value Wrenchmaster manages, but does not recover prior custom data.",
                "System info comes from the registry, so no WMI error dialogs.",
                "Failures land in the status bar instead of an empty Catch block.",
            ] {
                ui.label(format!("• {line}"));
            }
            ui.add_space(theme::GAP);
            ui.label(RichText::new("Changelog").strong());
            ui.add_space(theme::GAP_TIGHT);
            for (version, note) in [
                (
                    "v2.0",
                    "Rewritten in Rust as an offline tool: working cleaner, reversible tweaks, \
                     repair commands, registry export and restore points.",
                ),
                (
                    "v0.9",
                    "Last VB.NET build. License check against a remote server, log cleaner, \
                     seven tabs with six of them placeholders.",
                ),
                (
                    "v0.1",
                    "First version, as recorded in the original Changelog.txt.",
                ),
            ] {
                ui.label(RichText::new(version).strong().color(accent));
                ui.label(RichText::new(note).small().color(MUTED));
                ui.add_space(theme::GAP_TIGHT);
            }
            ui.add_space(theme::GAP);
            ui.label(
                RichText::new(
                    "Deletes are permanent. Explicit registry changes may replace a custom value; removing the Wrenchmaster policy value does not restore prior data. Create a restore point before broad changes.",
                )
                .small()
                .color(MUTED),
            );
        });
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
    egui::CollapsingHeader::new(format!(
        "{} · Event {} · {}",
        finding.source, finding.event_id, finding.time
    ))
    // The position in the list is what makes this unique. Windows really does
    // log the same source, event id and timestamp more than once, and keying
    // only on those made two rows share an id -- which egui reports by painting
    // "First/Second use of widget ID" over the page.
    .id_salt((
        "diagnostic-event",
        index,
        &finding.source,
        finding.event_id,
        &finding.time,
    ))
    .show(ui, |ui| {
        ui.label(RichText::new(&finding.message).small());
    });
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

/// One live reading: name, figure, gauge, caption.
///
/// Every card in the Overview row is built from this, which is why they line
/// up -- they are the same shape, rather than three different shapes pinned to
/// the same height. Change the shape here and the row stays aligned.
fn reading_card(
    ui: &mut Ui,
    name: &str,
    value: &str,
    color: Color32,
    fraction: f32,
    caption: &str,
    trend: Option<&[f32]>,
) {
    theme::card(ui).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new(name).small().color(MUTED));
        ui.label(
            RichText::new(value)
                .font(theme::readout_large())
                .color(color),
        );
        match trend {
            // Where there is history, the shape of the last few minutes says
            // more than the bar: it separates "busy" from "spiking".
            Some(values) if values.len() >= 2 => {
                chrome::sparkline(ui, values, color, 100.0);
                ui.label(RichText::new(caption).small().color(MUTED));
            }
            _ => {
                chrome::gauge(ui, fraction, color, Some(caption), 0.9);
            }
        }
    });
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
    let card = theme::card(ui)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
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
    card.clicked()
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
