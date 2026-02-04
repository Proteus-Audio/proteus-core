use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct Timer {
    pub time: Duration,
    start_time: Option<Instant>,
}

impl Timer {
    pub fn new() -> Self {
        Self {
            time: Duration::new(0, 0),
            start_time: None,
        }
    }

    pub fn start(&mut self) {
        self.start_time = Some(Instant::now());
    }

    pub fn start_at(&mut self, time: Duration) {
        self.start_time = Some(Instant::now());
        self.time = time;
    }

    pub fn un_pause(&mut self) {
        if self.start_time == None {
            self.start_time = Some(Instant::now());
        }
    }

    pub fn pause(&mut self) {
        if let Some(start) = self.start_time {
            self.time += start.elapsed();
            self.start_time = None;
        }
    }

    pub fn stop(&mut self) {
        self.reset();
    }

    pub fn get_time(&self) -> Duration {
        if let Some(start) = self.start_time {
            self.time + start.elapsed()
        } else {
            self.time
        }
    }

    pub fn reset(&mut self) {
        self.start_time = None;
        self.time = Duration::new(0, 0);
    }
}