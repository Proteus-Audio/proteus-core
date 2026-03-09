//! Peaks subcommand handlers.

use clap::ArgMatches;
use log::error;
use serde::Serialize;

#[derive(Serialize)]
struct PeakWindow {
    max: f32,
    min: f32,
}

#[derive(Serialize)]
struct PeaksChannel {
    peaks: Vec<PeakWindow>,
}

#[derive(Serialize)]
struct PeaksOutput {
    channels: Vec<PeaksChannel>,
}

/// Handle `peaks` commands.
pub(crate) fn run_peaks(args: &ArgMatches) -> i32 {
    match args.subcommand() {
        Some(("json", sub_args)) => {
            let file_path = sub_args.get_one::<String>("INPUT").unwrap();
            let limited = sub_args.get_flag("limited");
            run_peaks_json(file_path, limited)
        }
        Some(("write", sub_args)) => {
            let input_audio = sub_args.get_one::<String>("INPUT").unwrap();
            let output_peaks = sub_args.get_one::<String>("OUTPUT").unwrap();
            run_peaks_write(input_audio, output_peaks)
        }
        Some(("read", sub_args)) => {
            let peaks_file = sub_args.get_one::<String>("INPUT").unwrap();
            let start = match sub_args.get_one::<String>("start") {
                Some(value) => match value.parse::<f64>() {
                    Ok(parsed) => Some(parsed),
                    Err(err) => {
                        error!("Invalid --start value '{}': {}", value, err);
                        return -1;
                    }
                },
                None => None,
            };
            let end = match sub_args.get_one::<String>("end") {
                Some(value) => match value.parse::<f64>() {
                    Ok(parsed) => Some(parsed),
                    Err(err) => {
                        error!("Invalid --end value '{}': {}", value, err);
                        return -1;
                    }
                },
                None => None,
            };
            let target_peaks = match sub_args.get_one::<String>("peaks") {
                Some(value) => match value.parse::<usize>() {
                    Ok(parsed) => Some(parsed),
                    Err(err) => {
                        error!("Invalid --peaks value '{}': {}", value, err);
                        return -1;
                    }
                },
                None => None,
            };
            let channel_count = match sub_args.get_one::<String>("channels") {
                Some(value) => match value.parse::<usize>() {
                    Ok(parsed) => Some(parsed),
                    Err(err) => {
                        error!("Invalid --channels value '{}': {}", value, err);
                        return -1;
                    }
                },
                None => None,
            };
            run_peaks_read(peaks_file, start, end, target_peaks, channel_count)
        }
        Some((unknown, _)) => {
            error!("Unknown peaks subcommand: {}", unknown);
            -1
        }
        None => {
            if let Some(file_path) = args.get_one::<String>("INPUT") {
                let limited = args.get_flag("limited");
                return run_peaks_json(file_path, limited);
            }

            error!(
                "Missing peaks command. Use `peaks json <input>`, `peaks write <input> <output>`, or `peaks read <input>`"
            );
            -1
        }
    }
}

fn run_peaks_json(file_path: &str, limited: bool) -> i32 {
    let peaks = match proteus_lib::peaks::extract_peaks_from_audio(file_path, limited) {
        Ok(peaks) => peaks,
        Err(err) => {
            error!("Failed to extract peaks: {}", err);
            return -1;
        }
    };
    print_peaks_json(&peaks)
}

fn run_peaks_write(input_audio: &str, output_peaks: &str) -> i32 {
    match proteus_lib::peaks::write_peaks(input_audio, output_peaks) {
        Ok(()) => {
            println!("Wrote peaks to {}", output_peaks);
            0
        }
        Err(err) => {
            error!("Failed to write peaks: {}", err);
            -1
        }
    }
}

fn run_peaks_read(
    peaks_file: &str,
    start: Option<f64>,
    end: Option<f64>,
    target_peaks: Option<usize>,
    channel_count: Option<usize>,
) -> i32 {
    if (start.is_some() && end.is_none()) || (start.is_none() && end.is_some()) {
        error!("Both --start and --end must be provided together");
        return -1;
    }

    let peaks = match proteus_lib::peaks::get_peaks(
        peaks_file,
        proteus_lib::peaks::GetPeaksOptions {
            start_seconds: start,
            end_seconds: end,
            target_peaks,
            channels: channel_count,
        },
    ) {
        Ok(peaks) => peaks,
        Err(err) => {
            error!("Failed to read peaks: {}", err);
            return -1;
        }
    };

    print_peaks_json(&peaks)
}

fn print_peaks_json(peaks: &proteus_lib::peaks::PeaksData) -> i32 {
    let channels = peaks
        .channels
        .iter()
        .map(|channel| PeaksChannel {
            peaks: channel
                .iter()
                .map(|peak| PeakWindow {
                    max: peak.max,
                    min: peak.min,
                })
                .collect(),
        })
        .collect();
    let output = PeaksOutput { channels };
    match serde_json::to_string_pretty(&output) {
        Ok(json) => {
            println!("{}", json);
            0
        }
        Err(err) => {
            error!("Failed to serialize peaks: {}", err);
            -1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::run_peaks_json;

    #[test]
    fn peaks_json_missing_file_returns_error_code() {
        let code = run_peaks_json("/definitely/missing/audio.file", false);
        assert_eq!(code, -1);
    }
}
