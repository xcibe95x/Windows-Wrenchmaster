//! System health, scored from what the Overview already knows, and drawn as a
//! face. The 2017 build had a `CircleProgressBar` on the splash screen that
//! counted to 100 on a timer and meant nothing; this is the same idea with an
//! actual number behind it.

use egui::{Color32, Pos2, Response, Sense, Shape, Stroke, Ui, Vec2, epaint::PathShape, vec2};

use crate::machine::{self, Machine};
use crate::theme::MUTED;
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
    /// True when a machine check reports the same condition. The Diagnose
    /// page drops these once the checks have run, or the drive is listed
    /// twice: once as a reading and once as a check.
    pub also_a_check: bool,
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
            // Through the accessor like the other two. The bare constant is
            // the dark value, so on the light theme this was the one severity
            // painting itself in a colour the rest of the page had left behind.
            Severity::Bad => crate::theme::bad(ui),
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
/// What the score reads before anything has been measured.
///
/// Neutral on purpose. Zero would accuse a machine nobody has looked at yet,
/// and 100 would clear it; fifty says what is actually true, which is that the
/// tool has not finished looking.
pub const NEUTRAL_SCORE: u8 = 50;

const DISK_LOW_FREE: f32 = 0.20;
const DISK_LOW_BYTES: u64 = 10 * 1024 * 1024 * 1024;

/// Everything the score is computed from.
///
/// A struct rather than ten positional arguments: the old eight were already
/// one `Option<f32>` away from being impossible to call correctly, and two of
/// them meant "recent average load" in different units.
pub struct Inputs<'a> {
    pub machine: &'a Machine,
    /// A disk cleanup estimate, when the cleaner has scanned.
    pub junk_bytes: Option<u64>,
    pub cpu_load_average: Option<f32>,
    pub gpu_load_average: Option<f32>,
    pub cpu_temperature_average: Option<f32>,
    pub gpu_temperature_average: Option<f32>,
    pub reliability_events: Option<usize>,
    /// Verdicts from the read-only machine checks, as far as they have run.
    pub checks: &'a [crate::diagnose::CheckOutcome],
}

/// One contributor to the score: a penalty from 0 to 100, and what it is worth.
///
/// `None` means "not measured yet", which is not the same as zero and must not
/// be scored as if it were. An unmeasured pillar is dropped and the remaining
/// weights are renormalised, so the figure always means "out of what is known"
/// rather than silently crediting the machine for checks that never ran.
struct Pillar {
    weight: f32,
    penalty: Option<f32>,
}

/// Penalties are weighted-averaged, not added.
///
/// The old score subtracted a fixed penalty per finding from 100 and then
/// clamped the result, which is why a single full-ish drive could pin the whole
/// machine at 39 no matter what the other seventeen checks said. Averaging is
/// what the commercial fleet-health tools do -- ControlUp and Lakeside both
/// publish pillar weights and combine them this way -- and it has the property
/// that matters here: one bad pillar costs its own weight and no more.
fn combine(pillars: &[Pillar]) -> Option<f32> {
    let known: Vec<&Pillar> = pillars
        .iter()
        .filter(|pillar| pillar.penalty.is_some())
        .collect();
    if known.is_empty() {
        return None;
    }
    let total_weight: f32 = known.iter().map(|pillar| pillar.weight).sum();
    let weighted: f32 = known
        .iter()
        .map(|pillar| pillar.weight * pillar.penalty.unwrap_or(0.0))
        .sum();
    Some(weighted / total_weight)
}

/// Turn a tally of findings into a penalty, with diminishing returns.
///
/// Added up flat, four warnings came to 56 and five to 70 -- so a machine with
/// a handful of small, unrelated grumbles scored like one with a genuine
/// fault, and every extra check added to the suite made every machine look
/// worse. That is the arithmetic punishing breadth, which is backwards: a
/// suite that looks at more things should be more trustworthy, not harsher.
///
/// Each finding multiplies the *remaining* health instead, so the first one
/// costs the most and the twentieth almost nothing, and the total can approach
/// 100 without ever reaching it on warnings alone. A problem is worth far more
/// than a warning because the checks only call something a problem when it is
/// already affecting the machine.
fn findings_penalty(problems: u32, warnings: u32, unreadable: u32) -> f32 {
    let remaining = 0.58_f32.powi(problems as i32)
        * 0.88_f32.powi(warnings as i32)
        // A check that could not read is a hole in the diagnosis, not a fault
        // in the machine: it counts, barely, because an unknown is not a pass.
        * 0.97_f32.powi(unreadable as i32);
    ((1.0 - remaining) * 100.0).clamp(0.0, 100.0)
}

