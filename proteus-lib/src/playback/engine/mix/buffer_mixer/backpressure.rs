//! Decode backpressure state and synchronization.

use log::debug;
use std::collections::HashMap;
use std::sync::{Condvar, Mutex};

use super::{BufferInstance, SourceKey};

#[derive(Debug, Clone)]
struct DecodeBackpressureInstance {
    capacity_samples: usize,
    buffered_samples: usize,
    reserved_samples: usize,
    finished: bool,
}

#[derive(Debug, Default)]
struct DecodeBackpressureState {
    shutdown: bool,
    waiting_threads: usize,
    startup_priority_target_samples: Option<usize>,
    instances: Vec<DecodeBackpressureInstance>,
    source_to_instances: HashMap<SourceKey, Vec<usize>>,
}

/// Shared gate used by decode workers to avoid overrunning per-instance buffers.
#[derive(Debug, Default)]
pub(crate) struct DecodeBackpressure {
    state: Mutex<DecodeBackpressureState>,
    cv: Condvar,
}

impl DecodeBackpressure {
    /// Build backpressure state from the mixer's per-instance buffers.
    pub(super) fn from_instances(instances: &[BufferInstance]) -> Self {
        let mut state = DecodeBackpressureState::default();
        state.instances.reserve(instances.len());
        for (index, instance) in instances.iter().enumerate() {
            let source = SourceKey::from(&instance.meta.source_key);
            state
                .source_to_instances
                .entry(source.clone())
                .or_default()
                .push(index);
            state.instances.push(DecodeBackpressureInstance {
                capacity_samples: instance.buffer_capacity_samples,
                buffered_samples: instance.buffer.len(),
                reserved_samples: 0,
                finished: instance.finished,
            });
        }
        Self {
            state: Mutex::new(state),
            cv: Condvar::new(),
        }
    }

    /// Block until the given source can atomically reserve room across all routed instances.
    pub(crate) fn wait_for_source_room(
        &self,
        source: &SourceKey,
        required_samples: usize,
        abort: &std::sync::atomic::AtomicBool,
    ) -> bool {
        if required_samples == 0 {
            return true;
        }

        let mut guard = self.state.lock().unwrap_or_else(|_| {
            panic!("decode backpressure state lock poisoned — a thread panicked while holding it")
        });
        let mut wait_count = 0usize;
        loop {
            if guard.shutdown || abort.load(std::sync::atomic::Ordering::Relaxed) {
                debug!(
                    "decode_backpressure wait abort/shutdown: source={:?} required_samples={} shutdown={} abort={}",
                    source,
                    required_samples,
                    guard.shutdown,
                    abort.load(std::sync::atomic::Ordering::Relaxed)
                );
                return false;
            }
            let status = source_room_status(
                &guard,
                source,
                required_samples,
                log::log_enabled!(log::Level::Debug),
            );
            if status.allowed {
                reserve_source_room(&mut guard, source, required_samples);
                if wait_count > 0 {
                    debug!(
                        "decode_backpressure wait satisfied: source={:?} required_samples={} waits={} details={}",
                        source,
                        required_samples,
                        wait_count,
                        status.summary
                    );
                }
                return true;
            }
            if wait_count == 0 {
                debug!(
                    "decode_backpressure wait start: source={:?} required_samples={} details={}",
                    source, required_samples, status.summary
                );
            } else {
                debug!(
                    "decode_backpressure wait continue: source={:?} required_samples={} waits={} details={}",
                    source,
                    required_samples,
                    wait_count,
                    status.summary
                );
            }
            wait_count = wait_count.saturating_add(1);
            guard.waiting_threads = guard.waiting_threads.saturating_add(1);
            guard = self.cv.wait(guard).unwrap();
            guard.waiting_threads = guard.waiting_threads.saturating_sub(1);
            debug!(
                "decode_backpressure wait wake: source={:?} required_samples={} waits={}",
                source, required_samples, wait_count
            );
        }
    }

