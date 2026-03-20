//! Offline effect-meter subcommands.

use clap::ArgMatches;
use symphonia::core::errors::Result;

#[cfg(feature = "effect-meter-cli")]
use proteus_lib::tools::effect_meter::{
    run_report, EffectMeterReport, EffectMeterRunConfig, EffectMeterSummaryMode,
};

#[cfg(feature = "effect-meter-cli")]
use super::spectral_graph;

#[cfg(feature = "effect-meter-cli")]
use crate::project_files;

/// Run a meter subcommand.
pub(crate) fn run_meter_subcommand(args: &ArgMatches) -> Result<i32> {
    let (meter_cmd, meter_args) = match args.subcommand() {
        Some((cmd, args)) => (cmd, args),
        None => return Ok(1),
    };
    match meter_cmd {
        "effects" => run_effect_meter(meter_args).map(|code| code.unwrap_or(0)),
        _ => Ok(1),
    }
}

fn run_effect_meter(_args: &ArgMatches) -> Result<Option<i32>> {
    #[cfg(not(feature = "effect-meter-cli"))]
    {
        eprintln!("Effect metering requires the `effect-meter-cli` feature.");
        Ok(Some(1))
    }

    #[cfg(feature = "effect-meter-cli")]
    {
        let args = _args;
        let input_path = args.get_one::<String>("INPUT").unwrap().clone();
        let effects = if let Some(path) = args.get_one::<String>("effects-json") {
            match project_files::load_effects_json(path) {
                Ok(effects) => effects,
                Err(err) => {
                    log::error!("Failed to load effects json: {}", err);
                    return Ok(Some(1));
                }
            }
        } else {
            Vec::new()
        };

        let output_format = args
            .get_one::<String>("format")
            .map(String::as_str)
            .unwrap_or("table");
        let summary_mode = match parse_summary_mode(
            args.get_one::<String>("summary")
                .map(String::as_str)
                .unwrap_or("max"),
        ) {
            Some(mode) => mode,
            None => {
                eprintln!("Invalid summary mode. Expected `final` or `max`.");
                return Ok(Some(1));
            }
        };
        let include_spectral = args.get_flag("spectral");

        #[cfg(not(feature = "effect-meter-cli-spectral"))]
        if include_spectral {
            eprintln!("Spectral effect metering requires the `effect-meter-cli-spectral` feature.");
            return Ok(Some(1));
        }

        let duration_seconds = if let Some(raw) = args.get_one::<String>("duration") {
            match raw.parse::<f64>() {
                Ok(value) => Some(value),
                Err(_) => {
                    eprintln!("Invalid duration value.");
                    return Ok(Some(1));
                }
            }
        } else {
            None
        };

        let report = match run_report(&EffectMeterRunConfig {
            input_path,
            effects,
            seek_seconds: parse_f64_arg(args, "seek", 0.0),
            duration_seconds,
            summary_mode,
            include_spectral,
        }) {
            Ok(report) => report,
            Err(err) => {
                log::error!("{}", err);
                return Ok(Some(1));
            }
        };

        match output_format {
            "table" => print_table(&report),
            "bars" => print_bars(&report),
            "json" => match serde_json::to_string_pretty(&report) {
                Ok(json) => println!("{}", json),
                Err(err) => {
                    log::error!("Failed to serialize meter report: {}", err);
                    return Ok(Some(1));
                }
            },
            _ => {
                eprintln!("Invalid format. Expected `table`, `bars`, or `json`.");
                return Ok(Some(1));
            }
        }

        Ok(Some(0))
    }
}

#[cfg(feature = "effect-meter-cli")]
fn parse_summary_mode(value: &str) -> Option<EffectMeterSummaryMode> {
    match value {
        "final" => Some(EffectMeterSummaryMode::Final),
        "max" => Some(EffectMeterSummaryMode::Max),
        _ => None,
    }
}

#[cfg(feature = "effect-meter-cli")]
fn parse_f64_arg(args: &ArgMatches, key: &str, default: f64) -> f64 {
    args.get_one::<String>(key)
        .and_then(|value| value.parse::<f64>().ok())
        .unwrap_or(default)
}

#[cfg(feature = "effect-meter-cli")]
fn print_table(report: &EffectMeterReport) {
    println!(
        "input={} sample_rate={}Hz channels={} frames={} summary={:?}",
        report.input_path,
        report.sample_rate,
        report.channels,
        report.frames_processed,
        report.summary_mode
    );
    if report.effects.is_empty() {
        println!("No effects configured.");
        return;
    }

    println!(
        "{:<3} {:<18} {:>8} {:>8} {:>7} {:>8} {:>8} {:>7}",
        "idx", "effect", "in_peak", "out_peak", "delta", "in_rms", "out_rms", "delta"
    );
    for effect in &report.effects {
        let in_peak = max_dbfs(&effect.levels.input.peak);
        let out_peak = max_dbfs(&effect.levels.output.peak);
        let in_rms = max_dbfs(&effect.levels.input.rms);
        let out_rms = max_dbfs(&effect.levels.output.rms);
        println!(
            "{:<3} {:<18} {:>8} {:>8} {:>7} {:>8} {:>8} {:>7}",
            effect.effect_index,
            truncate_label(&effect.effect_name, 18),
            format_db(in_peak),
            format_db(out_peak),
            format_delta_db(in_peak, out_peak),
            format_db(in_rms),
            format_db(out_rms),
            format_delta_db(in_rms, out_rms),
        );
    }

    print_spectral_table(report);
}

