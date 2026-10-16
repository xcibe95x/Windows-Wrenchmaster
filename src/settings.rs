//! Persisted preferences — the modern stand-in for `My Project\Settings.settings`.
//!
//! The 2017 build stored `WindowsVersion`, `UniqueKey`, `Windows`, `Hash`,
//! `ProductKey` and friends in user settings; all of those were either license
//! plumbing or values better read fresh from the registry. What is actually
//! worth remembering between runs is how you left the UI, so that is all this
//! keeps: a plain `key = value` file under %APPDATA%, no serde, no registry.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub light: bool,
    pub dry_run: bool,
    /// Cleaner targets the user explicitly unticked, remembered by name so the
    /// file survives the target list growing or shrinking between versions.
    pub deselected: BTreeSet<String>,
    pub last_backup_dir: Option<PathBuf>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            light: false,
            // Safety default: never arrive armed to delete.
            dry_run: true,
            deselected: BTreeSet::new(),
            last_backup_dir: None,
        }
    }
}

impl Settings {
    pub fn path() -> Option<PathBuf> {
        crate::util::expand("%APPDATA%").map(|p| p.join("Wrenchmaster").join("settings.ini"))
    }

    pub fn load() -> Self {
        Self::path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map(|text| Self::parse(&text))
            .unwrap_or_default()
    }

    pub fn parse(text: &str) -> Self {
        let mut settings = Self::default();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim();
            match key.trim() {
                "light" => settings.light = value == "true",
                "dry_run" => settings.dry_run = value != "false",
                "deselected" => {
                    settings.deselected = value
                        .split('|')
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string)
                        .collect();
                }
                "last_backup_dir" if !value.is_empty() => {
                    settings.last_backup_dir = Some(PathBuf::from(value));
                }
                _ => {}
            }
        }
        settings
    }

    pub fn serialize(&self) -> String {
        let mut out = String::from("# Windows Wrenchmaster preferences\n");
        out.push_str(&format!("light = {}\n", self.light));
        out.push_str(&format!("dry_run = {}\n", self.dry_run));
        out.push_str(&format!(
            "deselected = {}\n",
            self.deselected
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join("|")
        ));
        out.push_str(&format!(
            "last_backup_dir = {}\n",
            self.last_backup_dir
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default()
        ));
        out
    }

    /// Best effort: a machine with a locked-down %APPDATA% still runs fine,
    /// it just forgets. Never surfaces an error for something this small.
    pub fn save(&self) {
        let Some(path) = Self::path() else { return };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(path, self.serialize());
    }

    pub fn remember_backup_dir(&mut self, dir: &Path) {
        self.last_backup_dir = Some(dir.to_path_buf());
        self.save();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let mut original = Settings {
            light: true,
            dry_run: false,
            deselected: ["Prefetch data", "Recycle Bin"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            last_backup_dir: Some(PathBuf::from(r"D:\backups")),
        };
        let parsed = Settings::parse(&original.serialize());
        assert_eq!(parsed, original);

        original.deselected.clear();
        original.last_backup_dir = None;
        assert_eq!(Settings::parse(&original.serialize()), original);
    }

    #[test]
    fn defaults_are_safe_and_tolerate_junk() {
        let s = Settings::parse("garbage\n\n# comment\nlight = yes\n");
        assert!(!s.light, "anything but `true` means dark");
        assert!(s.dry_run, "dry run must default on");
        assert!(s.deselected.is_empty());
    }
}
