use std::path::{Path, PathBuf};

use clap::{Args, Parser, Subcommand, ValueEnum};
use serde::Serialize;

use crate::compiler::{self, TimeRange};
use crate::error::{Result, VedError, message};
use crate::ffmpeg::{self, OutputKind, Quality};
use crate::history;
use crate::project::{
    self, AudioClip, Axis, Canvas, Coordinate, Dimension, ImageOverlay, Media, MediaKind, Outline,
    Position, Project, TextOverlay, validate_color,
};
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
    /// Create a new project.
    New {
        project: PathBuf,
        #[arg(long, value_enum, default_value_t = Preset::Vertical)]
        preset: Preset,
        #[arg(long, default_value_t = 30)]
        fps: u32,
    },
    /// Diagnose FFmpeg tools and optional local services.
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
    /// Insert video at a completed-timeline time.
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
    /// Change a named clip's source range.
    Trim {
        project: PathBuf,
        #[arg(long)]
        clip: String,
        #[command(flatten)]
        source: SourceRange,
    },
    /// Ripple-delete a completed-timeline range.
    Remove {
        project: PathBuf,
        #[arg(long)]
        from: f64,
        #[arg(long)]
        to: f64,
    },
    /// Change video and attached-audio playback speed.
    Speed {
        project: PathBuf,
        #[arg(long)]
        clip: String,
        #[arg(long)]
        rate: f64,
    },
    /// Add a text overlay in completed-timeline time.
    TextAdd {
        project: PathBuf,
        #[arg(long)]
        text: String,
        #[arg(long)]
        from: f64,
        #[arg(long)]
        to: f64,
        #[command(flatten)]
        position: PositionArgs,
        #[arg(long)]
        font: Option<String>,
        #[arg(long, default_value_t = 64)]
        font_size: u32,
        #[arg(long, default_value = "white")]
        color: String,
        #[arg(long)]
        outline_color: Option<String>,
        #[arg(long, default_value_t = 3)]
        outline_width: u32,
        #[arg(long)]
        background: Option<String>,
        #[arg(long, default_value_t = 1.0)]
        opacity: f64,
    },
    /// Change a text overlay.
    TextSet {
        project: PathBuf,
        id: String,
        #[arg(long)]
        text: Option<String>,
        #[arg(long = "from")]
        start: Option<f64>,
        #[arg(long = "to")]
        end: Option<f64>,
        #[command(flatten)]
        position: PositionArgs,
        #[arg(long)]
        font: Option<String>,
        #[arg(long, conflicts_with = "font")]
        clear_font: bool,
        #[arg(long)]
        font_size: Option<u32>,
        #[arg(long)]
        color: Option<String>,
        #[arg(long)]
        outline_color: Option<String>,
        #[arg(long)]
        outline_width: Option<u32>,
        #[arg(long, conflicts_with_all = ["outline_color", "outline_width"])]
        clear_outline: bool,
        #[arg(long)]
        background: Option<String>,
        #[arg(long, conflicts_with = "background")]
        clear_background: bool,
        #[arg(long)]
        opacity: Option<f64>,
    },
    /// Remove a text overlay.
    TextRemove { project: PathBuf, id: String },
    /// Add an imported image overlay.
    ImageAdd {
        project: PathBuf,
        media: String,
        #[arg(long)]
        from: f64,
        #[arg(long)]
        to: f64,
        #[command(flatten)]
        position: PositionArgs,
        #[arg(long)]
        width: Option<String>,
        #[arg(long)]
        height: Option<String>,
        #[arg(long, default_value_t = 1.0)]
        opacity: f64,
    },
    /// Change an image overlay.
    ImageSet {
        project: PathBuf,
        id: String,
        #[arg(long = "from")]
        start: Option<f64>,
        #[arg(long = "to")]
        end: Option<f64>,
        #[command(flatten)]
        position: PositionArgs,
        #[arg(long)]
        width: Option<String>,
        #[arg(long, conflicts_with = "width")]
        clear_width: bool,
        #[arg(long)]
        height: Option<String>,
        #[arg(long, conflicts_with = "height")]
        clear_height: bool,
        #[arg(long)]
        opacity: Option<f64>,
    },
    /// Remove an image overlay.
    ImageRemove { project: PathBuf, id: String },
    /// Add independent audio such as BGM or a sound effect.
    AudioAdd {
        project: PathBuf,
        media: String,
        #[arg(long)]
        at: f64,
        #[command(flatten)]
        source: SourceRange,
        #[arg(long, default_value_t = 1.0)]
        volume: f64,
        #[arg(long, default_value_t = 0.0)]
        fade_in: f64,
        #[arg(long, default_value_t = 0.0)]
        fade_out: f64,
        #[arg(long)]
        mute: bool,
    },
    /// Remove an independent audio clip.
    AudioRemove { project: PathBuf, id: String },
    /// Set volume for a video clip or independent audio clip.
    Volume {
        project: PathBuf,
        #[command(flatten)]
        target: AudioTarget,
        #[arg(long)]
        value: f64,
    },
    /// Mute or unmute a video clip or independent audio clip.
    Mute {
        project: PathBuf,
        #[command(flatten)]
        target: AudioTarget,
        #[arg(long)]
        off: bool,
    },
    /// Set fade-in duration for independent audio.
    FadeIn {
        project: PathBuf,
        #[arg(long)]
        audio: String,
        #[arg(long)]
        duration: f64,
    },
    /// Set fade-out duration for independent audio.
    FadeOut {
        project: PathBuf,
        #[arg(long)]
        audio: String,
        #[arg(long)]
        duration: f64,
    },
    /// Restore the previous whole-project snapshot.
    Undo { project: PathBuf },
    /// Reapply the most recently undone project snapshot.
    Redo { project: PathBuf },
    /// Print a concise resolved project description.
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
        #[arg(long)]
        dry_run: bool,
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
    #[arg(long, conflicts_with = "video_only")]
    mute: bool,
    #[arg(long)]
    video_only: bool,
    #[arg(long, default_value_t = 1.0)]
    volume: f64,
}