#[cfg(feature = "effect-meter-cli")]
fn print_bars(report: &EffectMeterReport) {
    println!(
        "input={} sample_rate={}Hz channels={} frames={} summary={:?}",
        report.input_path,
        report.sample_rate,
        report.channels,
        report.frames_processed,
        report.summary_mode
    );
    if report.effects.is_empty() {
        println!("No effects configured.");
        return;
    }

    for effect in &report.effects {
        println!("[{}] {}", effect.effect_index, effect.effect_name);
        print_level_line("in ", &effect.levels.input.peak);
        print_level_line("out", &effect.levels.output.peak);
        println!(
            "Δpk={}  Δrms={}",
            format_delta_db(
                max_dbfs(&effect.levels.input.peak),
                max_dbfs(&effect.levels.output.peak)
            ),
            format_delta_db(
                max_dbfs(&effect.levels.input.rms),
                max_dbfs(&effect.levels.output.rms)
            )
        );
        if let Some(spectral) = report
            .spectral
            .as_ref()
            .and_then(|rows| rows.get(effect.effect_index))
        {
            match spectral {
                Some(snapshot) => println!(
                    "spec o: {}",
                    spectral_graph::render_output_graph(snapshot, 24)
                ),
                None => println!(
                    "spec o: {}",
                    spectral_graph::placeholder_graph(24, "not supported")
                ),
            }
        }
        println!();
    }
}

#[cfg(feature = "effect-meter-cli")]
fn print_level_line(prefix: &str, values: &[f32]) {
    if values.is_empty() {
        println!("{}: no channels", prefix);
        return;
    }

    let labels = ["L", "R"];
    let mut parts = Vec::new();
    for (index, value) in values.iter().take(2).copied().enumerate() {
        parts.push(format!(
            "{} [{}] {} dBFS",
            labels[index],
            render_bar(value, 12),
            format_db(linear_to_dbfs_option(value)),
        ));
    }
    if values.len() > 2 {
        parts.push(format!("+{} more ch", values.len() - 2));
    }
    println!("{}: {}", prefix, parts.join(" | "));
}

#[cfg(feature = "effect-meter-cli")]
fn render_bar(linear: f32, width: usize) -> String {
    let filled = (linear.clamp(0.0, 1.0) * width as f32).round() as usize;
    let mut bar = String::with_capacity(width);
    for index in 0..width {
        bar.push(if index < filled { '#' } else { ' ' });
    }
    bar
}

#[cfg(feature = "effect-meter-cli")]
fn print_spectral_table(report: &EffectMeterReport) {
    let Some(spectral) = report.spectral.as_ref() else {
        return;
    };

    println!("spectral:");
    for (index, snapshot) in spectral.iter().enumerate() {
        match snapshot {
            Some(snapshot) => {
                let centers = snapshot
                    .input
                    .band_centers_hz
                    .iter()
                    .map(|freq| format!("{:.0}", freq))
                    .collect::<Vec<_>>()
                    .join(", ");
                let input = snapshot
                    .input
                    .bands_db
                    .iter()
                    .map(|value| format!("{value:.1}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                let output = snapshot
                    .output
                    .bands_db
                    .iter()
                    .map(|value| format!("{value:.1}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                println!("  [{}] {} Hz", index, centers);
                println!("      in : {}", input);
                println!("      out: {}", output);
            }
            None => println!("  [{}] none", index),
        }
    }
}

#[cfg(feature = "effect-meter-cli")]
fn truncate_label(value: &str, width: usize) -> String {
    if value.chars().count() <= width {
        return value.to_string();
    }
    value
        .chars()
        .take(width.saturating_sub(3))
        .collect::<String>()
        + "..."
}

#[cfg(feature = "effect-meter-cli")]
fn linear_to_dbfs_option(value: f32) -> Option<f32> {
    if value <= 0.0 {
        None
    } else {
        Some(20.0 * value.log10())
    }
}

#[cfg(feature = "effect-meter-cli")]
fn max_dbfs(values: &[f32]) -> Option<f32> {
    values
        .iter()
        .copied()
        .max_by(|left, right| left.total_cmp(right))
        .and_then(linear_to_dbfs_option)
}

#[cfg(feature = "effect-meter-cli")]
fn format_db(value: Option<f32>) -> String {
    match value {
        Some(value) => format!("{value:>5.1}"),
        None => " -inf".to_string(),
    }
}

#[cfg(feature = "effect-meter-cli")]
fn format_delta_db(before: Option<f32>, after: Option<f32>) -> String {
    match (before, after) {
        (Some(before), Some(after)) => format!("{:+5.1}", after - before),
        _ => "  n/a".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::run_meter_subcommand;

    #[test]
    fn meter_without_nested_subcommand_returns_one() {
        let args = clap::Command::new("meter").get_matches_from(["meter"]);
        let code = run_meter_subcommand(&args).expect("meter command should run");
        assert_eq!(code, 1);
    }
}
