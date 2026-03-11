//! Impulse response loading, caching, and reverb kernel construction.

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use log::warn;

use super::impulse_response;
use super::reverb;
use super::spec::ImpulseResponseSpec;

type ImpulseResponseCacheMap =
    HashMap<ImpulseResponseCacheKey, Arc<impulse_response::ImpulseResponse>>;
static IMPULSE_RESPONSE_CACHE: OnceLock<Mutex<ImpulseResponseCacheMap>> = OnceLock::new();
type ReverbKernelCacheMap = HashMap<ReverbKernelCacheKey, Arc<reverb::Reverb>>;
static REVERB_KERNEL_CACHE: OnceLock<Mutex<ReverbKernelCacheMap>> = OnceLock::new();

/// Clear process-wide convolution caches for test/session isolation.
pub fn clear_global_caches() {
    if let Some(cache) = IMPULSE_RESPONSE_CACHE.get() {
        cache.lock().unwrap().clear();
    }
    if let Some(cache) = REVERB_KERNEL_CACHE.get() {
        cache.lock().unwrap().clear();
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum ImpulseResponseCacheSource {
    Attachment {
        container_path: String,
        attachment_name: String,
    },
    FilePath {
        path: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ImpulseResponseCacheKey {
    source: ImpulseResponseCacheSource,
    tail_db_bits: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ReverbKernelCacheKey {
    channels: usize,
    impulse_response: ImpulseResponseCacheKey,
}

pub(super) fn build_reverb_with_impulse_response(
    channels: usize,
    dry_wet: f32,
    impulse_spec: Option<ImpulseResponseSpec>,
    container_path: Option<&str>,
    tail_db: f32,
) -> Option<reverb::Reverb> {
    let impulse_spec = impulse_spec?;

    use self::impulse_response::{
        load_impulse_response_from_file_with_tail,
        load_impulse_response_from_prot_attachment_with_tail, ImpulseResponseError,
    };

    #[derive(Debug)]
    enum ReverbLoadError {
        MissingContainerPath,
        PathNotFound(PathBuf),
        AttachmentLoad(ImpulseResponseError),
        FileLoad(ImpulseResponseError),
    }

    impl fmt::Display for ReverbLoadError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Self::MissingContainerPath => {
                    write!(f, "missing container path for attachment")
                }
                Self::PathNotFound(path) => {
                    write!(f, "impulse response path not found: {}", path.display())
                }
                Self::AttachmentLoad(err) => {
                    write!(f, "failed to load attachment impulse response: {}", err)
                }
                Self::FileLoad(err) => write!(f, "failed to load file impulse response: {}", err),
            }
        }
    }

    impl std::error::Error for ReverbLoadError {}

    let result = match impulse_spec {
        ImpulseResponseSpec::Attachment(name) => container_path
            .ok_or(ReverbLoadError::MissingContainerPath)
            .and_then(|path| {
                let cache_key = ImpulseResponseCacheKey {
                    source: ImpulseResponseCacheSource::Attachment {
                        container_path: path.to_string(),
                        attachment_name: name.clone(),
                    },
                    tail_db_bits: tail_db.to_bits(),
                };
                let impulse_response = load_cached_impulse_response(cache_key.clone(), || {
                    load_impulse_response_from_prot_attachment_with_tail(path, &name, Some(tail_db))
                        .map_err(ReverbLoadError::AttachmentLoad)
                })?;
                Ok((cache_key, impulse_response))
            }),
        ImpulseResponseSpec::FilePath(path) => {
            let resolved_path = resolve_impulse_response_path(container_path, &path);
            if resolved_path.exists() {
                let cache_key = ImpulseResponseCacheKey {
                    source: ImpulseResponseCacheSource::FilePath {
                        path: resolved_path.to_string_lossy().into_owned(),
                    },
                    tail_db_bits: tail_db.to_bits(),
                };
                load_cached_impulse_response(cache_key.clone(), || {
                    load_impulse_response_from_file_with_tail(&resolved_path, Some(tail_db))
                        .map_err(ReverbLoadError::FileLoad)
                })
                .map(|impulse_response| (cache_key, impulse_response))
            } else {
                match container_path {
                    Some(container_path) => {
                        let fallback_name = Path::new(&path)
                            .file_name()
                            .and_then(|name| name.to_str())
                            .map(|name| name.to_string());
                        if let Some(fallback_name) = fallback_name {
                            let cache_key = ImpulseResponseCacheKey {
                                source: ImpulseResponseCacheSource::Attachment {
                                    container_path: container_path.to_string(),
                                    attachment_name: fallback_name.clone(),
                                },
                                tail_db_bits: tail_db.to_bits(),
                            };
                            load_cached_impulse_response(cache_key.clone(), || {
                                load_impulse_response_from_prot_attachment_with_tail(
                                    container_path,
                                    &fallback_name,
                                    Some(tail_db),
                                )
                                .map_err(ReverbLoadError::AttachmentLoad)
                            })
                            .map(|impulse_response| (cache_key, impulse_response))
                        } else {
                            Err(ReverbLoadError::PathNotFound(resolved_path))
                        }
                    }
                    None => Err(ReverbLoadError::PathNotFound(resolved_path)),
                }
            }
        }
    };

    match result {
        Ok((impulse_response_cache_key, impulse_response)) => {
            let kernel_cache_key = ReverbKernelCacheKey {
                channels,
                impulse_response: impulse_response_cache_key,
            };
            Some(build_cached_reverb(
                kernel_cache_key,
                channels,
                dry_wet,
                &impulse_response,
            ))
        }
        Err(err) => {
            warn!(
                "Failed to load impulse response ({}); skipping convolution reverb.",
                err
            );
            None
        }
    }
}

fn build_cached_reverb(
    cache_key: ReverbKernelCacheKey,
    channels: usize,
    dry_wet: f32,
    impulse_response: &impulse_response::ImpulseResponse,
) -> reverb::Reverb {
    use super::DEFAULT_DRY_WET;

    let cache = REVERB_KERNEL_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(template) = cache.lock().unwrap().get(&cache_key).cloned() {
        let mut reverb = (*template).clone();
        reverb.clear_state();
        reverb.set_dry_wet(dry_wet);
        return reverb;
    }

    let mut template =
        reverb::Reverb::new_with_impulse_response(channels, DEFAULT_DRY_WET, impulse_response);
    template.clear_state();
    let template = Arc::new(template);

    let mut cache_guard = cache.lock().unwrap();
    let template = cache_guard
        .entry(cache_key)
        .or_insert_with(|| template.clone())
        .clone();
    let mut reverb = (*template).clone();
    reverb.clear_state();
    reverb.set_dry_wet(dry_wet);
    reverb
}

fn load_cached_impulse_response<F, E>(
    cache_key: ImpulseResponseCacheKey,
    loader: F,
) -> Result<Arc<impulse_response::ImpulseResponse>, E>
where
    F: FnOnce() -> Result<impulse_response::ImpulseResponse, E>,
{
    let cache = IMPULSE_RESPONSE_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(cached) = cache.lock().unwrap().get(&cache_key).cloned() {
        return Ok(cached);
    }

    let loaded = Arc::new(loader()?);
    let mut cache_guard = cache.lock().unwrap();
    let cached = cache_guard
        .entry(cache_key)
        .or_insert_with(|| loaded.clone())
        .clone();
    Ok(cached)
}

pub(super) fn resolve_impulse_response_path(container_path: Option<&str>, path: &str) -> PathBuf {
    let path = Path::new(path);
    if path.is_absolute() {
        return path.to_path_buf();
    }

    if let Some(container_path) = container_path {
        if let Some(parent) = Path::new(container_path).parent() {
            return parent.join(path);
        }
    }

    path.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::{clear_global_caches, resolve_impulse_response_path};
    use std::path::PathBuf;

    #[test]
    fn resolve_impulse_response_path_uses_container_parent_for_relative_paths() {
        let resolved = resolve_impulse_response_path(Some("/tmp/project/song.prot"), "ir/hall.wav");
        assert_eq!(resolved, PathBuf::from("/tmp/project/ir/hall.wav"));
    }

    #[test]
    fn clear_global_caches_is_idempotent() {
        clear_global_caches();
        clear_global_caches();
    }
}