#[derive(Debug, Args)]
struct PositionArgs {
    #[arg(long, conflicts_with = "position", requires = "y")]
    x: Option<String>,
    #[arg(long, conflicts_with = "position", requires = "x")]
    y: Option<String>,
    #[arg(long)]
    position: Option<String>,
}

#[derive(Debug, Args)]
#[group(required = true, multiple = false)]
struct AudioTarget {
    #[arg(long)]
    clip: Option<String>,
    #[arg(long)]
    audio: Option<String>,
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
        Command::Speed {
            project,
            clip,
            rate,
        } => edit(&project, |value| {
            timeline::speed(value, &clip, rate)?;
            Ok(format!("Set clip {clip} speed to {rate}x"))
        }),
        Command::TextAdd {
            project,
            text,
            from,
            to,
            position,
            font,
            font_size,
            color,
            outline_color,
            outline_width,
            background,
            opacity,
        } => {
            let font = normalize_font(&project, font)?;
            edit(&project, |value| {
                validate_color(&color)?;
                if let Some(value) = &background {
                    validate_color(value)?;
                }
                let outline = outline_color.map(|color| Outline {
                    color,
                    width: outline_width,
                });
                if let Some(value) = &outline {
                    validate_color(&value.color)?;
                }
                let id = value.next_text_id();
                value.text_overlays.push(TextOverlay {
                    id: id.clone(),
                    text,
                    start: from,
                    end: to,
                    position: resolve_position(position, true)?.unwrap(),
                    font,
                    font_size,
                    color,
                    outline,
                    background,
                    opacity,
                });
                Ok(format!("Added text overlay {id}"))
            })
        }
        Command::TextSet {
            project,
            id,
            text,
            start,
            end,
            position,
            font,
            clear_font,
            font_size,
            color,
            outline_color,
            outline_width,
            clear_outline,
            background,
            clear_background,
            opacity,
        } => {
            let font = normalize_font(&project, font)?;
            edit(&project, |value| {
                let new_position = resolve_position(position, false)?;
                let overlay = value
                    .text_overlays
                    .iter_mut()
                    .find(|item| item.id == id)
                    .ok_or_else(|| message(format!("text overlay {id} does not exist")))?;
                let mut changed = false;
                set_option(&mut overlay.text, text, &mut changed);
                set_option(&mut overlay.start, start, &mut changed);
                set_option(&mut overlay.end, end, &mut changed);
                set_option(&mut overlay.font_size, font_size, &mut changed);
                set_option(&mut overlay.color, color, &mut changed);
                set_option(&mut overlay.opacity, opacity, &mut changed);
                if let Some(value) = font {
                    overlay.font = Some(value);
                    changed = true;
                }
                if clear_font {
                    overlay.font = None;
                    changed = true;
                }
                if let Some(value) = background {
                    overlay.background = Some(value);
                    changed = true;
                }
                if clear_background {
                    overlay.background = None;
                    changed = true;
                }
                if let Some(value) = new_position {
                    overlay.position = value;
                    changed = true;
                }
                if clear_outline {
                    overlay.outline = None;
                    changed = true;
                }
                if outline_color.is_some() || outline_width.is_some() {
                    let existing = overlay.outline.clone().unwrap_or(Outline {
                        color: "black".into(),
                        width: 3,
                    });
                    overlay.outline = Some(Outline {
                        color: outline_color.unwrap_or(existing.color),
                        width: outline_width.unwrap_or(existing.width),
                    });
                    changed = true;
                }
                if !changed {
                    return Err(message("text-set requires at least one change"));
                }
                Ok(format!("Updated text overlay {id}"))
            })
        }
        Command::TextRemove { project, id } => edit(&project, |value| {
            remove_by_id(
                &mut value.text_overlays,
                &id,
                |item| &item.id,
                "text overlay",
            )?;
            Ok(format!("Removed text overlay {id}"))
        }),
        Command::ImageAdd {
            project,
            media,
            from,
            to,
            position,
            width,
            height,
            opacity,
        } => edit_with_media(&project, &media, |value, media_id| {
            if value.media_by_id(media_id)?.kind != MediaKind::Image {
                return Err(message("image-add requires imported image media"));
            }
            let id = value.next_image_id();
            value.image_overlays.push(ImageOverlay {
                id: id.clone(),
                media_id: media_id.into(),
                start: from,
                end: to,
                position: resolve_position(position, true)?.unwrap(),
                width: parse_dimension(width)?,
                height: parse_dimension(height)?,
                opacity,
            });
            Ok(format!("Added image overlay {id}"))
        }),
        Command::ImageSet {
            project,
            id,
            start,
            end,
            position,
            width,
            clear_width,
            height,
            clear_height,
            opacity,
        } => edit(&project, |value| {
            let new_position = resolve_position(position, false)?;
            let overlay = value
                .image_overlays
                .iter_mut()
                .find(|item| item.id == id)
                .ok_or_else(|| message(format!("image overlay {id} does not exist")))?;
            let mut changed = false;
            set_option(&mut overlay.start, start, &mut changed);
            set_option(&mut overlay.end, end, &mut changed);
            set_option(&mut overlay.opacity, opacity, &mut changed);
            if let Some(value) = new_position {
                overlay.position = value;
                changed = true;
            }
            if let Some(value) = width {
                overlay.width = Some(Dimension::parse(&value)?);
                changed = true;
            }
            if clear_width {
                overlay.width = None;
                changed = true;
            }
            if let Some(value) = height {
                overlay.height = Some(Dimension::parse(&value)?);
                changed = true;
            }
            if clear_height {
                overlay.height = None;
                changed = true;
            }
            if !changed {
                return Err(message("image-set requires at least one change"));
            }
            Ok(format!("Updated image overlay {id}"))
        }),
        Command::ImageRemove { project, id } => edit(&project, |value| {
            remove_by_id(
                &mut value.image_overlays,
                &id,
                |item| &item.id,
                "image overlay",
            )?;
            Ok(format!("Removed image overlay {id}"))
        }),
        Command::AudioAdd {
            project,
            media,
            at,
            source,
            volume,
            fade_in,
            fade_out,
            mute,
        } => edit_with_media(&project, &media, |value, media_id| {
            let media = value.media_by_id(media_id)?;
            if media.kind != MediaKind::Audio && !media.probe.has_audio {
                return Err(message("audio-add requires media with audio"));
            }
            let id = value.next_audio_id();
            value.audio_clips.push(AudioClip {
                id: id.clone(),
                media_id: media_id.into(),
                start: at,
                source_in: source.source_in.unwrap_or(0.0),
                source_out: source.source_out,
                volume,
                mute,
                fade_in,
                fade_out,
            });
            Ok(format!("Added audio clip {id}"))
        }),
        Command::AudioRemove { project, id } => edit(&project, |value| {
            remove_by_id(&mut value.audio_clips, &id, |item| &item.id, "audio clip")?;
            Ok(format!("Removed audio clip {id}"))
        }),
        Command::Volume {
            project,
            target,
            value: volume,
        } => edit(&project, |project| {
            if !volume.is_finite() || volume < 0.0 {
                return Err(message("volume must be a finite non-negative number"));
            }
            let id = update_audio_target(project, &target, |_mute, value| {
                *value = volume;
            })?;
            Ok(format!("Set {id} volume to {volume}"))
        }),
        Command::Mute {
            project,
            target,
            off,
        } => edit(&project, |project| {
            let id = update_audio_target(project, &target, |mute, _volume| {
                *mute = !off;
            })?;
            Ok(format!("{} {id}", if off { "Unmuted" } else { "Muted" }))
        }),
        Command::FadeIn {
            project,
            audio,
            duration,
        } => set_fade(&project, &audio, duration, true),
        Command::FadeOut {
            project,
            audio,
            duration,
        } => set_fade(&project, &audio, duration, false),
        Command::Undo { project } => {
            history::undo(&project)?;
            println!("Undid last edit");
            Ok(())
        }
        Command::Redo { project } => {
            history::redo(&project)?;
            println!("Redid last edit");
            Ok(())
        }
        Command::Describe { project } => describe(&project),
        Command::Render {
            project,
            output,
            quality,
            dry_run,
        } => render(&project, &output, quality, dry_run),
        Command::Play {
            project,
            range,
            dry_run,
        } => play(&project, range, dry_run),
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
        println!(
            "{:<12} {:<11} {}",
            status.name,
            if status.available {
                "ok"
            } else {
                "unavailable"
            },
            status.version.or(status.detail).unwrap_or_default()
        );
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
    let before = value.clone();
    let (kind, probe) = ffmpeg::probe(&canonical)?;
    let id = value.next_media_id();
    value.media.push(Media {
        id: id.clone(),
        path: stored.clone(),
        kind,
        probe,
    });
    save_edit(project_path, &before, &value)?;
    println!("Imported {id}: {stored}");
    Ok(())
}

