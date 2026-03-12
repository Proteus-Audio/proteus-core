//! Simple pause/resume timer utility.

use std::time::{Duration, Instant};

/// A pauseable timer that accumulates elapsed duration.
#[derive(Debug, Clone)]
pub struct Timer {
    /// Total accumulated elapsed time, not counting any currently running interval.
    pub time: Duration,
    start_time: Option<Instant>,
}

impl Timer {
    /// Create a new timer in the stopped state.
    pub fn new() -> Self {
        Self {
            time: Duration::new(0, 0),
            start_time: None,
        }
    }

    /// Start or restart timing from the current accumulated time.
    pub fn start(&mut self) {
        self.start_time = Some(Instant::now());
    }

    /// Start timing at a specific offset.
    pub fn start_at(&mut self, time: Duration) {
        self.start_time = Some(Instant::now());
        self.time = time;
    }

    /// Resume timing if it is currently paused.
    pub fn un_pause(&mut self) {
        if self.start_time.is_none() {
            self.start_time = Some(Instant::now());
        }
    }

    /// Pause timing and accumulate elapsed duration.
    pub fn pause(&mut self) {
        if let Some(start) = self.start_time {
            self.time += start.elapsed();
            self.start_time = None;
        }
    }

    /// Stop and reset the timer.
    pub fn stop(&mut self) {
        self.reset();
    }

    /// Get the current elapsed duration.
    pub fn get_time(&self) -> Duration {
        if let Some(start) = self.start_time {
            self.time + start.elapsed()
        } else {
            self.time
        }
    }

    /// Reset the timer to zero without running.
    pub fn reset(&mut self) {
        self.start_time = None;
        self.time = Duration::new(0, 0);
    }
}

impl Default for Timer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::Timer;
    use std::time::Duration;

    #[test]
    fn timer_start_at_and_reset_work() {
        let mut timer = Timer::new();
        timer.start_at(Duration::from_millis(50));
        assert!(timer.get_time() >= Duration::from_millis(50));
        timer.reset();
        assert_eq!(timer.get_time(), Duration::from_millis(0));
    }

    #[test]
    fn timer_pause_freezes_elapsed_time() {
        let mut timer = Timer::new();
        timer.start();
        std::thread::sleep(Duration::from_millis(2));
        timer.pause();
        let paused = timer.get_time();
        std::thread::sleep(Duration::from_millis(2));
        assert_eq!(timer.get_time(), paused);
    }
}