    /// Reconcile reserved room with actual routed writes and notify waiting workers.
    pub(super) fn on_samples_pushed(
        &self,
        instance_index: usize,
        attempted_samples: usize,
        pushed_samples: usize,
        is_full: bool,
    ) {
        if attempted_samples == 0 && pushed_samples == 0 && !is_full {
            return;
        }
        let mut guard = self.state.lock().unwrap_or_else(|_| {
            panic!("decode backpressure state lock poisoned — a thread panicked while holding it")
        });
        if let Some(instance) = guard.instances.get_mut(instance_index) {
            instance.reserved_samples = instance.reserved_samples.saturating_sub(attempted_samples);
            instance.buffered_samples = instance
                .buffered_samples
                .saturating_add(pushed_samples)
                .min(instance.capacity_samples.max(1));
            debug!(
                "decode_backpressure push: instance_index={} attempted_samples={} pushed_samples={} reserved={} buffered={} capacity={} finished={} full_flag={}",
                instance_index,
                attempted_samples,
                pushed_samples,
                instance.reserved_samples,
                instance.buffered_samples,
                instance.capacity_samples,
                instance.finished,
                is_full
            );
        }
        if attempted_samples > 0 || pushed_samples > 0 || !is_full {
            self.cv.notify_all();
        }
    }

    /// Record samples consumed from an instance buffer and notify waiting workers.
    pub(super) fn on_samples_popped(&self, instance_index: usize, popped_samples: usize) {
        if popped_samples == 0 {
            return;
        }
        let mut guard = self.state.lock().unwrap_or_else(|_| {
            panic!("decode backpressure state lock poisoned — a thread panicked while holding it")
        });
        if let Some(instance) = guard.instances.get_mut(instance_index) {
            instance.buffered_samples = instance.buffered_samples.saturating_sub(popped_samples);
            debug!(
                "decode_backpressure pop: instance_index={} popped_samples={} reserved={} buffered={} capacity={} finished={}",
                instance_index,
                popped_samples,
                instance.reserved_samples,
                instance.buffered_samples,
                instance.capacity_samples,
                instance.finished
            );
        }
        self.cv.notify_all();
    }

    /// Mark an instance finished so it no longer blocks backpressure checks.
    pub(super) fn on_finished(&self, instance_index: usize) {
        let mut guard = self.state.lock().unwrap_or_else(|_| {
            panic!("decode backpressure state lock poisoned — a thread panicked while holding it")
        });
        if let Some(instance) = guard.instances.get_mut(instance_index) {
            if !instance.finished {
                instance.finished = true;
                instance.reserved_samples = 0;
                debug!(
                    "decode_backpressure finished: instance_index={} reserved={} buffered={} capacity={}",
                    instance_index,
                    instance.reserved_samples,
                    instance.buffered_samples,
                    instance.capacity_samples
                );
                self.cv.notify_all();
            }
        }
    }

    /// Wake all waiters and force future room checks to fail.
    pub(crate) fn shutdown(&self) {
        let mut guard = self.state.lock().unwrap_or_else(|_| {
            panic!("decode backpressure state lock poisoned — a thread panicked while holding it")
        });
        guard.shutdown = true;
        debug!("decode_backpressure shutdown");
        self.cv.notify_all();
    }

    /// Return true when any decode worker is blocked waiting for room.
    pub(crate) fn has_waiters(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|_| {
                panic!(
                    "decode backpressure state lock poisoned — a thread panicked while holding it"
                )
            })
            .waiting_threads
            > 0
    }

    /// Enable startup fairness mode with a per-instance target occupancy.
    pub(crate) fn enable_startup_priority(&self, target_samples: usize) {
        let mut guard = self.state.lock().unwrap_or_else(|_| {
            panic!("decode backpressure state lock poisoned — a thread panicked while holding it")
        });
        guard.startup_priority_target_samples = Some(target_samples.max(1));
        self.cv.notify_all();
    }

    /// Disable startup fairness mode and resume steady-state buffering behavior.
    pub(crate) fn disable_startup_priority(&self) {
        let mut guard = self.state.lock().unwrap_or_else(|_| {
            panic!("decode backpressure state lock poisoned — a thread panicked while holding it")
        });
        guard.startup_priority_target_samples = None;
        self.cv.notify_all();
    }
}

#[derive(Debug, Clone)]
struct SourceRoomStatus {
    allowed: bool,
    summary: String,
}