fn edit<F>(project_path: &Path, operation: F) -> Result<()>
where
    F: FnOnce(&mut Project) -> Result<String>,
{
    let mut value = project::load(project_path)?;
    let before = value.clone();
    let output = operation(&mut value)?;
    value.validate()?;
    save_edit(project_path, &before, &value)?;
    println!("{output}");
    println!("Timeline duration: {:.3}s", timeline::duration(&value));
    Ok(())
}

fn edit_with_media<F>(project_path: &Path, media_ref: &str, operation: F) -> Result<()>
where
    F: FnOnce(&mut Project, &str) -> Result<String>,
{
    let mut value = project::load(project_path)?;
    let before = value.clone();
    let media_id = resolve_media_id(project_path, &value, media_ref)?;
    let output = operation(&mut value, &media_id)?;
    value.validate()?;
    save_edit(project_path, &before, &value)?;
    println!("{output}");
    println!("Timeline duration: {:.3}s", timeline::duration(&value));
    Ok(())
}

fn save_edit(path: &Path, before: &Project, after: &Project) -> Result<()> {
    history::record_before_edit(path, before)?;
    project::save(path, after)
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
    clip.volume = audio.volume;
    Ok(())
}

fn resolve_position(args: PositionArgs, use_default: bool) -> Result<Option<Position>> {
    if let Some(value) = args.position {
        return Ok(Some(named_position(&value)?));
    }
    match (args.x, args.y) {
        (Some(x), Some(y)) => Ok(Some(Position {
            x: Coordinate::parse(&x, Axis::X)?,
            y: Coordinate::parse(&y, Axis::Y)?,
        })),
        (None, None) if use_default => Ok(Some(Position::center())),
        (None, None) => Ok(None),
        _ => Err(message("--x and --y must be supplied together")),
    }
}

