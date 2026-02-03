//! # Prot Play
//!
//! A command-line audio player for the Prot audio format.
use std::{
    io::{self, Write},
    sync::{Arc, Mutex},
    thread::sleep,
    time::Duration,
};

use clap::{Arg, ArgMatches};
use log::error;
use proteus_lib::{player, reporter::Report, test_data};
use rand::Rng;
use symphonia::core::errors::Result;

/// The main entry point of the application.
fn main() {
    let args = clap::Command::new("Prot Play")
        .version("1.0")
        .author("Adam Howard <adam.thomas.howard@gmail.com>")
        .about("Play Prot audio")
        .arg(
            Arg::new("seek")
                .long("seek")
                .short('s')
                .value_name("TIME")
                .help("Seek to the given time in seconds")
                .conflicts_with_all(&["verify", "decode-only", "verify-only", "probe-only"]),
        )
        .arg(
            Arg::new("GAIN")
                .long("gain")
                .short('g')
                .value_name("GAIN")
                .default_value("70")
                // .min(0)
                // .max(100)
                .help("The playback gain"),
        )
        .arg(
            Arg::new("decode-only")
                .long("decode-only")
                .help("Decode, but do not play the audio")
                .conflicts_with_all(&["probe-only", "verify-only", "verify"]),
        )
        .arg(
            Arg::new("probe-only")
                .long("probe-only")
                .help("Only probe the input for metadata")
                .conflicts_with_all(&["decode-only", "verify-only"]),
        )
        .arg(
            Arg::new("verify-only")
                .long("verify-only")
                .help("Verify the decoded audio is valid, but do not play the audio")
                .conflicts_with_all(&["verify"]),
        )
        .arg(
            Arg::new("verify")
                .long("verify")
                .short('v')
                .help("Verify the decoded audio is valid during playback"),
        )
        .arg(
            Arg::new("no-progress")
                .long("no-progress")
                .help("Do not display playback progress"),
        )
        .arg(
            Arg::new("no-gapless")
                .long("no-gapless")
                .help("Disable gapless decoding and playback"),
        )
        .arg(Arg::new("debug").short('d').help("Show debug output"))
        .arg(
            Arg::new("INPUT")
                .help("The input file path, or - to use standard input")
                .required(true)
                .index(1),
        )
        .get_matches();

    // For any error, return an exit code -1. Otherwise return the exit code provided.
    let code = match run(&args) {
        Ok(code) => code,
        Err(err) => {
            error!("{}", err.to_string().to_lowercase());
            -1
        }
    };

    std::process::exit(code)
}

/// Formats a given time in seconds into a HH:MM:SS format.
///
/// # Arguments
///
/// * `time` - The time in seconds to format.
fn format_time(time: f64) -> String {
    // Seconds rounded up
    let seconds = (time / 1000.0).ceil() as u32;
    let minutes = seconds / 60;
    let seconds = seconds % 60;
    let hours = minutes / 60;
    let minutes = minutes % 60;

    format!("{:02}:{:02}:{:02}", hours, minutes, seconds)
}

/// The main logic of the application.
///
/// # Arguments
///
/// * `args` - The command-line arguments.
fn run(args: &ArgMatches) -> Result<i32> {
    let file_path = args.get_one::<String>("INPUT").unwrap().clone();
    let gain = args
        .get_one::<String>("GAIN")
        .unwrap()
        .parse::<f32>()
        .unwrap()
        .clone();

    // If file is not a .mka file, return an error
    if !(file_path.ends_with(".prot") || file_path.ends_with(".mka")) {
        panic!("File is not a .prot file");
    }

    // let mut player = player::Player::new(&file_path);

    // let info = info::Info::new(file_path);

    let test_data = test_data::TestData::new();
    let mut player = player::Player::new_from_file_paths(&test_data.wavs);
    println!("Test info: {:?}", player.info);

    // let test_info = info::Info::new_from_file_paths(test_data.wavs);
    // println!("Duration: {}", format_time(info.get_duration(0).unwrap() * 1000.0));

    player.play();

    player.set_volume(gain / 100.0);

    let mut loop_iteration = 0;

    let reporting_function = |Report {
                                  time,
                                  duration,
                                  playing,
                                  ..
                              }| {
        let state = if playing { "Playing" } else { "Paused " };
        let current = format_time(time * 1000.0);
        let total = format_time(duration * 1000.0);
        print!("\r{}  {} / {}", state, current, total);
        let _ = io::stdout().flush();
    };

    player.set_reporting(
        Arc::new(Mutex::new(reporting_function)),
        Duration::from_millis(100),
    );

    // player.pause();

    while !player.is_finished() {
        // player.shuffle();
        // println!("Shuffling");
        // match loop_iteration {
        //     10 => player.shuffle(),
        //     20 => player.shuffle(),
        //     30 => player.shuffle(),
        //     40 => player.shuffle(),
        //     50 => player.shuffle(),
        //     _ => {}
        // }
        // match loop_iteration {
        //     10 => {
        //         println!(
        //             "Pausing playback at {}",
        //             format_time(player.get_time() * 1000.0)
        //         );
        //         player.pause();
        //     }
        //     // 20 => {
        //     //     println!("Resuming playback at {}", format_time(player.get_time() * 1000.0));
        //     //     player.play();
        //     // },
        //     // 60 => {
        //     //     println!("Pausing playback at {}", format_time(player.get_time() * 1000.0));
        //     //     player.pause();
        //     // },
        //     20 => {
        //         println!("Seeking to 10.0 seconds");
        //         player.seek(10.0);
        //     }
        //     30 => {
        //         println!("Seeking to 2.0 seconds");
        //         player.seek(2.0);
        //     }
        //     50 => {
        //         println!("Seeking to 6.0 seconds");
        //         player.seek(6.0);
        //         // Set volume to random number between 0.0 and 1.0
        //         let volume = rand::thread_rng().gen_range(0.3..1.0);
        //         println!("Setting volume to {}", volume);
        //         player.set_volume(volume);
        //         println!("Get volume: {}", player.get_volume());
        //         player.play();
        //         println!(
        //             "Starting playback at {}",
        //             format_time(player.get_time() * 1000.0)
        //         );
        //     }
        //     _ => {}
        // }

        // loop_iteration += 1;

        // println!(
        //     "{} / {} ({}) - Timer: {}",
        //     format_time(player.get_time() * 1000.0),
        //     format_time(player.get_duration() * 1000.0),
        //     player.get_time(),
        //     timer.get_time().as_secs_f64()
        // );
        sleep(Duration::from_secs(1));
        // sleep(Duration::from_millis(100));
    }

    println!();
    Ok(0)
}
