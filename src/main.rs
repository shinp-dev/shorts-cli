mod audio_analysis;
mod audio_sync;
mod cli;
mod compiler;
mod error;
mod ffmpeg;
mod hashing;
mod history;
mod phase4;
mod project;
mod timeline;

fn main() {
    // clap builds the complete command graph while parsing. Keep that work off the
    // comparatively small Windows main-thread stack as the agent-facing CLI grows.
    let status = std::thread::Builder::new()
        .name("ved-cli".into())
        .stack_size(8 * 1024 * 1024)
        .spawn(|| match cli::run() {
            Ok(()) => 0,
            Err(error) => {
                cli::print_error(&error);
                1
            }
        })
        .expect("could not start ved CLI thread")
        .join()
        .unwrap_or(1);
    if status != 0 {
        std::process::exit(status);
    }
}
