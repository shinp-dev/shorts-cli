mod audio_analysis;
mod audio_sync;
mod cli;
mod compiler;
mod error;
mod ffmpeg;
mod hashing;
mod history;
mod phase3_cli;
mod phase4;
mod project;
mod timeline;
mod tts;

fn main() {
    // clap builds the complete command graph while parsing. Keep that work off the
    // comparatively small Windows main-thread stack as the agent-facing CLI grows.
    let status = std::thread::Builder::new()
        .name("ved-cli".into())
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            let phase3 = phase3_cli::is_phase3_command();
            let result = if phase3 {
                phase3_cli::run()
            } else {
                cli::run()
            };
            match result {
                Ok(()) => 0,
                Err(error) => {
                    if phase3 {
                        eprintln!("error: {error}");
                    } else {
                        cli::print_error(&error);
                    }
                    1
                }
            }
        })
        .expect("could not start ved CLI thread")
        .join()
        .unwrap_or(1);
    if status != 0 {
        std::process::exit(status);
    }
}
