//! System health, scored from what the Overview already knows, and drawn as a
//! face. The 2017 build had a `CircleProgressBar` on the splash screen that
//! counted to 100 on a timer and meant nothing; this is the same idea with an
//! actual number behind it.

use egui::{Color32, Pos2, Response, Sense, Shape, Stroke, Ui, Vec2, epaint::PathShape, vec2};

use crate::machine::Machine;
use crate::theme::BAD;
use crate::util::human_bytes;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mood {
    Happy,
    Content,
    Meh,
    Sad,
}

impl Mood {
    fn from_score(score: u8) -> Self {
        match score {
            85..=100 => Mood::Happy,
            65..=84 => Mood::Content,
            40..=64 => Mood::Meh,
            _ => Mood::Sad,
        }
    }

    pub fn headline(self) -> &'static str {
        match self {
            Mood::Happy => "This machine is healthy",
            Mood::Content => "Mostly fine, a couple of things to look at",
            Mood::Meh => "Needs attention",
            Mood::Sad => "Something here needs fixing now",
        }
    }

    pub fn color(self, ui: &egui::Ui) -> Color32 {
        match self {
            Mood::Happy | Mood::Content => crate::theme::good(ui),
            Mood::Meh => crate::theme::warn(ui),
            Mood::Sad => crate::theme::bad(ui),
        }
    }

    /// How much the mouth curves: positive smiles, negative frowns.
    fn curve(self) -> f32 {
        match self {
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

impl Health {
    /// `junk` is the cleaner's last scan total, if the user has run one.
    pub fn assess(machine: &Machine, junk_bytes: Option<u64>) -> Self {
        let mut penalty: i32 = 0;
        let mut notes: Vec<Note> = Vec::new();

        // Disks: the system drive matters most, but a full data volume still
        // breaks things, so every mounted volume is checked.
        for disk in &machine.disks {
            let free = 1.0 - disk.used_fraction();
            let label = format!(
                "{} is {:.0}% full ({} free)",
                disk.mount.trim_end_matches('\\'),
                disk.used_fraction() * 100.0,
                human_bytes(disk.available)
            );
            if free < 0.05 {
                penalty += 35;
                notes.push(Note {
                    text: label,
                    severity: Severity::Bad,
                });
            } else if free < 0.12 {
                penalty += 15;
                notes.push(Note {
                    text: label,
                    severity: Severity::Warn,
                });
            }
        }

        // Memory pressure right now.
        let ram = machine.ram_fraction();
        if ram > 0.92 {
            penalty += 20;
            notes.push(Note {
                text: format!(
                    "Memory is {:.0}% used — {}",
                    ram * 100.0,
                    machine.ram_summary()
                ),
                severity: Severity::Bad,
            });
        } else if ram > 0.85 {
            penalty += 8;
            notes.push(Note {
                text: format!("Memory is {:.0}% used", ram * 100.0),
                severity: Severity::Warn,
            });
        }

        // Uptime: past a few weeks, updates are waiting and drivers have leaked.
        if machine.uptime_days() >= 21 {
            penalty += 12;
            notes.push(Note {
                text: format!(
                    "Up for {} days — a reboot would apply pending updates",
                    machine.uptime_days()
                ),
                severity: Severity::Warn,
            });
        } else if machine.uptime_days() >= 7 {
            penalty += 4;
            notes.push(Note {
                text: format!("Up for {} days", machine.uptime_days()),
                severity: Severity::Info,
            });
        }

        // Junk, but only once it has actually been measured.
        match junk_bytes {
            Some(bytes) if bytes >= 20 * 1024 * 1024 * 1024 => {
                penalty += 14;
                notes.push(Note {
                    text: format!("{} of reclaimable junk found", human_bytes(bytes)),
                    severity: Severity::Warn,
                });
            }
            Some(bytes) if bytes >= 2 * 1024 * 1024 * 1024 => {
                penalty += 5;
                notes.push(Note {
                    text: format!("{} of reclaimable junk found", human_bytes(bytes)),
                    severity: Severity::Info,
                });
            }
            Some(_) => notes.push(Note {
                text: "Caches are clean".into(),
                severity: Severity::Info,
            }),
            None => notes.push(Note {
                text: "Junk not measured yet — run a scan in Storage > Cleanup".into(),
                severity: Severity::Info,
            }),
        }

        // An OS the tool was never tested against is worth saying out loud.
        if !machine.era.supported() {
            penalty += 10;
            notes.push(Note {
                text: format!("{} is outside the tested range", machine.era.label()),
                severity: Severity::Warn,
            });
        }

        let score = (100 - penalty).clamp(0, 100) as u8;
        Self {
            score,
            mood: Mood::from_score(score),
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
            let y = base_y - lift * (1.0 - (2.0 * t - 1.0).powi(2));
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