fn named_position(value: &str) -> Result<Position> {
    let normalized = value.trim().to_ascii_lowercase();
    let (y, x) = match normalized.as_str() {
        "top-left" => ("top", "left"),
        "top-center" => ("top", "center"),
        "top-right" => ("top", "right"),
        "center-left" => ("center", "left"),
        "center" | "center-center" => ("center", "center"),
        "center-right" => ("center", "right"),
        "bottom-left" => ("bottom", "left"),
        "bottom-center" => ("bottom", "center"),
        "bottom-right" => ("bottom", "right"),
        _ => return Err(message(format!("invalid named position {value:?}"))),
    };
    Ok(Position {
        x: Coordinate::parse(x, Axis::X)?,
        y: Coordinate::parse(y, Axis::Y)?,
    })
}

fn parse_dimension(value: Option<String>) -> Result<Option<Dimension>> {
    value.map(|value| Dimension::parse(&value)).transpose()
}

fn normalize_font(project_path: &Path, font: Option<String>) -> Result<Option<String>> {
    let Some(font) = font else {
        return Ok(None);
    };
    let path = Path::new(&font);
    if !path.is_file() {
        return Ok(Some(font));
    }
    let canonical = std::fs::canonicalize(path).map_err(|source| VedError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(Some(stored_media_path(project_path, &canonical)?))
}

fn update_audio_target<F>(
    project: &mut Project,
    target: &AudioTarget,
    operation: F,
) -> Result<String>
where
    F: FnOnce(&mut bool, &mut f64),
{
    if let Some(id) = &target.clip {
        let clip = project
            .timeline
            .iter_mut()
            .find(|item| item.id == *id)
            .ok_or_else(|| message(format!("clip {id} does not exist")))?;
        operation(&mut clip.mute, &mut clip.volume);
        return Ok(format!("clip {id}"));
    }
    let id = target.audio.as_ref().unwrap();
    let audio = project
        .audio_clips
        .iter_mut()
        .find(|item| item.id == *id)
        .ok_or_else(|| message(format!("audio clip {id} does not exist")))?;
    operation(&mut audio.mute, &mut audio.volume);
    Ok(format!("audio clip {id}"))
}

fn set_fade(project_path: &Path, id: &str, duration: f64, fade_in: bool) -> Result<()> {
    edit(project_path, |project| {
        let audio = project
            .audio_clips
            .iter_mut()
            .find(|item| item.id == id)
            .ok_or_else(|| message(format!("audio clip {id} does not exist")))?;
        if fade_in {
            audio.fade_in = duration;
        } else {
            audio.fade_out = duration;
        }
        Ok(format!(
            "Set {id} {} to {duration}s",
            if fade_in { "fade-in" } else { "fade-out" }
        ))
    })
}

fn remove_by_id<T, F>(items: &mut Vec<T>, id: &str, get_id: F, kind: &str) -> Result<()>
where
    F: Fn(&T) -> &str,
{
    let index = items
        .iter()
        .position(|item| get_id(item) == id)
        .ok_or_else(|| message(format!("{kind} {id} does not exist")))?;
    items.remove(index);
    Ok(())
}

fn set_option<T>(target: &mut T, value: Option<T>, changed: &mut bool) {
    if let Some(value) = value {
        *target = value;
        *changed = true;
    }
}

fn describe(project_path: &Path) -> Result<()> {
    let value = project::load(project_path)?;
    println!("Duration: {:.3}s", timeline::duration(&value));
    println!(
        "Canvas: {}x{} @ {}fps",
        value.canvas.width, value.canvas.height, value.canvas.fps
    );
    println!("\nTimeline:");
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
            "    {} [{} - {}] speed={}x volume={}{}",
            media.path,
            timestamp(item.clip.source_in),
            timestamp(item.clip.source_out),
            item.clip.speed,
            item.clip.volume,
            if item.clip.mute { " mute" } else { "" }
        );
    }
    println!("\nText:");
    if value.text_overlays.is_empty() {
        println!("(none)");
    }
    for item in &value.text_overlays {
        println!(
            "{}  {} - {}\n    {:?} x={} y={}",
            item.id,
            timestamp(item.start),
            timestamp(item.end),
            item.text,
            item.position.x.0,
            item.position.y.0
        );
    }
    println!("\nImages:");
    if value.image_overlays.is_empty() {
        println!("(none)");
    }
    for item in &value.image_overlays {
        println!(
            "{}  {} - {}\n    {} x={} y={}",
            item.id,
            timestamp(item.start),
            timestamp(item.end),
            value.media_by_id(&item.media_id)?.path,
            item.position.x.0,
            item.position.y.0
        );
    }
    println!("\nAudio:");
    if value.audio_clips.is_empty() {
        println!("(none)");
    }
    for item in &value.audio_clips {
        println!(
            "{}  at {}\n    {} volume={} fade-in={} fade-out={}{}",
            item.id,
            timestamp(item.start),
            value.media_by_id(&item.media_id)?.path,
            item.volume,
            item.fade_in,
            item.fade_out,
            if item.mute { " mute" } else { "" }
        );
    }
    Ok(())
}