/// A penalty that stays at zero until `start`, then climbs to 100 at `full`.
///
/// Squared rather than linear, so the first percent past a threshold is nearly
/// free and the last one hurts. A machine at 86% memory is not in the same
/// trouble as one at 99%, and a linear ramp scores them two points apart.
fn ramp(value: f32, start: f32, full: f32) -> f32 {
    if value <= start {
        return 0.0;
    }
    if value >= full {
        return 100.0;
    }
    let progress = (value - start) / (full - start);
    (progress * progress * 100.0).clamp(0.0, 100.0)
}

impl Health {
    /// Score the machine from everything currently known about it.
    pub fn assess(input: Inputs<'_>) -> Self {
        let Inputs {
            machine,
            junk_bytes,
            cpu_load_average,
            gpu_load_average,
            cpu_temperature_average,
            gpu_temperature_average,
            reliability_events,
            checks,
        } = input;
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
            // The same bands as the storage check, so one condition cannot
            // be amber here and red there.
            if free < DISK_CRITICAL_FREE || disk.available < DISK_CRITICAL_BYTES {
                notes.push(Note {
                    text: label,
                    severity: Severity::Bad,
                    also_a_check: true,
                    recommendation: "The Windows drive is out of room. Free space now: with this little left, Windows cannot stage an update or grow the page file.",
                });
            } else if free < DISK_LOW_FREE || disk.available < DISK_LOW_BYTES {
                notes.push(Note {
                    text: label,
                    severity: Severity::Warn,
                    also_a_check: true,
                    recommendation: "Free space on the Windows drive: past 90% full a solid-state drive has little room to manage its own writes and sustained writes slow down.",
                });
            }
        }

        if let Some(count) = reliability_events {
            if count >= 8 {
                notes.push(Note {
                    text: format!("{count} recent Windows reliability events need attention"),
                    severity: Severity::Bad,
                    also_a_check: true,
                    recommendation: "Review the event details below and match their times to the slowdowns or crashes you noticed.",
                });
            } else if count >= 3 {
                notes.push(Note {
                    text: format!("{count} recent Windows reliability events were found"),
                    severity: Severity::Warn,
                    also_a_check: true,
                    recommendation: "Review repeated event sources below and correlate their timestamps with symptoms.",
                });
            }
        }

