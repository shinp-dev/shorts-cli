mod cli;
mod compiler;
mod error;
mod ffmpeg;
mod history;
mod phase3_cli;
mod project;
mod timeline;
mod tts;

fn main() {
    let result = if phase3_cli::is_phase3_command() {
        phase3_cli::run()
    } else {
        cli::run()
    };
    if let Err(error) = result {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}
