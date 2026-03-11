//! Mix-thread startup and decode-worker spawning.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Instant;

use log::info;
use rodio::buffer::SamplesBuffer;

use crate::dsp::effects::{convolution_reverb, AudioEffect, EffectContext};

use super::super::buffer_mixer::{BufferMixer, DecodeBackpressure, SourceKey};
use super::super::decoder_events::DecodeWorkerEvent;
use super::super::types::MixThreadArgs;
use super::decode::{
    spawn_container_decode_worker, spawn_file_decode_worker, DecodeWorkerJoinGuard,
};
use super::state::MixLoopState;

pub(super) fn setup_mix_state(
    args: MixThreadArgs,
    sender: mpsc::SyncSender<(SamplesBuffer, f64)>,
    startup_trace: Instant,
) -> Option<MixLoopState> {
    let MixThreadArgs {
        audio_info,
        buffer_notify,
        effects_reset,
        inline_effects_update,
        inline_track_mix_updates,
        finished_tracks,
        prot,
        abort,
        start_time,
        buffer_settings,
        effects,
        dsp_metrics,
    } = args;
    info!("mix startup trace: thread start");

    let startup = prepare_runtime_startup(&prot, start_time);
    info!(
        "mix startup trace: runtime plan built in {}ms (instances={})",
        startup_trace.elapsed().as_millis(),
        startup.instance_plan.instances.len()
    );
    if startup.instance_plan.instances.is_empty() {
        abort.store(true, Ordering::SeqCst);
        return None;
    }

    let (start_samples, min_mix_samples, convolution_batch_samples) =
        compute_mix_buffer_sizes(&audio_info, &buffer_settings, &effects);
    let track_mix_by_logical = build_track_mix_map(
        &startup.instance_plan.instances,
        &startup.track_mix_settings_by_slot,
    );
    let track_buffer_size = ((audio_info.sample_rate as usize * 10)
        * audio_info.channels.max(1) as usize)
        .max(start_samples * 2);
    let buffer_mixer = BufferMixer::new(
        startup.instance_plan,
        audio_info.sample_rate,
        audio_info.channels.max(1) as usize,
        track_buffer_size,
        track_mix_by_logical,
        min_mix_samples,
    );
    info!(
        "mix startup trace: buffer_mixer ready in {}ms (track_buffer_size={} min_mix_samples={} start_samples={})",
        startup_trace.elapsed().as_millis(),
        track_buffer_size,
        min_mix_samples,
        start_samples
    );

    Some(finalize_mix_startup(
        sender,
        audio_info,
        buffer_notify,
        effects_reset,
        inline_effects_update,
        inline_track_mix_updates,
        finished_tracks,
        prot,
        abort,
        effects,
        dsp_metrics,
        buffer_mixer,
        startup.container_path,
        startup.effect_context,
        start_time,
        convolution_batch_samples,
        start_samples,
        min_mix_samples,
        startup_trace,
    ))
}

struct RuntimeStartup {
    instance_plan: crate::container::prot::RuntimeInstancePlan,
    container_path: Option<String>,
    effect_context: EffectContext,
    track_mix_settings_by_slot: HashMap<u16, (f32, f32)>,
}

fn prepare_runtime_startup(
    prot: &Arc<std::sync::Mutex<crate::container::prot::Prot>>,
    start_time: f64,
) -> RuntimeStartup {
    let p = prot.lock().unwrap();
    RuntimeStartup {
        instance_plan: p.build_runtime_instance_plan(start_time),
        container_path: p.get_container_path(),
        effect_context: EffectContext {
            sample_rate: p.info.sample_rate,
            channels: p.info.channels as usize,
            container_path: p.get_container_path(),
            impulse_response_spec: p.get_impulse_response_spec(),
            impulse_response_tail_db: p.get_impulse_response_tail_db().unwrap_or(-60.0),
        },
        track_mix_settings_by_slot: p.get_track_mix_settings(),
    }
}

