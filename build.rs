//! Windows resource metadata for the shipped executable.
//!
//! Without this the exe has no icon in Explorer or the taskbar, a blank
//! Properties -> Details tab, and no declared DPI awareness. The first two
//! also cost SmartScreen reputation, which for an unsigned maintenance tool
//! that asks for administrator rights is the difference between a warning
//! the user clicks through and one they back out of.
//!
//! The window icon set at runtime through eframe is a separate thing: it
//! dresses the window once the app is up and does nothing for the file on
//! disk or the shortcut the user pins.

fn main() {
    // Nothing here applies anywhere else, and the resource compiler only
    // exists on a Windows toolchain.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    println!("cargo:rerun-if-changed=assets/wrench.ico");
    println!("cargo:rerun-if-changed=assets/wrenchmaster.manifest");
    println!("cargo:rerun-if-changed=build.rs");

    let mut resource = winresource::WindowsResource::new();
    resource.set_icon("assets/wrench.ico");
    resource.set_manifest_file("assets/wrenchmaster.manifest");
    resource.set("FileDescription", "Windows Wrenchmaster");
    resource.set("ProductName", "Windows Wrenchmaster");
    resource.set("OriginalFilename", "wrenchmaster.exe");
    resource.set("LegalCopyright", "MIT licensed");

    // A failed resource build must not fail the crate build. A developer on a
    // machine without the resource compiler should still get a working exe,
    // just an undressed one; the release build is where this matters and it
    // is loud there because the icon is simply absent.
    if let Err(error) = resource.compile() {
        println!("cargo:warning=could not embed Windows resources: {error}");
    }
}
