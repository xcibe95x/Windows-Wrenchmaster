//! System health, scored from what the Overview already knows, and drawn as a
//! face. The 2017 build had a `CircleProgressBar` on the splash screen that
//! counted to 100 on a timer and meant nothing; this is the same idea with an
//! actual number behind it.

use egui::{Color32, Pos2, Response, Sense, Shape, Stroke, Ui, Vec2, epaint::PathShape, vec2};

use crate::machine::{self, Machine};
use crate::theme::{BAD, MUTED};
use crate::util::human_bytes;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mood {
    Checking,
    Happy,
    Content,
    Meh,
    Sad,
}

impl Mood {
    pub fn from_score(score: u8) -> Self {
        match score {
            85..=100 => Mood::Happy,
            65..=84 => Mood::Content,
            40..=64 => Mood::Meh,
            _ => Mood::Sad,
        }
    }

    pub fn headline(self) -> &'static str {
        match self {
            Mood::Checking => "Checking system performance",
            Mood::Happy => "No major issues detected",
            Mood::Content => "Mostly fine, a couple of things to look at",
            Mood::Meh => "Needs attention",
            Mood::Sad => "Something here needs fixing now",
        }
    }

    pub fn color(self, ui: &egui::Ui) -> Color32 {
        match self {
            Mood::Checking => MUTED,
            Mood::Happy | Mood::Content => crate::theme::good(ui),
            Mood::Meh => crate::theme::warn(ui),
            Mood::Sad => crate::theme::bad(ui),
        }
    }

    /// How much the mouth curves: positive smiles, negative frowns.
    fn curve(self) -> f32 {
        match self {
            Mood::Checking => 0.0,
            Mood::Happy => 1.0,
            Mood::Content => 0.45,
            Mood::Meh => 0.0,
            Mood::Sad => -0.7,
        }
    }
}

/// One thing worth telling the user about, with the severity that earned it.
pub struct Note {
    pub text: String,
    pub severity: Severity,
    pub recommendation: &'static str,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Info,
    Warn,
    Bad,
}

impl Severity {
    pub fn color(self, ui: &egui::Ui) -> Color32 {
        match self {
            Severity::Info => crate::theme::good(ui),
            Severity::Warn => crate::theme::warn(ui),
            Severity::Bad => BAD,
        }
    }
}

pub struct Health {
    pub score: u8,
    pub mood: Mood,
    pub notes: Vec<Note>,
}

/// The point at which the Windows volume is hurting the machine rather than
/// merely getting full: under a tenth of the drive free, or too little room
/// for an update to stage itself whatever the proportion.
const DISK_CRITICAL_FREE: f32 = 0.10;
const DISK_CRITICAL_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// Worth mentioning before it becomes a problem. Twenty percent is the
/// headroom a solid-state drive wants in order to keep writing at full speed.
const DISK_LOW_FREE: f32 = 0.20;
const DISK_LOW_BYTES: u64 = 10 * 1024 * 1024 * 1024;

