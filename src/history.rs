//! Recent history for the live readings.
//!
//! A gauge answers "how much right now"; it cannot answer "is this normal, or
//! did it just spike". That needs the last few minutes kept somewhere, which is
//! what this is: a fixed-size ring per series, sampled on the same tick the
//! machine snapshot refreshes on.
//!
//! Fixed capacity on purpose. A maintenance tool that grows its own memory
//! while reporting memory pressure would be reporting on itself.

use std::collections::VecDeque;
use std::time::Instant;

/// Roughly five minutes at one sample per second, which is the longest window
/// the UI offers.
const CAPACITY: usize = 320;

#[derive(Clone, Copy)]
pub struct Sample {
    pub at: Instant,
    pub value: f32,
}

/// One measured quantity over time.
pub struct Series {
    samples: VecDeque<Sample>,
}

impl Default for Series {
    fn default() -> Self {
        Self::new()
    }
}

impl Series {
    pub fn new() -> Self {
        Self {
            samples: VecDeque::with_capacity(CAPACITY),
        }
    }

    pub fn push(&mut self, value: f32) {
        if self.samples.len() == CAPACITY {
            self.samples.pop_front();
        }
        self.samples.push_back(Sample {
            at: Instant::now(),
            value,
        });
    }

    #[allow(dead_code)] // kept for the next chart that needs it
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    pub fn len(&self) -> usize {
        self.samples.len()
    }

    /// Average the most recent samples once there is enough history to be useful.
    pub fn recent_average(&self, max_samples: usize, minimum_samples: usize) -> Option<f32> {
        let count = self.samples.len().min(max_samples);
        if count < minimum_samples || count == 0 {
            return None;
        }
        let total: f32 = self.samples.iter().rev().take(count).map(|s| s.value).sum();
        Some(total / count as f32)
    }

    #[allow(dead_code)] // kept for the next chart that needs it
    pub fn latest(&self) -> Option<f32> {
        self.samples.back().map(|s| s.value)
    }

    /// The values alone, oldest first -- what a sparkline needs.
    pub fn values(&self) -> impl Iterator<Item = f32> + '_ {
        self.samples.iter().map(|s| s.value)
    }

    /// Points as (seconds ago, value), newest at x = 0 and older values to the
    /// left, which is the direction a time axis reads.
    pub fn points(&self, window_secs: f32) -> Vec<[f64; 2]> {
        let Some(now) = self.samples.back().map(|s| s.at) else {
            return Vec::new();
        };
        self.samples
            .iter()
            .filter_map(|sample| {
                let age = now.duration_since(sample.at).as_secs_f32();
                (age <= window_secs).then(|| [-(age as f64), sample.value as f64])
            })
            .collect()
    }

    /// Highest value in the series, for scaling an axis that is not a percentage.
    #[allow(dead_code)] // kept for the next chart that needs it
    pub fn peak(&self) -> f32 {
        self.samples.iter().map(|s| s.value).fold(0.0_f32, f32::max)
    }
}

/// Every series the app keeps.
#[derive(Default)]
pub struct History {
    pub cpu: Series,
    pub gpu_load: Series,
    pub ram: Series,
    pub cpu_temp: Series,
    pub gpu_temp: Series,
}

impl History {
    /// How long the charts look back. Kept here so the selector and the chart
    /// cannot disagree about what "5 min" means.
    pub const WINDOWS: [(&'static str, f32); 3] =
        [("1 min", 60.0), ("5 min", 300.0), ("15 min", 900.0)];
}
