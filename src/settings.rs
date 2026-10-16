//! Persisted preferences — the modern stand-in for `My Project\Settings.settings`.
//!
//! The 2017 build stored `WindowsVersion`, `UniqueKey`, `Windows`, `Hash`,
//! `ProductKey` and friends in user settings; all of those were either license
//! plumbing or values better read fresh from the registry. What is actually
//! worth remembering between runs is how you left the UI, so that is all this
//! keeps: a plain `key = value` file under %APPDATA%, no serde, no registry.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Which palette the app paints with.
///
/// This was a plain `light: bool` for as long as there were only two answers.
/// A third was added because the per-colour overrides in `appearance` used to
/// be applied in every mode: a background hand-picked while dark stayed put
/// after a switch to light and the text went with the light ramp, so half the
/// window was unreadable. Those overrides now belong to `Custom` alone, which
/// leaves switching between `Dark` and `Light` lossless.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Dark,
    Light,
    Custom,
}

impl Mode {
    /// The word on the segmented control. It doubles as the persisted token so
    /// the file stays something you can read and edit by hand, which is the
    /// whole reason this is an ini and not serde.
    pub fn label(self) -> &'static str {
        match self {
            Mode::Dark => "Dark",
            Mode::Light => "Light",
            Mode::Custom => "Custom",
        }
    }

    /// The inverse of [`Mode::label`], case-insensitive and forgiving: an
    /// unrecognised token reads as `None` so the caller can fall back rather
    /// than resetting someone's preference over a typo.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "dark" => Some(Mode::Dark),
            "light" => Some(Mode::Light),
            "custom" => Some(Mode::Custom),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub mode: Mode,
    /// Which of the two ramps `Mode::Custom` is built on. Custom overrides a
    /// handful of surfaces, not all of them; everything it leaves alone --
    /// scrollbars, selection, disabled text -- still comes from a real theme,
    /// so one has to be named.
    pub custom_light: bool,
    pub dry_run: bool,
    /// Cleaner targets the user explicitly unticked, remembered by name so the
    /// file survives the target list growing or shrinking between versions.
    pub deselected: BTreeSet<String>,
    pub last_backup_dir: Option<PathBuf>,
    /// Records that the passive 30-second startup baseline was collected.
    pub first_run_baseline_completed: bool,
    /// Last completed health score, retained while the next silent check runs.
    pub last_health_score: Option<u8>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            mode: Mode::Dark,
            custom_light: false,
            // Safety default: never arrive armed to delete.
            dry_run: true,
            deselected: BTreeSet::new(),
            last_backup_dir: None,
            first_run_baseline_completed: false,
            last_health_score: None,
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
        // Both keys are collected and resolved after the loop because the
        // older `light` boolean is only a fallback; deciding inside the loop
        // would make the outcome depend on which line came first.
        let mut mode = None;
        let mut legacy_light = None;
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
                "mode" => mode = Mode::parse(value),
                "custom_light" => settings.custom_light = value == "true",
                "light" => legacy_light = Some(value == "true"),
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
                "first_run_baseline_completed" => {
                    settings.first_run_baseline_completed = value == "true";
                }
                "last_health_score" => {
                    settings.last_health_score = value.parse::<u8>().ok().filter(|s| *s <= 100);
                }
                _ => {}
            }
        }
        // A file written before Custom existed has no `mode` key, only the old
        // boolean. Honour it, otherwise everyone who had chosen light would
        // silently be moved back to dark on the first run after upgrading.
        settings.mode = mode.unwrap_or(if legacy_light.unwrap_or(false) {
            Mode::Light
        } else {
            Mode::Dark
        });
        settings
    }

    /// The ramp currently on screen: the mode itself, or the base Custom was
    /// built on. Everything that needs to know whether it is painting dark or
    /// light asks here, so no caller has to spell out the Custom case.
    pub fn light(&self) -> bool {
        match self.mode {
            Mode::Dark => false,
            Mode::Light => true,
            Mode::Custom => self.custom_light,
        }
    }

    /// Flip between the two ramps. In Custom this moves the base rather than
    /// leaving the mode, so the status-bar sun/moon switch keeps meaning the
    /// same thing without discarding a palette you assembled.
    pub fn toggle_light(&mut self) {
        match self.mode {
            Mode::Dark => self.mode = Mode::Light,
            Mode::Light => self.mode = Mode::Dark,
            Mode::Custom => self.custom_light = !self.custom_light,
        }
    }

    pub fn serialize(&self) -> String {
        let mut out = String::from("# Windows Wrenchmaster preferences\n");
        out.push_str(&format!(
            "mode = {}\n",
            self.mode.label().to_ascii_lowercase()
        ));
        out.push_str(&format!("custom_light = {}\n", self.custom_light));
        // Still written, and still read by `parse` when `mode` is missing, so
        // a file round-trips through an older build without losing the theme.
        out.push_str(&format!("light = {}\n", self.light()));
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
        out.push_str(&format!(
            "first_run_baseline_completed = {}\n",
            self.first_run_baseline_completed
        ));
        out.push_str(&format!(
            "last_health_score = {}\n",
            self.last_health_score
                .map(|score| score.to_string())
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
            mode: Mode::Custom,
            custom_light: true,
            dry_run: false,
            deselected: ["Prefetch data", "Recycle Bin"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            last_backup_dir: Some(PathBuf::from(r"D:\backups")),
            first_run_baseline_completed: true,
            last_health_score: Some(88),
        };
        let parsed = Settings::parse(&original.serialize());
        assert_eq!(parsed, original);

        original.deselected.clear();
        original.last_backup_dir = None;
        original.first_run_baseline_completed = false;
        original.last_health_score = None;
        assert_eq!(Settings::parse(&original.serialize()), original);
    }

    #[test]
    fn defaults_are_safe_and_tolerate_junk() {
        let s = Settings::parse("garbage\n\n# comment\nlight = yes\n");
        assert_eq!(s.mode, Mode::Dark, "anything but `true` means dark");
        assert!(s.dry_run, "dry run must default on");
        assert!(s.deselected.is_empty());
    }

    #[test]
    fn reads_the_boolean_written_before_custom_existed() {
        let s = Settings::parse("light = true\ndry_run = false\n");
        assert_eq!(s.mode, Mode::Light, "the old boolean is still honoured");

        let s = Settings::parse("light = true\nmode = dark\n");
        assert_eq!(s.mode, Mode::Dark, "an explicit mode wins over the boolean");

        let s = Settings::parse("mode = custom\ncustom_light = true\n");
        assert_eq!(s.mode, Mode::Custom);
        assert!(s.light(), "custom reports the ramp it is built on");
    }
}
