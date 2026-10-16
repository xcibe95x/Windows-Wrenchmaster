//! The Tweaks tab: a small, reversible set of registry switches.
//!
//! Every tweak declares both the "on" and the "off" value, so the toggle can
//! put the machine back exactly where it was. Nothing is applied on startup
//! and nothing is applied in bulk without the user clicking it.

#[cfg(windows)]
use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WRITE};
#[cfg(windows)]
use winreg::{RegKey, RegValue};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Hive {
    CurrentUser,
    LocalMachine,
}

impl Hive {
    pub fn label(self) -> &'static str {
        match self {
            Hive::CurrentUser => "HKCU",
            Hive::LocalMachine => "HKLM",
        }
    }
    #[cfg(windows)]
    fn raw(self) -> winreg::HKEY {
        match self {
            Hive::CurrentUser => HKEY_CURRENT_USER,
            Hive::LocalMachine => HKEY_LOCAL_MACHINE,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Value {
    Dword(u32),
    /// The tweak is "off" when the value simply is not there.
    Absent,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum State {
    On,
    Off,
    /// Something else is in the registry: a policy, another tool, a newer
    /// Windows default. Shown as-is rather than silently overwritten.
    Other,
    Unreadable,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Category {
    General,
    Privacy,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Recommendation {
    Recommended,
    Optional,
    Careful,
}

impl Recommendation {
    pub fn label(self) -> &'static str {
        match self {
            Self::Recommended => "Recommended",
            Self::Optional => "Optional",
            Self::Careful => "Review first",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum After {
    Nothing,
    RestartExplorer,
    SignOut,
    Reboot,
}

impl After {
    pub fn note(self) -> Option<&'static str> {
        match self {
            After::Nothing => None,
            After::RestartExplorer => Some("takes effect after Explorer restarts"),
            After::SignOut => Some("takes effect after you sign out"),
            After::Reboot => Some("takes effect after a reboot"),
        }
    }
}

pub struct Tweak {
    pub name: &'static str,
    pub what: &'static str,
    pub hive: Hive,
    pub path: &'static str,
    pub value: &'static str,
    pub on: Value,
    pub off: Value,
    pub after: After,
    pub state: State,
    pub category: Category,
    pub recommendation: Recommendation,
}

impl Tweak {
    pub fn needs_admin(&self) -> bool {
        self.hive == Hive::LocalMachine
    }

    pub fn location(&self) -> String {
        format!("{}\\{}\\{}", self.hive.label(), self.path, self.value)
    }

    pub fn refresh(&mut self) {
        self.state = read_state(self);
    }

    pub fn set(&mut self, on: bool) -> Result<(), String> {
        let wanted = if on { self.on } else { self.off };
        write_value(self, wanted)?;
        self.refresh();
        Ok(())
    }
}

pub fn all() -> Vec<Tweak> {
    let mut list = vec![
        tweak(
            "Show file extensions",
            "Explorer stops hiding .exe, .scr and friends.",
            Hive::CurrentUser,
            r"Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced",
            "HideFileExt",
            Value::Dword(0),
            Value::Dword(1),
            After::RestartExplorer,
        ),
        tweak(
            "Show hidden files",
            "Hidden items become visible in Explorer.",
            Hive::CurrentUser,
            r"Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced",
            "Hidden",
            Value::Dword(1),
            Value::Dword(2),
            After::RestartExplorer,
        ),
        tweak(
            "Seconds in the taskbar clock",
            "Costs a little power, worth it.",
            Hive::CurrentUser,
            r"Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced",
            "ShowSecondsInSystemClock",
            Value::Dword(1),
            Value::Dword(0),
            After::RestartExplorer,
        ),
        tweak(
            "End Task in the taskbar menu",
            "Right-click an app on the taskbar to kill it outright.",
            Hive::CurrentUser,
            r"Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced\TaskbarDeveloperSettings",
            "TaskbarEndTask",
            Value::Dword(1),
            Value::Dword(0),
            After::RestartExplorer,
        ),
        tweak(
            "No startup delay for autostart apps",
            "Removes the deliberate ~10s stagger after sign-in.",
            Hive::CurrentUser,
            r"Software\Microsoft\Windows\CurrentVersion\Explorer\Serialize",
            "StartupDelayInMSec",
            Value::Dword(0),
            Value::Absent,
            After::SignOut,
        ),
        tweak(
            "Snappier menus",
            "Drops the menu fade delay from 400ms to 100ms.",
            Hive::CurrentUser,
            r"Control Panel\Desktop",
            "MenuShowDelay",
            Value::Dword(100),
            Value::Dword(400),
            After::SignOut,
        ),
        tweak(
            "No suggested content in Settings",
            "Turns off the tips and ads surfaced by Content Delivery Manager.",
            Hive::CurrentUser,
            r"Software\Microsoft\Windows\CurrentVersion\ContentDeliveryManager",
            "SubscribedContent-338393Enabled",
            Value::Dword(0),
            Value::Absent,
            After::Nothing,
        ),
        tweak(
            "No app suggestions in Start",
            "Stops Windows from installing promoted apps into your Start menu.",
            Hive::CurrentUser,
            r"Software\Microsoft\Windows\CurrentVersion\ContentDeliveryManager",
            "SilentInstalledAppsEnabled",
            Value::Dword(0),
            Value::Absent,
            After::Nothing,
        ),
        tweak(
            "No advertising ID",
            "Apps can no longer read a per-user ad identifier.",
            Hive::CurrentUser,
            r"Software\Microsoft\Windows\CurrentVersion\AdvertisingInfo",
            "Enabled",
            Value::Dword(0),
            Value::Absent,
            After::SignOut,
        ),
        tweak(
            "No web results in search",
            "Start menu search stays local.",
            Hive::CurrentUser,
            r"Software\Policies\Microsoft\Windows\Explorer",
            "DisableSearchBoxSuggestions",
            Value::Dword(1),
            Value::Absent,
            After::RestartExplorer,
        ),
        tweak(
            "Verbose startup and shutdown messages",
            "Windows names the service it is waiting on instead of spinning.",
            Hive::LocalMachine,
            r"SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\System",
            "VerboseStatus",
            Value::Dword(1),
            Value::Absent,
            After::Reboot,
        ),
        tweak(
            "Disable Fast Startup",
            "Makes shutdown a real shutdown. Helps dual boot and driver updates.",
            Hive::LocalMachine,
            r"SYSTEM\CurrentControlSet\Control\Session Manager\Power",
            "HiberbootEnabled",
            Value::Dword(0),
            Value::Dword(1),
            After::Reboot,
        ),
        tweak(
            "Required diagnostic data",
            "Requests Required (Basic) diagnostic data. Windows may enforce edition or organization policy; this does not disable required Windows services or updates.",
            Hive::LocalMachine,
            r"SOFTWARE\Policies\Microsoft\Windows\DataCollection",
            "AllowTelemetry",
            Value::Dword(1),
            Value::Absent,
            After::Reboot,
        ),
        tweak(
            "Turn off tailored experiences",
            "Stops Windows using diagnostic data to personalize tips and recommendations.",
            Hive::CurrentUser,
            r"Software\Policies\Microsoft\Windows\CloudContent",
            "DisableTailoredExperiencesWithDiagnosticData",
            Value::Dword(1),
            Value::Absent,
            After::Nothing,
        ),
        tweak(
            "Turn off feedback prompts",
            "Stops periodic Windows feedback notifications. It does not turn off error reporting.",
            Hive::LocalMachine,
            r"SOFTWARE\Policies\Microsoft\Windows\DataCollection",
            "DoNotShowFeedbackNotifications",
            Value::Dword(1),
            Value::Absent,
            After::Nothing,
        ),
        tweak(
            "Turn off cloud clipboard sync",
            "Keeps clipboard history and copied text from syncing to other devices. Local clipboard use still works.",
            Hive::LocalMachine,
            r"SOFTWARE\Policies\Microsoft\Windows\System",
            "AllowCrossDeviceClipboard",
            Value::Dword(0),
            Value::Absent,
            After::SignOut,
        ),
        tweak(
            "Disable NTFS last-access timestamps",
            "Fewer metadata writes on every file read.",
            Hive::LocalMachine,
            r"SYSTEM\CurrentControlSet\Control\FileSystem",
            "NtfsDisableLastAccessUpdate",
            Value::Dword(0x8000_0001),
            Value::Dword(0x8000_0000),
            After::Reboot,
        ),
        tweak(
            "Shorter shutdown service timeout",
            "Waits 5s instead of 20s for services to stop.",
            Hive::LocalMachine,
            r"SYSTEM\CurrentControlSet\Control",
            "WaitToKillServiceTimeout",
            Value::Dword(5000),
            Value::Dword(20000),
            After::Reboot,
        ),
    ];
    for t in &mut list {
        match t.name {
            "No suggested content in Settings"
            | "No app suggestions in Start"
            | "No advertising ID"
            | "Turn off tailored experiences"
            | "Turn off feedback prompts" => {
                t.category = Category::Privacy;
                t.recommendation = Recommendation::Recommended;
            }
            "No web results in search" | "Turn off cloud clipboard sync" => {
                t.category = Category::Privacy;
                t.recommendation = Recommendation::Optional;
            }
            "Required diagnostic data" => {
                t.category = Category::Privacy;
                t.recommendation = Recommendation::Careful;
            }
            _ => {}
        }
        t.refresh();
    }
    list
}

#[allow(clippy::too_many_arguments)]
fn tweak(
    name: &'static str,
    what: &'static str,
    hive: Hive,
    path: &'static str,
    value: &'static str,
    on: Value,
    off: Value,
    after: After,
) -> Tweak {
    Tweak {
        name,
        what,
        hive,
        path,
        value,
        on,
        off,
        after,
        state: State::Unreadable,
        category: Category::General,
        recommendation: Recommendation::Optional,
    }
}

/// `WaitToKillServiceTimeout` is a REG_SZ even though it holds a number; the
/// rest are DWORDs. Reading both shapes keeps the state display honest.
#[cfg(windows)]
fn read_raw(tweak: &Tweak) -> Option<Option<u32>> {
    let key = match RegKey::predef(tweak.hive.raw()).open_subkey_with_flags(tweak.path, KEY_READ) {
        Ok(key) => key,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Some(None),
        Err(_) => return None,
    };
    match key.get_raw_value(tweak.value) {
        Ok(raw) => {
            use winreg::enums::RegType;
            let expected_type = if tweak.value == "WaitToKillServiceTimeout" {
                matches!(raw.vtype, RegType::REG_SZ | RegType::REG_EXPAND_SZ)
            } else {
                raw.vtype == RegType::REG_DWORD
            };
            if expected_type {
                decode(&raw).map(Some)
            } else {
                None
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Some(None),
        Err(_) => None,
    }
}

#[cfg(windows)]
fn decode(raw: &RegValue) -> Option<u32> {
    use winreg::enums::RegType;
    match raw.vtype {
        RegType::REG_DWORD if raw.bytes.len() >= 4 => Some(u32::from_le_bytes([
            raw.bytes[0],
            raw.bytes[1],
            raw.bytes[2],
            raw.bytes[3],
        ])),
        RegType::REG_SZ | RegType::REG_EXPAND_SZ => {
            let text: String = String::from_utf16_lossy(
                &raw.bytes
                    .chunks_exact(2)
                    .map(|c| u16::from_le_bytes([c[0], c[1]]))
                    .take_while(|&u| u != 0)
                    .collect::<Vec<u16>>(),
            );
            text.trim().parse().ok()
        }
        _ => None,
    }
}

#[cfg(windows)]
fn read_state(tweak: &Tweak) -> State {
    let Some(current) = read_raw(tweak) else {
        return State::Unreadable;
    };
    let as_value = match current {
        Some(n) => Value::Dword(n),
        None => Value::Absent,
    };
    if as_value == tweak.on {
        State::On
    } else if as_value == tweak.off {
        State::Off
    } else {
        State::Other
    }
}

#[cfg(windows)]
fn write_value(tweak: &Tweak, wanted: Value) -> Result<(), String> {
    let hive = RegKey::predef(tweak.hive.raw());
    match wanted {
        Value::Dword(n) => {
            let (key, _) = hive
                .create_subkey_with_flags(tweak.path, KEY_READ | KEY_WRITE)
                .map_err(|e| describe(tweak, &e))?;
            // Keep the original type for the one string-backed timeout.
            let is_string = tweak.value == "WaitToKillServiceTimeout";
            if is_string {
                key.set_value(tweak.value, &n.to_string())
                    .map_err(|e| describe(tweak, &e))
            } else {
                key.set_value(tweak.value, &n)
                    .map_err(|e| describe(tweak, &e))
            }
        }
        Value::Absent => match hive.open_subkey_with_flags(tweak.path, KEY_READ | KEY_WRITE) {
            Ok(key) => match key.delete_value(tweak.value) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(describe(tweak, &e)),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(describe(tweak, &e)),
        },
    }
}

#[cfg(windows)]
#[cfg(windows)]
fn describe(tweak: &Tweak, err: &std::io::Error) -> String {
    if err.kind() == std::io::ErrorKind::PermissionDenied {
        format!("{} needs administrator rights.", tweak.name)
    } else {
        format!("{}: {err}", tweak.location())
    }
}

#[cfg(not(windows))]
fn read_state(_tweak: &Tweak) -> State {
    State::Unreadable
}
#[cfg(not(windows))]
fn write_value(_tweak: &Tweak, _wanted: Value) -> Result<(), String> {
    Err("Windows only".into())
}

/// Restart Explorer so the shell tweaks apply without signing out.
pub fn restart_explorer() -> std::io::Result<()> {
    crate::util::powershell_wait("Stop-Process -Name explorer -Force -ErrorAction SilentlyContinue")
        .map(|_| ())
}
