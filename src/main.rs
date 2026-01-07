use clap::Parser;
use log::error;
use proteus_audio::{info, player, prot};
use rand::Rng;
use serde_json::Number;
use symphonia::core::errors::Result;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Seek to the given time in seconds
    #[arg(short, long, value_name = "TIME", conflicts_with_all = ["verify", "decode_only", "verify_only", "probe_only"])]
    seek: Option<String>,

    /// The playback gain
    #[arg(short, long, default_value_t = 70.0, value_name = "GAIN")]
    gain: f32,

    /// Decode, but do not play the audio
    #[arg(long, conflicts_with_all = ["probe_only", "verify_only", "verify"])]
    decode_only: bool,

    /// Only probe the input for metadata
    #[arg(long, conflicts_with_all = ["decode_only", "verify_only"])]
    probe_only: bool,

    /// Verify the decoded audio is valid, but do not play the audio
    #[arg(long, conflicts_with_all = ["verify"])]
    verify_only: bool,

    /// Verify the decoded audio is valid during playback
    #[arg(short, long)]
    verify: bool,

    /// Do not display playback progress
    #[arg(long)]
    no_progress: bool,

    /// Disable gapless decoding and playback
    #[arg(long)]
    no_gapless: bool,

    /// Show debug output
    #[arg(short, long)]
    debug: bool,

    /// The input file path, or - to use standard input
    #[arg(required = true)]
    input: String,
}

fn main() {
    let args = Cli::parse();

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

fn format_time(time: f64) -> String {
    // Seconds rounded up
    let seconds = (time / 1000.0).ceil() as u32;
    let minutes = seconds / 60;
    let seconds = seconds % 60;
    let hours = minutes / 60;
    let minutes = minutes % 60;

    format!("{:02}:{:02}:{:02}", hours, minutes, seconds)
}

fn run(args: &Cli) -> Result<i32> {
    let file_path = &args.input;
    let gain = args.gain;

    // If file is not a .mka file, return an error
    if !(file_path.ends_with(".prot") || file_path.ends_with(".mka")) {
        panic!("File is not a .prot file");
    }

    let mut player = player::Player::new(file_path);

    let info = info::Info::new(file_path.clone());
    println!("Files: {:?}", info.file_paths);
    println!("Duration: {:?}", info.duration_map);
    println!("Channels: {:?}", info.channels);
    // println!("Duration: {}", format_time(info.get_duration(0).unwrap() * 1000.0));

    player.play();

    player.set_volume(gain / 100.0);

    let mut loop_iteration = 0;
    // while !player.is_finished() {
    //     if loop_iteration > 0 {
    //         // println!("Get time: {}", player.get_time());
    //         println!("Refreshing tracks at {}", format_time(player.get_time() * 1000.0));
    //         println!("Duration: {}", format_time(player.get_duration() * 1000.0));
    //         // Measure the time it takes to refresh tracks
    //         let start = std::time::Instant::now();
    //         player.refresh_tracks();
    //         let duration = start.elapsed();
    //         println!("Refreshed tracks in {}ms", duration.as_millis());
    //         // println!("Get time: {}", player.get_time());
    //     }

    //     // if loop_iteration > 0 {
    //     //     if !player.is_paused() {
    //     //         player.pause();
    //     //     } else {
    //     //         // Set volume to random number between 0.0 and 1.0
    //     //         let volume = rand::thread_rng().gen_range(0.3..1.0);
    //     //         println!("Setting volume to {}", volume);
    //     //         player.set_volume(volume);
    //     //         println!("Get volume: {}", player.get_volume());
    //     //         player.play();
    //     //         println!("Starting playback at {}", format_time(player.get_time() * 1000.0));
    //     //     }
    //     // }
    //     loop_iteration += 1;

    //     std::thread::sleep(std::time::Duration::from_secs(2));
    // }

    while !player.is_finished() {
        println!(
            "{} / {}",
            format_time(player.get_time() * 1000.0),
            format_time(player.get_duration() * 1000.0)
        );
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    Ok(0)
}