#[allow(clippy::too_many_arguments)]
fn finalize_mix_startup(
    sender: mpsc::SyncSender<(SamplesBuffer, f64)>,
    audio_info: crate::container::info::Info,
    buffer_notify: Arc<std::sync::Condvar>,
    effects_reset: Arc<std::sync::atomic::AtomicU64>,
    inline_effects_update: Arc<
        std::sync::Mutex<Option<crate::playback::engine::InlineEffectsUpdate>>,
    >,
    inline_track_mix_updates: Arc<
        std::sync::Mutex<Vec<crate::playback::engine::InlineTrackMixUpdate>>,
    >,
    finished_tracks: Arc<std::sync::Mutex<Vec<u16>>>,
    prot: Arc<std::sync::Mutex<crate::container::prot::Prot>>,
    abort: Arc<std::sync::atomic::AtomicBool>,
    effects: Arc<std::sync::Mutex<Vec<AudioEffect>>>,
    dsp_metrics: Arc<std::sync::Mutex<crate::playback::engine::DspChainMetrics>>,
    buffer_mixer: BufferMixer,
    container_path: Option<String>,
    effect_context: EffectContext,
    start_time: f64,
    convolution_batch_samples: usize,
    start_samples: usize,
    min_mix_samples: usize,
    startup_trace: Instant,
) -> MixLoopState {
    let decode_backpressure = buffer_mixer.decode_backpressure();
    let (packet_rx, decode_workers) = spawn_mix_decode_workers(
        &buffer_mixer,
        container_path,
        start_time,
        audio_info.channels as u8,
        start_samples.max(min_mix_samples),
        &decode_backpressure,
        &abort,
        startup_trace,
    );
    warm_up_effects(&effects, &effect_context, min_mix_samples, startup_trace);

    MixLoopState::new(
        sender,
        audio_info,
        buffer_notify,
        effects_reset,
        inline_effects_update,
        inline_track_mix_updates,
        finished_tracks,
        prot,
        abort,
        effects,
        dsp_metrics,
        buffer_mixer,
        decode_backpressure,
        packet_rx,
        decode_workers,
        effect_context,
        convolution_batch_samples,
        start_samples,
        min_mix_samples,
    )
}

fn compute_mix_buffer_sizes(
    audio_info: &crate::container::info::Info,
    buffer_settings: &Arc<std::sync::Mutex<crate::playback::engine::PlaybackBufferSettings>>,
    effects: &Arc<std::sync::Mutex<Vec<AudioEffect>>>,
) -> (usize, usize, usize) {
    const MIN_MIX_MS: f32 = 30.0;
    let start_buffer_ms = buffer_settings.lock().unwrap().start_buffer_ms;
    let start_samples = ((audio_info.sample_rate as f32 * start_buffer_ms) / 1000.0) as usize
        * audio_info.channels as usize;
    let mut min_mix_samples = (((audio_info.sample_rate as f32 * MIN_MIX_MS) / 1000.0) as usize)
        .max(1)
        * audio_info.channels as usize;
    let has_convolution = effects
        .lock()
        .unwrap()
        .iter()
        .any(|e| matches!(e, AudioEffect::ConvolutionReverb(e) if e.enabled));
    let convolution_batch_samples = if has_convolution {
        convolution_reverb::preferred_batch_samples(audio_info.channels.max(1) as usize)
    } else {
        0
    };
    if has_convolution && convolution_batch_samples > 0 {
        min_mix_samples =
            min_mix_samples.div_ceil(convolution_batch_samples) * convolution_batch_samples;
    }
    (start_samples, min_mix_samples, convolution_batch_samples)
}

fn build_track_mix_map(
    instances: &[crate::container::prot::RuntimeInstanceMeta],
    track_mix_settings_by_slot: &HashMap<u16, (f32, f32)>,
) -> HashMap<usize, (f32, f32)> {
    let mut track_mix_by_logical = HashMap::new();
    for instance in instances {
        track_mix_by_logical
            .entry(instance.logical_track_index)
            .or_insert_with(|| {
                track_mix_settings_by_slot
                    .get(&(instance.slot_index as u16))
                    .copied()
                    .unwrap_or((1.0, 0.0))
            });
    }
    track_mix_by_logical
}

