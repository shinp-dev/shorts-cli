use std::path::{Path, PathBuf};

use clap::{Args, Parser, Subcommand, ValueEnum};
use serde::Serialize;

use crate::compiler::{self, TimeRange};
use crate::error::{Result, VedError, message};
use crate::ffmpeg::{self, OutputKind, Quality};
use crate::project::{self, Canvas, Media, Project};
use crate::timeline;

#[derive(Debug, Parser)]
#[command(
    name = "ved",
    version,
    about = "Deterministic video editing for humans and AI agents"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Create a project file.
    New {
        project: PathBuf,
        #[arg(long, value_enum, default_value_t = Preset::Vertical)]
        preset: Preset,
        #[arg(long, default_value_t = 30)]
        fps: u32,
    },
    /// Diagnose local FFmpeg tools and optional local TTS services.
    Doctor,
    /// Probe and register a media file.
    Import { project: PathBuf, file: PathBuf },
    /// Append a video clip.
    Add {
        project: PathBuf,
        media: String,
        #[command(flatten)]
        source: SourceRange,
        #[command(flatten)]
        audio: ClipAudio,
    },
    /// Insert a video at a completed-timeline time, splitting a clip if needed.
    Insert {
        project: PathBuf,
        media: String,
        #[arg(long)]
        at: f64,
        #[command(flatten)]
        source: SourceRange,
        #[command(flatten)]
        audio: ClipAudio,
    },
    /// Change a clip's source range and ripple later clips.
    Trim {
        project: PathBuf,
        #[arg(long)]
        clip: String,
        #[command(flatten)]
        source: SourceRange,
    },
    /// Ripple-delete a completed-timeline range [from, to).
    Remove {
        project: PathBuf,
        #[arg(long)]
        from: f64,
        #[arg(long)]
        to: f64,
    },
    /// Print a concise, stable timeline description.
    Describe { project: PathBuf },
    /// Render the complete timeline to MP4.
    Render {
        project: PathBuf,
        output: PathBuf,
        #[arg(long, value_enum, default_value_t = Quality::Normal)]
        quality: Quality,
        #[arg(long)]
        dry_run: bool,
    },
    /// Play all or part of the completed timeline.
    Play {
        project: PathBuf,
        #[command(flatten)]
        range: TimelineRange,
    },
    /// Export one frame at a completed-timeline time.
    Frame {
        project: PathBuf,
        time: f64,
        #[arg(default_value = "frame.png")]
        output: PathBuf,
    },
    /// Export a completed-timeline range to MP4.
    ExportRange {
        project: PathBuf,
        #[arg(long)]
        from: f64,
        #[arg(long)]
        to: f64,
        #[arg(long)]
        out: PathBuf,
        #[arg(long, value_enum, default_value_t = Quality::Preview)]
        quality: Quality,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Preset {
    Vertical,
    Horizontal,
    Square,
}

#[derive(Debug, Args)]
struct SourceRange {
    #[arg(long = "in")]
    source_in: Option<f64>,
    #[arg(long = "out")]
    source_out: Option<f64>,
}

#[derive(Debug, Args)]
struct TimelineRange {
    #[arg(long)]
    from: Option<f64>,
    #[arg(long)]
    to: Option<f64>,
}

#[derive(Debug, Args)]
struct ClipAudio {
    /// Mute this video's own audio.
    #[arg(long, conflicts_with = "video_only")]
    mute: bool,
    /// Use only this video's picture (stored canonically as mute).
    #[arg(long)]
    video_only: bool,
    /// Set this video's own audio gain (1.0 is unchanged).
    #[arg(long, default_value_t = 1.0)]
    volume: f64,
}

pub fn run() -> Result<()> {
    match Cli::parse().command {
        Command::New {
            project,
            preset,
            fps,
        } => new_project(&project, preset, fps),
        Command::Doctor => doctor(),
        Command::Import { project, file } => import(&project, &file),
        Command::Add {
            project,
            media,
            source,
            audio,
        } => edit_with_media(&project, &media, |value, media_id| {
            let id = timeline::add(value, media_id, source.source_in, source.source_out)?;
            apply_clip_audio(value, &id, audio)?;
            Ok(format!("Added clip {id}"))
        }),
        Command::Insert {
            project,
            media,
            at,
            source,
            audio,
        } => edit_with_media(&project, &media, |value, media_id| {
            let id = timeline::insert(value, media_id, at, source.source_in, source.source_out)?;
            apply_clip_audio(value, &id, audio)?;
            Ok(format!("Inserted clip {id} at {}", timestamp(at)))
        }),
        Command::Trim {
            project,
            clip,
            source,
        } => edit(&project, |value| {
            if source.source_in.is_none() && source.source_out.is_none() {
                return Err(message("trim requires --in, --out, or both"));
            }
            timeline::trim(value, &clip, source.source_in, source.source_out)?;
            Ok(format!("Trimmed clip {clip}"))
        }),
        Command::Remove { project, from, to } => edit(&project, |value| {
            timeline::remove(value, from, to)?;
            Ok(format!("Removed {} - {}", timestamp(from), timestamp(to)))
        }),
        Command::Describe { project } => describe(&project),
        Command::Render {
            project,
            output,
            quality,
            dry_run,
        } => render(&project, &output, quality, dry_run),
        Command::Play { project, range } => play(&project, range),
        Command::Frame {
            project,
            time,
            output,
        } => frame(&project, time, &output),
        Command::ExportRange {
            project,
            from,
            to,
            out,
            quality,
        } => export_range(&project, from, to, &out, quality),
    }
}

fn new_project(path: &Path, preset: Preset, fps: u32) -> Result<()> {
    if path.exists() {
        return Err(message(format!(
            "project already exists: {}",
            path.display()
        )));
    }
    if fps == 0 {
        return Err(message("--fps must be greater than zero"));
    }
    let (width, height) = match preset {
        Preset::Vertical => (1080, 1920),
        Preset::Horizontal => (1920, 1080),
        Preset::Square => (1080, 1080),
    };
    project::save(path, &Project::new(Canvas { width, height, fps }))?;
    println!(
        "Created {} ({}x{} @ {}fps)",
        path.display(),
        width,
        height,
        fps
    );
    Ok(())
}

fn doctor() -> Result<()> {
    println!("ved {}", env!("CARGO_PKG_VERSION"));
    for status in ffmpeg::doctor() {
        let marker = if status.available {
            "ok"
        } else {
            "unavailable"
        };
        let detail = status.version.or(status.detail).unwrap_or_default();
        println!("{:<12} {:<11} {}", status.name, marker, detail);
    }
    Ok(())
}

fn import(project_path: &Path, input: &Path) -> Result<()> {
    let mut value = project::load(project_path)?;
    let canonical = std::fs::canonicalize(input).map_err(|source| VedError::Io {
        path: input.to_path_buf(),
        source,
    })?;
    if !canonical.is_file() {
        return Err(message(format!("media is not a file: {}", input.display())));
    }
    let stored = stored_media_path(project_path, &canonical)?;
    if let Some(existing) = value
        .media
        .iter()
        .find(|media| media_paths_equal(&media.path, &stored))
    {
        println!("Already imported as {}: {}", existing.id, existing.path);
        return Ok(());
    }
    let (kind, probe) = ffmpeg::probe(&canonical)?;
    let id = value.next_media_id();
    value.media.push(Media {
        id: id.clone(),
        path: stored.clone(),
        kind,
        probe,
    });
    project::save(project_path, &value)?;
    println!("Imported {id}: {stored}");
    Ok(())
}

fn edit<F>(project_path: &Path, operation: F) -> Result<()>
where
    F: FnOnce(&mut Project) -> Result<String>,
{
    let mut value = project::load(project_path)?;
    let message = operation(&mut value)?;
    project::save(project_path, &value)?;
    println!("{message}");
    println!("Timeline duration: {:.3}s", timeline::duration(&value));
    Ok(())
}

fn edit_with_media<F>(project_path: &Path, media_ref: &str, operation: F) -> Result<()>
where
    F: FnOnce(&mut Project, &str) -> Result<String>,
{
    let mut value = project::load(project_path)?;
    let media_id = if let Ok(media) = value.media_ref(media_ref) {
        media.id.clone()
    } else {
        let canonical = std::fs::canonicalize(media_ref).map_err(|_| {
            message(format!(
                "media {media_ref:?} is not imported (use a media ID or imported path)"
            ))
        })?;
        let stored = stored_media_path(project_path, &canonical)?;
        value
            .media
            .iter()
            .find(|media| media_paths_equal(&media.path, &stored))
            .map(|media| media.id.clone())
            .ok_or_else(|| message(format!("media {media_ref:?} is not imported")))?
    };
    let message = operation(&mut value, &media_id)?;
    project::save(project_path, &value)?;
    println!("{message}");
    println!("Timeline duration: {:.3}s", timeline::duration(&value));
    Ok(())
}

fn apply_clip_audio(project: &mut Project, clip_id: &str, audio: ClipAudio) -> Result<()> {
    if !audio.volume.is_finite() || audio.volume < 0.0 {
        return Err(message("--volume must be a finite non-negative number"));
    }
    let clip = project
        .timeline
        .iter_mut()
        .find(|clip| clip.id == clip_id)
        .ok_or_else(|| message(format!("clip {clip_id} does not exist")))?;
    clip.mute = audio.mute || audio.video_only;
    clip.volume = if clip.mute { 1.0 } else { audio.volume };
    Ok(())
}

fn describe(project_path: &Path) -> Result<()> {
    let value = project::load(project_path)?;
    println!("Duration: {:.3}s", timeline::duration(&value));
    println!(
        "Canvas: {}x{} @ {}fps",
        value.canvas.width, value.canvas.height, value.canvas.fps
    );
    println!();
    println!("Timeline:");
    if value.timeline.is_empty() {
        println!("(empty)");
    }
    for item in timeline::resolve(&value) {
        let media = value.media_by_id(&item.clip.media_id)?;
        println!(
            "{}  {} - {}",
            item.clip.id,
            timestamp(item.timeline_start),
            timestamp(item.timeline_end)
        );
        println!(
            "    {} [{} - {}]",
            media.path,
            timestamp(item.clip.source_in),
            timestamp(item.clip.source_out)
        );
    }
    Ok(())
}

fn render(project_path: &Path, output: &Path, quality: Quality, dry_run: bool) -> Result<()> {
    let value = project::load(project_path)?;
    let plan = compiler::compile(project_path, &value, None)?;
    if dry_run {
        print_dry_run(&plan, output, OutputKind::Mp4(quality))?;
        return Ok(());
    }
    ffmpeg::run(&plan, output, OutputKind::Mp4(quality))?;
    println!("Rendered {} ({:.3}s)", output.display(), plan.duration);
    Ok(())
}

fn play(project_path: &Path, range: TimelineRange) -> Result<()> {
    let value = project::load(project_path)?;
    let total = timeline::duration(&value);
    let range = match (range.from, range.to) {
        (None, None) => None,
        (from, to) => Some(TimeRange {
            start: from.unwrap_or(0.0),
            end: to.unwrap_or(total),
        }),
    };
    let plan = compiler::compile(project_path, &value, range)?;
    ffmpeg::play(&plan)
}

fn frame(project_path: &Path, time: f64, output: &Path) -> Result<()> {
    let value = project::load(project_path)?;
    let total = timeline::duration(&value);
    if !time.is_finite() || time < 0.0 || time >= total {
        return Err(message(format!("frame time must be in [0, {:.3})", total)));
    }
    let sample_duration = (1.0 / value.canvas.fps as f64).max(0.05);
    let plan = compiler::compile(
        project_path,
        &value,
        Some(TimeRange {
            start: time,
            end: (time + sample_duration).min(total),
        }),
    )?;
    ffmpeg::run(&plan, output, OutputKind::Frame)?;
    println!("Wrote frame at {} to {}", timestamp(time), output.display());
    Ok(())
}

fn export_range(
    project_path: &Path,
    from: f64,
    to: f64,
    output: &Path,
    quality: Quality,
) -> Result<()> {
    let value = project::load(project_path)?;
    let plan = compiler::compile(
        project_path,
        &value,
        Some(TimeRange {
            start: from,
            end: to,
        }),
    )?;
    ffmpeg::run(&plan, output, OutputKind::Mp4(quality))?;
    println!(
        "Exported {} - {} to {}",
        timestamp(from),
        timestamp(to),
        output.display()
    );
    Ok(())
}

fn stored_media_path(project_path: &Path, canonical: &Path) -> Result<String> {
    let parent = project_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let base = std::fs::canonicalize(parent).map_err(|source| VedError::Io {
        path: parent.to_path_buf(),
        source,
    })?;
    let chosen = canonical.strip_prefix(&base).unwrap_or(canonical);
    Ok(chosen.to_string_lossy().replace('\\', "/"))
}

fn media_paths_equal(left: &str, right: &str) -> bool {
    if cfg!(windows) {
        left.eq_ignore_ascii_case(right)
    } else {
        left == right
    }
}

fn print_dry_run(
    plan: &crate::compiler::RenderPlan,
    output: &Path,
    kind: OutputKind,
) -> Result<()> {
    #[derive(Serialize)]
    struct DryRun<'a> {
        ffmpeg: &'static str,
        output: String,
        plan: &'a crate::compiler::RenderPlan,
        args: Vec<String>,
    }
    let data = DryRun {
        ffmpeg: "ffmpeg",
        output: output.to_string_lossy().into_owned(),
        plan,
        args: ffmpeg::build_args(plan, output, kind)
            .into_iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect(),
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&data).map_err(|error| message(error.to_string()))?
    );
    Ok(())
}

fn timestamp(seconds: f64) -> String {
    let total_ms = (seconds * 1000.0).round() as u64;
    let minutes = total_ms / 60_000;
    let seconds = (total_ms % 60_000) / 1000;
    let millis = total_ms % 1000;
    format!("{minutes:02}:{seconds:02}.{millis:03}")
}
