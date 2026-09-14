mod cli;
mod compiler;
mod error;
mod ffmpeg;
mod project;
mod timeline;

fn main() {
    if let Err(error) = cli::run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}