fn render(project_path: &Path, output: &Path, quality: Quality, dry_run: bool) -> Result<()> {
    let value = project::load(project_path)?;
    let plan = compiler::compile(project_path, &value, None)?;
    if dry_run {
        return print_dry_run(&plan, output, OutputKind::Mp4(quality));
    }
    ffmpeg::run(&plan, output, OutputKind::Mp4(quality))?;
    println!("Rendered {} ({:.3}s)", output.display(), plan.duration);
    Ok(())
}

fn play(project_path: &Path, range: TimelineRange, dry_run: bool) -> Result<()> {
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
    if dry_run {
        print_dry_run(&plan, Path::new("pipe:1"), OutputKind::MatroskaPreview)
    } else {
        ffmpeg::play(&plan)
    }
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

fn resolve_media_id(project_path: &Path, project: &Project, media_ref: &str) -> Result<String> {
    if let Ok(media) = project.media_ref(media_ref) {
        return Ok(media.id.clone());
    }
    let canonical = std::fs::canonicalize(media_ref)
        .map_err(|_| message(format!("media {media_ref:?} is not imported")))?;
    let stored = stored_media_path(project_path, &canonical)?;
    project
        .media
        .iter()
        .find(|media| media_paths_equal(&media.path, &stored))
        .map(|media| media.id.clone())
        .ok_or_else(|| message(format!("media {media_ref:?} is not imported")))
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
    Ok(canonical
        .strip_prefix(&base)
        .unwrap_or(canonical)
        .to_string_lossy()
        .replace('\\', "/"))
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