        // Sustained load is a better slowdown signal than one busy sample.
        if let Some(load) = cpu_load_average {
            if load >= 95.0 {
                notes.push(Note {
                    text: format!(
                        "CPU load averaged {load:.0}% recently; active work may feel slow"
                    ),
                    severity: Severity::Warn,
                    also_a_check: false,
                    recommendation: "Use the largest CPU users list below; close or update the app keeping the CPU busy.",
                });
            } else if load >= 90.0 {
                notes.push(Note {
                    text: format!("CPU load averaged {load:.0}% recently; the system may feel sluggish"),
                    severity: Severity::Warn,
                    also_a_check: false,
                    recommendation: "Check the largest CPU users below and let updates or scans finish if expected.",
                });
            } else if load >= 85.0 {
                notes.push(Note {
                    text: format!("CPU load averaged {load:.0}% recently"),
                    severity: Severity::Info,
                    also_a_check: false,
                    recommendation: "Check whether the load is expected; close unnecessary work if the PC feels slow.",
                });
            }
        }
        if let Some(load) = gpu_load_average {
            if load >= 95.0 {
                notes.push(Note {
                    text: format!(
                        "GPU load averaged {load:.0}% recently; graphics tasks may feel slow"
                    ),
                    severity: Severity::Warn,
                    also_a_check: false,
                    recommendation: "Check the app using graphics and the GPU temperatures on the Hardware page.",
                });
            } else if load >= 85.0 {
                notes.push(Note {
                    text: format!("GPU load averaged {load:.0}% recently"),
                    severity: Severity::Info,
                    also_a_check: false,
                    recommendation: "Check whether a game, render, or video task is expected to be using the GPU.",
                });
            }
        }
        // Memory pressure right now.
        let ram = machine.ram_fraction();
        if ram > 0.95 {
            notes.push(Note {
                also_a_check: false,
                text: format!("Memory is {:.0}% used — {}", ram * 100.0, machine.ram_summary()),
                severity: Severity::Bad,
                recommendation: "Close the largest memory users below; save work before closing apps.",
            });
        } else if ram > 0.90 {
            notes.push(Note {
                also_a_check: false,
                text: format!(
                    "Memory is {:.0}% used — {}",
                    ram * 100.0,
                    machine.ram_summary()
                ),
                severity: Severity::Bad,
                recommendation: "Close the largest memory users below; save work before closing apps.",
            });
        } else if ram > 0.85 {
            notes.push(Note {
                also_a_check: false,
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
                    notes.push(Note {
                        text: format!("{name} temperature averaged {temp:.0} °C recently"),
                        severity: Severity::Bad,
                        also_a_check: false,
                    recommendation: "Stop demanding work and check cooling, fan operation, and airflow.",
                    });
                } else if temp >= threshold {
                    notes.push(Note {
                        text: format!("{name} temperature averaged {temp:.0} °C recently"),
                        severity: Severity::Warn,
                        also_a_check: false,
                    recommendation: "Check cooling and airflow; reduce demanding work if temperature stays high.",
                    });
                }
            }
        }

