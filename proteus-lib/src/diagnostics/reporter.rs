//! Periodic playback state reporter for UI updates.

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use crate::playback::player::Player;

/// Snapshot of playback state sent to UI consumers.
#[derive(Clone, PartialEq)]
pub struct Report {
    pub time: f64,
    pub volume: f32,
    pub duration: f64,
    pub playing: bool,
}

/// Background reporter that polls the [`Player`] at fixed intervals.
#[derive(Clone)]
pub struct Reporter {
    player: Arc<Mutex<Player>>,
    report: Arc<Mutex<dyn Fn(Report) + Send>>,
    interval: Duration,
    finish: Arc<Mutex<bool>>,
}

impl Reporter {
    /// Create a new reporter for the given player and callback.
    pub fn new(
        player: Arc<Mutex<Player>>,
        report: Arc<Mutex<dyn Fn(Report) + Send>>,
        interval: Duration,
    ) -> Self {
        Self {
            player,
            report,
            interval,
            finish: Arc::new(Mutex::new(false)),
        }
    }

    fn run(&self) {
        let mut last_report = Report {
            time: 0.0,
            volume: 0.0,
            duration: 0.0,
            playing: false,
        };

        loop {
            let player = self.player.lock().unwrap();
            let time = player.get_time();
            let volume = player.get_volume();
            let duration = player.get_duration();
            let playing = player.is_playing();

            let report = Report {
                time,
                volume,
                duration,
                playing,
            };

            drop(player);

            if report != last_report {
                (*self.report.lock().unwrap())(report.clone());
                last_report = report;
            }

            if *self.finish.lock().unwrap() {
                break;
            }

            std::thread::sleep(self.interval);
        }
    }

    /// Start the background reporting thread.
    pub fn start(&self) {
        let this = self.clone();
        Some(std::thread::spawn(move || this.run()));
        *self.finish.lock().unwrap() = false;
    }

    /// Stop the background reporting thread.
    pub fn stop(&self) {
        *self.finish.lock().unwrap() = true;
        // if let Some(child) = self.child.take() {
        //     child.join().unwrap();
        // }
    }
}
