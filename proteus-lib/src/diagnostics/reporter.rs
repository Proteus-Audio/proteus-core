//! Periodic playback state reporter for UI updates.

use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread::JoinHandle,
    time::Duration,
};

use crate::playback::player::PlayerState;

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
    time: Arc<Mutex<f64>>,
    volume: Arc<Mutex<f32>>,
    duration: Arc<Mutex<f64>>,
    state: Arc<Mutex<PlayerState>>,
    report: Arc<Mutex<dyn Fn(Report) + Send>>,
    interval: Duration,
    finish: Arc<AtomicBool>,
    thread_handle: Arc<Mutex<Option<JoinHandle<()>>>>,
}

impl Reporter {
    /// Create a new reporter for the given player and callback.
    pub fn new(
        time: Arc<Mutex<f64>>,
        volume: Arc<Mutex<f32>>,
        duration: Arc<Mutex<f64>>,
        state: Arc<Mutex<PlayerState>>,
        report: Arc<Mutex<dyn Fn(Report) + Send>>,
        interval: Duration,
    ) -> Self {
        Self {
            time,
            volume,
            duration,
            state,
            report,
            interval,
            finish: Arc::new(AtomicBool::new(false)),
            thread_handle: Arc::new(Mutex::new(None)),
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
            let report = Report {
                time: *self.time.lock().unwrap(),
                volume: *self.volume.lock().unwrap(),
                duration: *self.duration.lock().unwrap(),
                playing: *self.state.lock().unwrap() == PlayerState::Playing,
            };

            if report != last_report {
                (*self.report.lock().unwrap())(report.clone());
                last_report = report;
            }

            if self.finish.load(Ordering::Relaxed) {
                break;
            }

            std::thread::sleep(self.interval);
        }
    }

    /// Start the background reporting thread.
    pub fn start(&self) {
        self.stop();
        self.finish.store(false, Ordering::Relaxed);
        let this = self.clone();
        let handle = std::thread::spawn(move || this.run());
        *self.thread_handle.lock().unwrap() = Some(handle);
    }

    /// Stop the background reporting thread.
    pub fn stop(&self) {
        self.finish.store(true, Ordering::Relaxed);
        if let Some(handle) = self.thread_handle.lock().unwrap().take() {
            if handle.thread().id() == std::thread::current().id() {
                log::warn!("reporter stop called from reporter thread; skipping join");
            } else if handle.join().is_err() {
                log::warn!("reporter thread panicked during join");
            }
        }
    }
}