impl Health {
    /// Cleaner junk is a disk cleanup estimate; CPU and GPU are recent load averages.
    pub fn assess(
        machine: &Machine,
        junk_bytes: Option<u64>,
        cpu_load_average: Option<f32>,
        gpu_load_average: Option<f32>,
        cpu_temperature_average: Option<f32>,
        gpu_temperature_average: Option<f32>,
        cpu_samples: usize,
        reliability_events: Option<usize>,
    ) -> Self {
        let mut penalty: i32 = 0;
        let mut notes: Vec<Note> = Vec::new();

        // Only free space on the Windows installation volume affects system health.
        if let Some(disk) = machine
            .disks
            .iter()
            .find(|disk| machine::is_system_drive_mount(&disk.mount))
        {
            let used = disk.used_fraction();
            let free = 1.0 - used;
            let label = format!(
                "Windows drive {} is {:.0}% full ({} free)",
                disk.mount.trim_end_matches('\\'),
                used * 100.0,
                human_bytes(disk.available)
            );
            if free < DISK_CRITICAL_FREE || disk.available < DISK_CRITICAL_BYTES {
                penalty += 35;
                notes.push(Note {
                    text: label,
                    severity: Severity::Bad,
                    recommendation: "Free space on the Windows drive: past 90% full a solid-state drive loses write speed and the whole machine slows down.",
                });
            } else if free < DISK_LOW_FREE || disk.available < DISK_LOW_BYTES {
                penalty += 15;
                notes.push(Note {
                    text: label,
                    severity: Severity::Warn,
                    recommendation: "Free space on the Windows drive; avoid deleting system folders manually.",
                });
            }
        }

        if let Some(count) = reliability_events {
            if count >= 8 {
                penalty += 30;
                notes.push(Note {
                    text: format!("{count} recent Windows reliability events need attention"),
                    severity: Severity::Bad,
                    recommendation: "Review the event details below and match their times to the slowdowns or crashes you noticed.",
                });
            } else if count >= 3 {
                penalty += 15;
                notes.push(Note {
                    text: format!("{count} recent Windows reliability events were found"),
                    severity: Severity::Warn,
                    recommendation: "Review repeated event sources below and correlate their timestamps with symptoms.",
                });
            }
        } else {
            notes.push(Note {
                text: "Windows reliability events have not been checked yet".into(),
                severity: Severity::Info,
                recommendation: "Wait for the background check or run the full benchmark to check recent Windows events.",
            });
        }

        // Sustained load is a better slowdown signal than one busy sample.
        if let Some(load) = cpu_load_average {
            if load >= 95.0 {
                penalty += 35;
                notes.push(Note {
                    text: format!(
                        "CPU load averaged {load:.0}% recently; active work may feel slow"
                    ),
                    severity: Severity::Warn,
                    recommendation: "Use the largest CPU users list below; close or update the app keeping the CPU busy.",
                });
            } else if load >= 90.0 {
                penalty += 22;
                notes.push(Note {
                    text: format!("CPU load averaged {load:.0}% recently; the system may feel sluggish"),
                    severity: Severity::Warn,
                    recommendation: "Check the largest CPU users below and let updates or scans finish if expected.",
                });
            } else if load >= 85.0 {
                penalty += 10;
                notes.push(Note {
                    text: format!("CPU load averaged {load:.0}% recently"),
                    severity: Severity::Info,
                    recommendation: "Check whether the load is expected; close unnecessary work if the PC feels slow.",
                });
            }
        }
        if let Some(load) = gpu_load_average {
            if load >= 95.0 {
                penalty += 20;
                notes.push(Note {
                    text: format!(
                        "GPU load averaged {load:.0}% recently; graphics tasks may feel slow"
                    ),
                    severity: Severity::Warn,
                    recommendation: "Check the app using graphics and the GPU temperatures on the Hardware page.",
                });
            } else if load >= 85.0 {
                penalty += 8;
                notes.push(Note {
                    text: format!("GPU load averaged {load:.0}% recently"),
                    severity: Severity::Info,
                    recommendation: "Check whether a game, render, or video task is expected to be using the GPU.",
                });
            }
        }
        // Memory pressure right now.
        let ram = machine.ram_fraction();
        if ram > 0.95 {
            penalty += 35;
            notes.push(Note {
                text: format!("Memory is {:.0}% used — {}", ram * 100.0, machine.ram_summary()),
                severity: Severity::Bad,
                recommendation: "Close the largest memory users below; save work before closing apps.",
            });
        } else if ram > 0.90 {
            penalty += 22;
            notes.push(Note {
                text: format!(
                    "Memory is {:.0}% used — {}",
                    ram * 100.0,
                    machine.ram_summary()
                ),
                severity: Severity::Bad,
                recommendation: "Close the largest memory users below; save work before closing apps.",
            });
        } else if ram > 0.85 {
            penalty += 10;
            notes.push(Note {
                text: format!("Memory is {:.0}% used", ram * 100.0),
                severity: Severity::Warn,
                recommendation: "Review the largest memory users and close apps you are not using.",
            });
        }

        for (name, temperature, threshold, critical) in [
            ("CPU", cpu_temperature_average, 85.0, 95.0),
            ("GPU", gpu_temperature_average, 85.0, 95.0),
        ] {
            if let Some(temp) = temperature {
                if temp >= critical {
                    penalty += 30;
                    notes.push(Note {
                        text: format!("{name} temperature averaged {temp:.0} °C recently"),
                        severity: Severity::Bad,
                        recommendation: "Stop demanding work and check cooling, fan operation, and airflow.",
                    });
                } else if temp >= threshold {
                    penalty += 16;
                    notes.push(Note {
                        text: format!("{name} temperature averaged {temp:.0} °C recently"),
                        severity: Severity::Warn,
                        recommendation: "Check cooling and airflow; reduce demanding work if temperature stays high.",
                    });
                }
            }
        }

        // Uptime: past a few weeks, updates are waiting and drivers have leaked.
        if machine.uptime_days() >= 21 {
            notes.push(Note {
                text: format!(
                    "Up for {} days — a reboot would apply pending updates",
                    machine.uptime_days()
                ),
                severity: Severity::Warn,
                recommendation: "Restart when convenient, especially if Windows has pending updates.",
            });
        } else if machine.uptime_days() >= 7 {
            notes.push(Note {
                text: format!("Up for {} days", machine.uptime_days()),
                severity: Severity::Info,
                recommendation: "Consider restarting after saving your work if updates are pending.",
            });
        }

        // Cleanup findings are optional. An unmeasured scan is not a health issue.
        match junk_bytes {
            Some(bytes) if bytes >= 20 * 1024 * 1024 * 1024 => {
                notes.push(Note {
                    text: format!("{} of reclaimable junk found", human_bytes(bytes)),
                    severity: Severity::Warn,
                    recommendation: "Review Cleanup's preview and remove only files you recognize.",
                });
            }
            Some(bytes) if bytes >= 2 * 1024 * 1024 * 1024 => {
                notes.push(Note {
                    text: format!("{} of reclaimable junk found", human_bytes(bytes)),
                    severity: Severity::Info,
                    recommendation: "Review Cleanup's preview if you want to reclaim this space.",
                });
            }
            Some(_) => notes.push(Note {
                text: "Caches are clean".into(),
                severity: Severity::Info,
                recommendation: "No cleanup action is needed.",
            }),
            None => {}
        }

        // An OS the tool was never tested against is worth saying out loud.
        if !machine.era.supported() {
            notes.push(Note {
                text: format!("{} is outside the tested range", machine.era.label()),
                severity: Severity::Warn,
                recommendation: "Check Windows support status and install available security updates.",
            });
        }

        if cpu_samples < 30 {
            notes.push(Note {
                text: format!("Collecting a 30-second performance baseline ({cpu_samples}/30 samples)"),
                severity: Severity::Info,
                recommendation: "Leave Wrenchmaster open briefly; the baseline is passive and does not stress the PC.",
            });
        }

        let mut score = (100 - penalty).clamp(0, 100) as u8;
        // High observed pressure must never still read as a healthy 85+ score.
        if cpu_load_average.is_some_and(|load| load >= 90.0)
            || gpu_load_average.is_some_and(|load| load >= 95.0)
            || ram > 0.90
            || cpu_temperature_average.is_some_and(|temp| temp >= 85.0)
            || gpu_temperature_average.is_some_and(|temp| temp >= 85.0)
        {
            score = score.min(64);
        }
        // The same two bands the notes use. They were 2% and 12% here
        // against 5% and 12% above, so a drive could be called out in red on
        // the list while the score stayed in the eighties and the face kept
        // smiling. One pair of constants now, so the headline figure and the
        // reason given for it cannot disagree.
        if machine.disks.iter().any(|disk| {
            machine::is_system_drive_mount(&disk.mount)
                && (disk.available < DISK_CRITICAL_BYTES
                    || (1.0 - disk.used_fraction()) < DISK_CRITICAL_FREE)
        }) {
            score = score.min(39);
        } else if machine.disks.iter().any(|disk| {
            machine::is_system_drive_mount(&disk.mount)
                && (disk.available < DISK_LOW_BYTES || (1.0 - disk.used_fraction()) < DISK_LOW_FREE)
        }) {
            score = score.min(84);
        }
        Self {
            score,
            mood: if (cpu_samples < 30 || reliability_events.is_none()) && score >= 65 {
                Mood::Checking
            } else {
                Mood::from_score(score)
            },
            notes,
        }
    }
}

