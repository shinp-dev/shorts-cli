mod cli;
mod compiler;
mod error;
mod ffmpeg;
mod history;
mod project;
mod timeline;
mod tts;

fn main() {
    if let Err(error) = cli::run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}
