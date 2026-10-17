// Windows Wrenchmaster — modern offline rebuild.
// No console window in release; the GUI is the whole program.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod ai_config;
mod app;
mod app_residue;
mod appearance;
mod chrome;
mod cleaner;
mod diagnose;
mod health;
mod history;
mod machine;
mod malware_lookup;
mod mcp_client;
mod network_watch;
mod ram;
mod registry_audit;
mod security_integrity;
mod security_scan;
mod settings;
mod startup;
mod theme;
mod tools;
mod treesize;
mod tweaks;
mod util;

fn main() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        startup_log(&format!("panic: {info}"));
        default_hook(info);
    }));
    startup_log("process entered main");
    if let Err(error) = run_app() {
        startup_log(&format!("startup error: {error}"));
        let _ = rfd::MessageDialog::new()
            .set_title("Windows Wrenchmaster could not start")
            .set_description(error.to_string())
            .show();
    }
}

fn startup_log(message: &str) {
    use std::io::Write;
    let path = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("wrenchmaster-startup.log")))
        .unwrap_or_else(|| std::env::temp_dir().join("wrenchmaster-startup.log"));
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(file, "pid {}: {message}", std::process::id());
    }
}

fn run_app() -> Result<(), Box<dyn std::error::Error>> {
    // Two gates, in this order, and the order is the whole subtlety. Neither
    // applies in development: the dev launcher rebuilds and restarts the app
    // on every save, so a second copy starting while the first is still
    // closing is the normal case there, not a mistake to correct. Enforcing
    // one instance against that loop means the rebuilt app raises the window
    // of the build it was meant to replace and exits -- which looks exactly
    // like hot reload having stopped working.
    if !util::is_development_mode() && util::focus_existing_window() {
        startup_log("a window is already open; raised it and exited");
        return Ok(());
    }
    startup_log("requesting/checking administrator access");
    let elevated = util::request_elevation_at_startup()?;
    startup_log(&format!("elevation check returned {elevated}"));
    if !elevated {
        return Ok(());
    }
    // Then the mutex, and only now -- after the handoff, in the process that
    // will actually own a window. Claiming it before the handoff made the
    // launcher hold the name and the elevated copy it had just spawned find
    // the name taken and exit, so the app closed instead of starting. The
    // mutex is still worth having for the moment the window has not been
    // created yet, which the title search cannot see.
    if !util::is_development_mode() && !util::claim_single_instance() {
        startup_log("another instance already holds the single-instance name");
        util::focus_existing_window();
        return Ok(());
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1080.0, 660.0])
            .with_min_inner_size([880.0, 560.0])
            .with_position(egui::pos2(80.0, 80.0))
            .with_visible(true)
            .with_decorations(false)
            .with_title("Windows Wrenchmaster")
            .with_icon(icon()),
        ..Default::default()
    };

    startup_log("entering egui window loop");
    let result = eframe::run_native(
        "Windows Wrenchmaster",
        options,
        Box::new(|cc| {
            startup_log("native window created; building app contents");
            let app = app::App::new(cc);
            startup_log("app contents ready");
            Ok(Box::new(app))
        }),
    );
    startup_log(&format!("egui window loop returned: {}", result.is_ok()));
    Ok(result?)
}

/// The original wrench, decoded at startup so the exe stays a single file.
fn icon() -> egui::IconData {
    let (width, height, rgba) = chrome::official_icon_rgba();
    egui::IconData {
        rgba,
        width,
        height,
    }
}