fn warm_up_effects(
    effects: &Arc<std::sync::Mutex<Vec<AudioEffect>>>,
    effect_context: &EffectContext,
    min_mix_samples: usize,
    startup_trace: Instant,
) {
    if min_mix_samples > 0 {
        for effect in effects.lock().unwrap().iter_mut() {
            effect.warm_up(effect_context);
        }
    }
    info!(
        "mix startup trace: effect warmup complete in {}ms (warmup_samples={})",
        startup_trace.elapsed().as_millis(),
        min_mix_samples
    );
}

fn spawn_mix_decode_workers(
    buffer_mixer: &BufferMixer,
    container_path: Option<String>,
    start_time: f64,
    channels: u8,
    startup_gate_samples: usize,
    decode_backpressure: &Arc<DecodeBackpressure>,
    abort: &Arc<AtomicBool>,
    startup_trace: Instant,
) -> (mpsc::Receiver<DecodeWorkerEvent>, DecodeWorkerJoinGuard) {
    let (packet_tx, packet_rx) = mpsc::sync_channel::<DecodeWorkerEvent>(64);
    let mut decode_workers = DecodeWorkerJoinGuard::default();
    let sources = buffer_mixer.sources();
    let (track_ids, file_paths) = partition_sources(&sources);
    maybe_enable_startup_priority(decode_backpressure, &file_paths, startup_gate_samples);
    spawn_decode_workers(
        &mut decode_workers,
        packet_tx.clone(),
        container_path,
        track_ids,
        file_paths,
        start_time,
        channels,
        abort,
        decode_backpressure,
    );
    drop(packet_tx);
    log_decode_worker_counts(&sources, startup_trace);
    (packet_rx, decode_workers)
}

fn partition_sources(sources: &[SourceKey]) -> (HashSet<u32>, HashSet<String>) {
    let mut track_ids = HashSet::new();
    let mut file_paths = HashSet::new();
    for source in sources {
        match source {
            SourceKey::TrackId(id) => {
                track_ids.insert(*id);
            }
            SourceKey::FilePath(path) => {
                file_paths.insert(path.clone());
            }
        }
    }
    (track_ids, file_paths)
}

fn maybe_enable_startup_priority(
    decode_backpressure: &Arc<DecodeBackpressure>,
    file_paths: &HashSet<String>,
    startup_gate_samples: usize,
) {
    if !file_paths.is_empty() && startup_gate_samples > 0 {
        decode_backpressure.enable_startup_priority(startup_gate_samples);
    }
}

#[allow(clippy::too_many_arguments)]
fn spawn_decode_workers(
    decode_workers: &mut DecodeWorkerJoinGuard,
    packet_tx: mpsc::SyncSender<DecodeWorkerEvent>,
    container_path: Option<String>,
    track_ids: HashSet<u32>,
    file_paths: HashSet<String>,
    start_time: f64,
    channels: u8,
    abort: &Arc<AtomicBool>,
    decode_backpressure: &Arc<DecodeBackpressure>,
) {
    if !track_ids.is_empty() {
        if let Some(path) = container_path {
            decode_workers.push(spawn_container_decode_worker(
                path,
                track_ids.into_iter().collect(),
                start_time,
                channels,
                packet_tx.clone(),
                abort.clone(),
                decode_backpressure.clone(),
            ));
        }
    }
    for path in file_paths {
        decode_workers.push(spawn_file_decode_worker(
            path,
            start_time,
            channels,
            packet_tx.clone(),
            abort.clone(),
            decode_backpressure.clone(),
        ));
    }
}

fn log_decode_worker_counts(sources: &[SourceKey], startup_trace: Instant) {
    let container_count = usize::from(sources.iter().any(|s| matches!(s, SourceKey::TrackId(_))));
    let file_count = sources
        .iter()
        .filter(|s| matches!(s, SourceKey::FilePath(_)))
        .count();
    info!(
        "mix startup trace: decode workers spawned in {}ms (container_sources={} file_sources={})",
        startup_trace.elapsed().as_millis(),
        container_count,
        file_count
    );
}