        // Uptime: past a few weeks, updates are waiting and drivers have leaked.
        if machine.uptime_days() >= 21 {
            notes.push(Note {
                also_a_check: false,
                text: format!(
                    "Up for {} days — a reboot would apply pending updates",
                    machine.uptime_days()
                ),
                severity: Severity::Warn,
                recommendation: "Restart when convenient, especially if Windows has pending updates.",
            });
        } else if machine.uptime_days() >= 7 {
            notes.push(Note {
                also_a_check: false,
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
                    also_a_check: false,
                    recommendation: "Review Cleanup's preview and remove only files you recognize.",
                });
            }
            Some(bytes) if bytes >= 2 * 1024 * 1024 * 1024 => {
                notes.push(Note {
                    text: format!("{} of reclaimable junk found", human_bytes(bytes)),
                    severity: Severity::Info,
                    also_a_check: false,
                    recommendation: "Review Cleanup's preview if you want to reclaim this space.",
                });
            }
            Some(_) => notes.push(Note {
                also_a_check: false,
                text: "Caches are clean".into(),
                severity: Severity::Info,
                recommendation: "No cleanup action is needed.",
            }),
            None => {}
        }

        // An OS the tool was never tested against is worth saying out loud.
        if !machine.era.supported() {
            notes.push(Note {
                also_a_check: false,
                text: format!("{} is outside the tested range", machine.era.label()),
                severity: Severity::Warn,
                recommendation: "Check Windows support status and install available security updates.",
            });
        }

        // --- the pillars ---------------------------------------------------
        //
        // Weights say what each kind of trouble is worth relative to the rest.
        // The checks carry the most because there are eighteen of them and
        // they are the only pillar that looks at the machine rather than at
        // this minute's readings; load and heat carry the least because they
        // describe whatever happens to be running right now, which the user
        // may have started on purpose.
        let system_disk = machine
            .disks
            .iter()
            .find(|disk| machine::is_system_drive_mount(&disk.mount));

        // Two separate problems, scored separately and the worse one taken.
        // The previous rule required a drive to be short by *both* measures
        // before it counted, which let them cancel: 136 GB free is plenty of
        // room to work, so a system SSD sitting at 93% full scored nothing at
        // all. Both of these are true at once and only one of them is about
        // gigabytes.
        let storage_penalty = system_disk.map(|disk| {
            let free_fraction = (1.0 - disk.used_fraction()).clamp(0.0, 1.0);
            let free_bytes = disk.available;

            // Room to work: can Windows stage an update, grow the page file,
            // write a restore point? That question is answered in gigabytes
            // and a big drive answers it easily.
            let capacity_penalty = if free_bytes < DISK_CRITICAL_BYTES {
                100.0
            } else {
                ramp(
                    DISK_LOW_BYTES.saturating_sub(free_bytes) as f32,
                    0.0,
                    (DISK_LOW_BYTES - DISK_CRITICAL_BYTES) as f32,
                )
            };

            // Headroom: a solid-state drive needs free *proportion*, not free
            // gigabytes. Below roughly a tenth free, garbage collection runs
            // out of room to work with, write amplification climbs and
            // sustained writes slow down -- on a 4 TB drive just as much as on
            // a 128 GB one. Windows draws the same line itself: Explorer turns
            // the capacity bar red at 90% used.
            let headroom_penalty = if free_fraction >= DISK_LOW_FREE {
                0.0
            } else if free_fraction >= DISK_CRITICAL_FREE {
                (DISK_LOW_FREE - free_fraction) / (DISK_LOW_FREE - DISK_CRITICAL_FREE) * 35.0
            } else {
                // Linear from here, not squared: past the point Windows calls
                // low, every further percent is a real loss of headroom and
                // the curve should not flatter it.
                35.0 + ((DISK_CRITICAL_FREE - free_fraction) / 0.08 * 65.0).clamp(0.0, 65.0)
            };

            capacity_penalty.max(headroom_penalty)
        });

        let memory_penalty = Some(ramp(machine.ram_fraction(), 0.85, 1.0));
        let load_penalty = cpu_load_average.map(|cpu| {
            let cpu_penalty = ramp(cpu, 80.0, 100.0);
            // The GPU counts for less: a pegged GPU is usually a game or a
            // render the user started and is watching happen.
            let gpu_penalty = gpu_load_average.map_or(0.0, |gpu| ramp(gpu, 90.0, 100.0) * 0.6);
            cpu_penalty.max(gpu_penalty)
        });
        let thermal_penalty = match (cpu_temperature_average, gpu_temperature_average) {
            (None, None) => None,
            (cpu, gpu) => {
                Some(ramp(cpu.unwrap_or(0.0), 80.0, 95.0).max(ramp(gpu.unwrap_or(0.0), 80.0, 95.0)))
            }
        };
        let stability_penalty = reliability_events.map(|count| match count {
            0 => 0.0,
            1..=2 => 12.0,
            3..=7 => 45.0,
            _ => 90.0,
        });

        // The eighteen read-only checks, as far as they have run. Scored per
        // finding rather than per check so that a partial sweep is still worth
        // something: three problems out of five checks is already bad news,
        // and waiting for all eighteen before saying so helps nobody.
        let checks_penalty = (!checks.is_empty()).then(|| {
            let mut problems = 0u32;
            let mut warnings = 0u32;
            let mut unreadable = 0u32;
            for outcome in checks {
                match outcome.status {
                    crate::diagnose::CheckStatus::Problem => problems += 1,
                    crate::diagnose::CheckStatus::Warning => warnings += 1,
                    crate::diagnose::CheckStatus::Error => unreadable += 1,
                    // Needing an administrator is this app's situation, not a
                    // fault in the machine. It is reported in full on the
                    // check list, and it costs the score nothing: marking a
                    // PC down because the tool was launched without a token
                    // would be scoring ourselves.
                    crate::diagnose::CheckStatus::NeedsAdmin
                    | crate::diagnose::CheckStatus::Pass
                    | crate::diagnose::CheckStatus::Skipped => {}
                }
            }
            findings_penalty(problems, warnings, unreadable)
        });

        let pillars = [
            Pillar {
                weight: 30.0,
                penalty: checks_penalty,
            },
            Pillar {
                weight: 20.0,
                penalty: stability_penalty,
            },
            Pillar {
                weight: 15.0,
                penalty: storage_penalty,
            },
            Pillar {
                weight: 15.0,
                penalty: memory_penalty,
            },
            Pillar {
                weight: 10.0,
                penalty: load_penalty,
            },
            Pillar {
                weight: 10.0,
                penalty: thermal_penalty,
            },
        ];

        // Nothing measured yet: the honest answer is a flat 50, not a perfect
        // 100. The first version started every machine at 100 and took marks
        // off as the checks landed, so the app opened on a clean bill of
        // health it had no basis for and then quietly got worse while you
        // watched it.
        let measured = combine(&pillars);
        let still_looking = checks_penalty.is_none() || reliability_events.is_none();
        let score = match measured {
            None => NEUTRAL_SCORE,
            Some(penalty) => (100.0 - penalty).round().clamp(0.0, 100.0) as u8,
        };

        Self {
            score,
            mood: if measured.is_none() || (still_looking && score >= 65) {
                // A high score from half the evidence is not yet a verdict.
                // A low one is: something already found a fault, and waiting
                // for the rest of the sweep to say so would be hiding it.
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

    #[test]
    fn nothing_measured_reads_neutral() {
        // Every pillar unknown. Not 100, which would clear a machine nobody
        // has looked at, and not 0, which would accuse it.
        assert_eq!(combine(&[]), None);
        assert_eq!(
            combine(&[Pillar {
                weight: 30.0,
                penalty: None
            }]),
            None
        );
    }

    #[test]
    fn one_bad_pillar_costs_only_its_own_weight() {
        // The regression this scoring exists for: a full Windows drive used
        // to clamp the whole score to 39 regardless of everything else. At
        // 15% of the total, a maxed-out storage penalty can cost 15 points
        // and no more.
        let penalty = combine(&[
            Pillar {
                weight: 15.0,
                penalty: Some(100.0),
            },
            Pillar {
                weight: 30.0,
                penalty: Some(0.0),
            },
            Pillar {
                weight: 20.0,
                penalty: Some(0.0),
            },
            Pillar {
                weight: 15.0,
                penalty: Some(0.0),
            },
            Pillar {
                weight: 10.0,
                penalty: Some(0.0),
            },
            Pillar {
                weight: 10.0,
                penalty: Some(0.0),
            },
        ])
        .expect("pillars are known");
        assert_eq!((100.0 - penalty).round() as u8, 85);
    }

    #[test]
    fn unknown_pillars_are_dropped_not_counted_as_healthy() {
        // Two pillars known, one of them at fault: the score reflects the
        // half that was measured rather than being diluted by the half that
        // was not.
        let penalty = combine(&[
            Pillar {
                weight: 30.0,
                penalty: Some(50.0),
            },
            Pillar {
                weight: 10.0,
                penalty: Some(0.0),
            },
            Pillar {
                weight: 60.0,
                penalty: None,
            },
        ])
        .expect("two pillars are known");
        // 30*50 / (30+10) = 37.5
        assert!((penalty - 37.5).abs() < 0.01, "{penalty}");
    }

    #[test]
    fn findings_have_diminishing_returns() {
        // One warning is cheap, four are not four times as bad, and no number
        // of warnings alone reaches a full penalty.
        let one = findings_penalty(0, 1, 0);
        let four = findings_penalty(0, 4, 0);
        let twenty = findings_penalty(0, 20, 0);
        assert!((one - 12.0).abs() < 1.0, "{one}");
        assert!(four < one * 4.0, "{four} vs {one}");
        assert!(twenty < 100.0, "{twenty}");
        // A confirmed problem outweighs several warnings, which is the point
        // of the two words meaning different things.
        assert!(findings_penalty(1, 0, 0) > findings_penalty(0, 3, 0));
        // Needing an administrator is not passed in at all; a clean sweep is
        // a clean sweep.
        assert_eq!(findings_penalty(0, 0, 0), 0.0);
    }

    #[test]
    fn the_ramp_is_gentle_at_the_start_and_steep_at_the_end() {
        assert_eq!(ramp(0.80, 0.85, 1.0), 0.0);
        assert_eq!(ramp(1.00, 0.85, 1.0), 100.0);
        // A third of the way in costs about a ninth of the penalty, which is
        // the whole point of squaring it.
        let third = ramp(0.90, 0.85, 1.0);
        assert!((third - 11.1).abs() < 0.5, "{third}");
    }
}