/// Draw the face. Painted rather than typed, because the monochrome emoji in
/// the default font cannot change expression and look like clip art.
pub fn face(ui: &mut Ui, mood: Mood, diameter: f32) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(diameter), Sense::hover());
    if !ui.is_rect_visible(rect) {
        return response;
    }

    let color = mood.color(ui);
    let painter = ui.painter();
    let center = rect.center();
    let radius = diameter * 0.5 - 2.0;

    painter.circle_filled(center, radius, color.gamma_multiply(0.12));
    painter.circle_stroke(center, radius, Stroke::new(2.0, color));

    // Eyes.
    let eye_offset = vec2(radius * 0.36, radius * 0.28);
    let eye_radius = (radius * 0.1).max(1.5);
    painter.circle_filled(center - eye_offset, eye_radius, color);
    painter.circle_filled(
        center + vec2(eye_offset.x, -eye_offset.y),
        eye_radius,
        color,
    );

    // Mouth: a quadratic curve sampled into a polyline, so one code path
    // covers smiling, flat and frowning.
    let curve = mood.curve();
    let half_width = radius * 0.46;
    let base_y = center.y + radius * 0.28;
    let lift = radius * 0.34 * curve;
    let points: Vec<Pos2> = (0..=16)
        .map(|i| {
            let t = i as f32 / 16.0;
            let x = center.x - half_width + 2.0 * half_width * t;
            // Parabola through the two corners, peaking in the middle.
            let y = base_y + lift * (1.0 - (2.0 * t - 1.0).powi(2));
            Pos2::new(x, y)
        })
        .collect();
    painter.add(Shape::Path(PathShape::line(
        points,
        Stroke::new(2.2, color),
    )));

    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moods_follow_the_score() {
        assert_eq!(Mood::from_score(100), Mood::Happy);
        assert_eq!(Mood::from_score(85), Mood::Happy);
        assert_eq!(Mood::from_score(84), Mood::Content);
        assert_eq!(Mood::from_score(64), Mood::Meh);
        assert_eq!(Mood::from_score(39), Mood::Sad);
        assert_eq!(Mood::from_score(0), Mood::Sad);
    }
}