/// Evaluate whether a source can reserve room for its next decoded packet.
/// Compute whether a source can reserve room for its next packet span.
fn source_room_status(
    state: &DecodeBackpressureState,
    source: &SourceKey,
    required_samples: usize,
    include_details: bool,
) -> SourceRoomStatus {
    let Some(instance_indices) = state.source_to_instances.get(source) else {
        return SourceRoomStatus {
            allowed: true,
            summary: if include_details {
                "no_instances".to_string()
            } else {
                String::new()
            },
        };
    };

    let mut saw_unfinished = false;
    let mut all_have_target_room = true;
    let startup_target = state.startup_priority_target_samples;
    let mut source_has_startup_deficit = false;
    let mut parts = include_details.then(Vec::new);

    for instance_index in instance_indices {
        update_source_room_state(
            state.instances.get(*instance_index),
            *instance_index,
            startup_target,
            required_samples,
            &mut saw_unfinished,
            &mut all_have_target_room,
            &mut source_has_startup_deficit,
            &mut parts,
        );
    }

    if !saw_unfinished {
        return SourceRoomStatus {
            allowed: true,
            summary: parts
                .as_ref()
                .map(|parts| format!("all_finished [{}]", parts.join(", ")))
                .unwrap_or_default(),
        };
    }

    let global_startup_deficit_exists = startup_target.is_some_and(|startup_target| {
        state.instances.iter().any(|instance| {
            if instance.finished {
                return false;
            }
            let occupied = instance
                .buffered_samples
                .saturating_add(instance.reserved_samples)
                .min(instance.capacity_samples);
            let target = startup_target.min(instance.capacity_samples.max(1));
            occupied < target
        })
    });

    // Routing writes a full packet span (real samples or underlay) into every unfinished instance
    // for the source. Allowing partial room here silently drops packet tails and compresses time.
    let startup_fairness_allows = !global_startup_deficit_exists || source_has_startup_deficit;
    let allowed = all_have_target_room && startup_fairness_allows;
    SourceRoomStatus {
        allowed,
        summary: if let Some(parts) = parts.as_ref() {
            format!(
                "allowed={} all_target={} startup_deficit_global={} startup_deficit_source={} [{}]",
                allowed,
                all_have_target_room,
                global_startup_deficit_exists,
                source_has_startup_deficit,
                parts.join(", ")
            )
        } else {
            String::new()
        },
    }
}

#[allow(clippy::too_many_arguments)]
fn update_source_room_state(
    instance: Option<&DecodeBackpressureInstance>,
    instance_index: usize,
    startup_target: Option<usize>,
    required_samples: usize,
    saw_unfinished: &mut bool,
    all_have_target_room: &mut bool,
    source_has_startup_deficit: &mut bool,
    parts: &mut Option<Vec<String>>,
) {
    let Some(instance) = instance else {
        return;
    };
    if instance.finished {
        if let Some(parts) = parts.as_mut() {
            parts.push(format!("i{}:finished", instance_index));
        }
        return;
    }

    *saw_unfinished = true;
    let occupied = occupied_samples(instance);
    let free = instance.capacity_samples.saturating_sub(occupied);
    if startup_target.is_some_and(|target| occupied < target.min(instance.capacity_samples.max(1)))
    {
        *source_has_startup_deficit = true;
    }

    let target_room = required_samples.min(instance.capacity_samples.max(1));
    *all_have_target_room &= free >= target_room;
    if let Some(parts) = parts.as_mut() {
        parts.push(format!(
            "i{}:buf={} res={} /{} free={} target={}",
            instance_index,
            instance.buffered_samples,
            instance.reserved_samples,
            instance.capacity_samples,
            free,
            target_room
        ));
    }
}

fn occupied_samples(instance: &DecodeBackpressureInstance) -> usize {
    instance
        .buffered_samples
        .saturating_add(instance.reserved_samples)
        .min(instance.capacity_samples)
}

/// Reserve space for a source across all routed instances before packet delivery.
/// Reserve capacity for a source packet across all unfinished routed instances.
fn reserve_source_room(
    state: &mut DecodeBackpressureState,
    source: &SourceKey,
    required_samples: usize,
) {
    let Some(instance_indices) = state.source_to_instances.get(source).cloned() else {
        return;
    };
    for instance_index in instance_indices {
        let Some(instance) = state.instances.get_mut(instance_index) else {
            continue;
        };
        if instance.finished {
            continue;
        }
        let reserve = required_samples.min(instance.capacity_samples.max(1));
        instance.reserved_samples = instance
            .reserved_samples
            .saturating_add(reserve)
            .min(instance.capacity_samples.max(1));
    }
}

#[cfg(test)]
mod tests;
