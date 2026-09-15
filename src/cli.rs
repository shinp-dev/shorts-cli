use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use clap::{Args, Parser, Subcommand, ValueEnum};
use serde::Serialize;

use crate::compiler::{self, TimeRange};
use crate::error::{Result, VedError, message};
use crate::ffmpeg::{self, OutputKind, Quality};
use crate::history;
use crate::project::{
    self, AudioClip, AudioDucking, Axis, Canvas, Coordinate, Dimension, ImageOverlay, Media,
    MediaKind, Outline, Position, Project, TextOverlay, validate_agent_key, validate_color,
    validate_speed,
};
use crate::timeline;

const OUTPUT_HUMAN: u8 = 0;
const OUTPUT_JSON: u8 = 1;
const OUTPUT_QUIET: u8 = 2;
static OUTPUT_MODE: AtomicU8 = AtomicU8::new(OUTPUT_HUMAN);
static JSON_EMITTED: AtomicBool = AtomicBool::new(false);
static VERBOSE: AtomicBool = AtomicBool::new(false);
static COMMAND_NAME: OnceLock<&'static str> = OnceLock::new();

macro_rules! hprintln {
    ($($arg:tt)*) => {
        if OUTPUT_MODE.load(Ordering::Relaxed) == OUTPUT_HUMAN {
            println!($($arg)*);
        }
    };
}

#[derive(Debug, Parser)]
#[command(
    name = "ved",
    version,
    about = "Deterministic video editing for humans and AI agents"
)]
struct Cli {
    /// Emit one compact machine-readable JSON value.
    #[arg(long, global = true, conflicts_with = "quiet")]
    json: bool,
    /// Emit nothing on success.
    #[arg(long, global = true, conflicts_with = "json")]
    quiet: bool,
    /// Include full per-item details in diagnostic output.
    #[arg(long, global = true)]
    verbose: bool,
    #[command(subcommand)]
    command: Box<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Create a new project.
    New {
        project: PathBuf,
        #[arg(long, value_enum, conflicts_with_all = ["width", "height"])]
        preset: Option<Preset>,
        #[arg(long, requires = "height")]
        width: Option<u32>,
        #[arg(long, requires = "width")]
        height: Option<u32>,
        #[arg(long, default_value_t = 30)]
        fps: u32,
    },
    /// Diagnose FFmpeg tools and optional local services.
    Doctor,
    /// Report the bounded machine-readable feature contract.
    Capabilities {
        /// Return one named artifact JSON Schema.
        #[arg(long)]
        schema: Option<String>,
    },
    /// Inspect stable IDs and completed-timeline coordinates.
    Inspect {
        project: PathBuf,
        /// Restrict output to one or more compact sections.
        #[arg(long, value_enum)]
        section: Vec<crate::phase4::InspectSection>,
    },
    /// Validate a bounded director-to-executor work order without mutation.
    BriefCheck { project: PathBuf, brief: PathBuf },
    /// Run read-only project quality checks.
    Check {
        project: PathBuf,
        #[arg(long, default_value = "shorts")]
        profile: String,
    },
    /// Measure EBU R128 loudness for one clip, track, or final mix.
    Loudness {
        project: PathBuf,
        #[command(flatten)]
        target: LoudnessTarget,
        #[arg(long)]
        from: Option<f64>,
        #[arg(long)]
        to: Option<f64>,
    },
    /// Export a deterministic waveform PNG for one audio track.
    Waveform {
        project: PathBuf,
        #[arg(long)]
        track: String,
        #[arg(long)]
        out: PathBuf,
        #[arg(long, default_value_t = 1600)]
        width: u32,
        #[arg(long)]
        peaks_out: Option<PathBuf>,
        #[arg(long)]
        overwrite: bool,
    },
    /// Export stable completed-timeline markers without mutating the project.
    Markers {
        project: PathBuf,
        #[arg(long)]
        include: Vec<String>,
        #[arg(long, value_enum, default_value_t = crate::phase4::MarkerFormat::Json)]
        format: crate::phase4::MarkerFormat,
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        overwrite: bool,
    },
    /// Build an atomic preview and compact human/director review bundle.
    ReviewBuild {
        project: PathBuf,
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        notes: Option<PathBuf>,
        #[arg(long, default_value = "shorts")]
        profile: String,
        #[arg(long)]
        since: Option<PathBuf>,
        #[arg(long)]
        overwrite: bool,
    },
    /// List available local TTS voices.
    Voices {
        #[arg(long)]
        provider: Option<String>,
    },
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
        #[command(flatten)]
        retime: RetimeArgs,
    },
    /// Insert a still-frame hold.
    Hold {
        project: PathBuf,
        #[arg(long, requires = "source", conflicts_with_all = ["after", "before"])]
        clip: Option<String>,
        #[arg(long, requires = "clip", conflicts_with_all = ["after", "before"])]
        source: Option<f64>,
        #[arg(long, conflicts_with_all = ["clip", "source", "before"])]
        after: Option<String>,
        #[arg(long, conflicts_with_all = ["clip", "source", "after"])]
        before: Option<String>,
        #[arg(long)]
        duration: f64,
        #[command(flatten)]
        retime: RetimeArgs,
    },
    /// Change a still-frame hold's duration.
    HoldSet {
        project: PathBuf,
        #[arg(long)]
        hold: String,
        #[arg(long)]
        duration: f64,
        #[command(flatten)]
        retime: RetimeArgs,
    },
    /// Remove a still-frame hold.
    HoldRemove {
        project: PathBuf,
        #[arg(long)]
        hold: String,
        #[command(flatten)]
        retime: RetimeArgs,
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
        #[arg(long)]
        key: Option<String>,
        #[arg(long)]
        track: Option<String>,
        #[arg(long)]
        to: Option<String>,
        #[command(flatten)]
        source: SourceRange,
        #[arg(long, default_value_t = 1.0)]
        speed: f64,
        #[arg(long)]
        r#loop: bool,
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
    /// Atomically synchronize a keyed audio schedule.
    AudioSync {
        project: PathBuf,
        manifest: PathBuf,
        #[arg(long)]
        replace_track: Vec<String>,
        #[arg(long)]
        replace_ducking: bool,
    },
    /// Change independent audio playback speed.
    AudioSpeed {
        project: PathBuf,
        #[arg(long)]
        audio: String,
        #[arg(long)]
        rate: f64,
    },
    /// Refresh imported probe metadata.
    MediaRefresh {
        project: PathBuf,
        media: Option<String>,
        #[arg(long, conflicts_with = "media")]
        all: bool,
    },
    /// Add automatic track-based ducking.
    DuckAdd {
        project: PathBuf,
        #[arg(long)]
        key: String,
        #[arg(long)]
        target_track: String,
        #[arg(long, required = true)]
        trigger_track: Vec<String>,
        #[arg(long, default_value_t = 12.0)]
        reduction_db: f64,
        #[arg(long, default_value_t = 0.12)]
        attack: f64,
        #[arg(long, default_value_t = 0.35)]
        release: f64,
    },
    /// Change automatic track-based ducking.
    DuckSet {
        project: PathBuf,
        #[arg(long)]
        ducking: String,
        #[arg(long)]
        target_track: Option<String>,
        #[arg(long)]
        trigger_track: Vec<String>,
        #[arg(long)]
        reduction_db: Option<f64>,
        #[arg(long)]
        attack: Option<f64>,
        #[arg(long)]
        release: Option<f64>,
    },
    /// Remove automatic track-based ducking.
    DuckRemove {
        project: PathBuf,
        #[arg(long)]
        ducking: String,
    },
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
        #[arg(long)]
        overwrite: bool,
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
        #[arg(long)]
        overwrite: bool,
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
        #[arg(long)]
        overwrite: bool,
    },
}

impl Command {
    fn name(&self) -> &'static str {
        match self {
            Self::New { .. } => "new",
            Self::Doctor => "doctor",
            Self::Capabilities { .. } => "capabilities",
            Self::Inspect { .. } => "inspect",
            Self::BriefCheck { .. } => "brief-check",
            Self::Check { .. } => "check",
            Self::Loudness { .. } => "loudness",
            Self::Waveform { .. } => "waveform",
            Self::Markers { .. } => "markers",
            Self::ReviewBuild { .. } => "review-build",
            Self::Voices { .. } => "voices",
            Self::Import { .. } => "import",
            Self::Add { .. } => "add",
            Self::Insert { .. } => "insert",
            Self::Trim { .. } => "trim",
            Self::Remove { .. } => "remove",
            Self::Speed { .. } => "speed",
            Self::Hold { .. } => "hold",
            Self::HoldSet { .. } => "hold-set",
            Self::HoldRemove { .. } => "hold-remove",
            Self::TextAdd { .. } => "text-add",
            Self::TextSet { .. } => "text-set",
            Self::TextRemove { .. } => "text-remove",
            Self::ImageAdd { .. } => "image-add",
            Self::ImageSet { .. } => "image-set",
            Self::ImageRemove { .. } => "image-remove",
            Self::AudioAdd { .. } => "audio-add",
            Self::AudioRemove { .. } => "audio-remove",
            Self::AudioSync { .. } => "audio-sync",
            Self::AudioSpeed { .. } => "audio-speed",
            Self::MediaRefresh { .. } => "media-refresh",
            Self::DuckAdd { .. } => "duck-add",
            Self::DuckSet { .. } => "duck-set",
            Self::DuckRemove { .. } => "duck-remove",
            Self::Volume { .. } => "volume",
            Self::Mute { .. } => "mute",
            Self::FadeIn { .. } => "fade-in",
            Self::FadeOut { .. } => "fade-out",
            Self::Undo { .. } => "undo",
            Self::Redo { .. } => "redo",
            Self::Describe { .. } => "describe",
            Self::Render { .. } => "render",
            Self::Play { .. } => "play",
            Self::Frame { .. } => "frame",
            Self::ExportRange { .. } => "export-range",
        }
    }
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
struct RetimeArgs {
    #[arg(long)]
    retime_overlays: bool,
    #[arg(long)]
    retime_track: Vec<String>,
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

#[derive(Debug, Args)]
#[group(required = true, multiple = false)]
struct LoudnessTarget {
    #[arg(long)]
    audio: Option<String>,
    #[arg(long)]
    track: Option<String>,
    #[arg(long)]
    mix: bool,
}

pub fn run() -> Result<()> {
    let cli = Cli::parse();
    OUTPUT_MODE.store(
        if cli.json {
            OUTPUT_JSON
        } else if cli.quiet {
            OUTPUT_QUIET
        } else {
            OUTPUT_HUMAN
        },
        Ordering::Relaxed,
    );
    VERBOSE.store(cli.verbose, Ordering::Relaxed);
    let _ = COMMAND_NAME.set(cli.command.name());
    let result = match *cli.command {
        Command::New {
            project,
            preset,
            width,
            height,
            fps,
        } => new_project(&project, preset, width, height, fps),
        Command::Doctor => doctor(),
        Command::Capabilities { schema } => capabilities(schema.as_deref()),
        Command::Inspect { project, section } => inspect(&project, &section),
        Command::BriefCheck { project, brief } => brief_check(&project, &brief),
        Command::Check { project, profile } => check(&project, &profile),
        Command::Loudness {
            project,
            target,
            from,
            to,
        } => loudness(&project, &target, from, to),
        Command::Waveform {
            project,
            track,
            out,
            width,
            peaks_out,
            overwrite,
        } => waveform(
            &project,
            &track,
            &out,
            width,
            peaks_out.as_deref(),
            overwrite,
        ),
        Command::Markers {
            project,
            include,
            format,
            out,
            overwrite,
        } => markers(&project, &include, format, &out, overwrite),
        Command::ReviewBuild {
            project,
            out,
            notes,
            profile,
            since,
            overwrite,
        } => review_build(
            &project,
            &out,
            notes.as_deref(),
            &profile,
            since.as_deref(),
            overwrite,
        ),
        Command::Voices { provider } => voices(provider.as_deref()),
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
            retime,
        } => edit(&project, |value| {
            validate_retime_tracks(&retime.retime_track)?;
            timeline::speed_with_retime(
                value,
                &clip,
                rate,
                retime.retime_overlays,
                &retime.retime_track,
            )?;
            Ok(format!("Set clip {clip} speed to {rate}x"))
        }),
        Command::Hold {
            project,
            clip,
            source,
            after,
            before,
            duration,
            retime,
        } => edit(&project, |value| {
            validate_retime_tracks(&retime.retime_track)?;
            let id = if let (Some(clip), Some(source)) = (clip.as_deref(), source) {
                timeline::hold_at_source(
                    value,
                    clip,
                    source,
                    duration,
                    retime.retime_overlays,
                    &retime.retime_track,
                )?
            } else if let Some(clip) = after.as_deref() {
                timeline::hold_after(
                    value,
                    clip,
                    duration,
                    retime.retime_overlays,
                    &retime.retime_track,
                )?
            } else if let Some(clip) = before.as_deref() {
                timeline::hold_before(
                    value,
                    clip,
                    duration,
                    retime.retime_overlays,
                    &retime.retime_track,
                )?
            } else {
                return Err(message(
                    "hold requires --clip with --source, --after, or --before",
                ));
            };
            Ok(format!("Added hold {id}"))
        }),
        Command::HoldSet {
            project,
            hold,
            duration,
            retime,
        } => edit(&project, |value| {
            validate_retime_tracks(&retime.retime_track)?;
            timeline::hold_set(
                value,
                &hold,
                duration,
                retime.retime_overlays,
                &retime.retime_track,
            )?;
            Ok(format!("Set hold {hold} duration to {duration}s"))
        }),
        Command::HoldRemove {
            project,
            hold,
            retime,
        } => edit(&project, |value| {
            validate_retime_tracks(&retime.retime_track)?;
            timeline::hold_remove(value, &hold, retime.retime_overlays, &retime.retime_track)?;
            Ok(format!("Removed hold {hold}"))
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
            key,
            track,
            to,
            source,
            speed,
            r#loop,
            volume,
            fade_in,
            fade_out,
            mute,
        } => edit_with_media(&project, &media, |value, media_id| {
            if let Some(value) = &key {
                validate_agent_key(value, "audio key")?;
            }
            if let Some(value) = &track {
                validate_agent_key(value, "audio track")?;
            }
            validate_speed(speed)?;
            let media = value.media_by_id(media_id)?;
            if media.kind != MediaKind::Audio && !media.probe.has_audio {
                return Err(message("audio-add requires media with audio"));
            }
            let id = value.next_audio_id();
            let end = resolve_audio_end(to.as_deref(), value)?;
            value.audio_clips.push(AudioClip {
                id: id.clone(),
                key,
                track,
                media_id: media_id.into(),
                start: at,
                end,
                source_in: source.source_in.unwrap_or(0.0),
                source_out: source.source_out,
                speed,
                r#loop,
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
        Command::AudioSync {
            project,
            manifest,
            replace_track,
            replace_ducking,
        } => {
            let summary =
                crate::audio_sync::sync(&project, &manifest, &replace_track, replace_ducking)?;
            if summary.changed {
                hprintln!(
                    "Synchronized audio: +{} ~{} -{}, media +{} ~{}, ducking +{} ~{} -{}",
                    summary.audio_created,
                    summary.audio_updated,
                    summary.audio_removed,
                    summary.media_created,
                    summary.media_updated,
                    summary.ducking_created,
                    summary.ducking_updated,
                    summary.ducking_removed
                );
            } else {
                hprintln!("Audio schedule is already synchronized");
            }
            let current = project::load(&project)?;
            machine_json(serde_json::json!({
                "project": project.to_string_lossy(),
                "duration": timeline::duration(&current),
                "changed": summary.changed,
                "created": {
                    "media": summary.media_created,
                    "audio": summary.audio_created,
                    "ducking": summary.ducking_created
                },
                "updated": {
                    "media": summary.media_updated,
                    "audio": summary.audio_updated,
                    "ducking": summary.ducking_updated
                },
                "removed": {
                    "media": 0,
                    "audio": summary.audio_removed,
                    "ducking": summary.ducking_removed
                },
                "warnings": [],
                "media_created": summary.media_created,
                "media_updated": summary.media_updated,
                "audio_created": summary.audio_created,
                "audio_updated": summary.audio_updated,
                "audio_removed": summary.audio_removed,
                "ducking_created": summary.ducking_created,
                "ducking_updated": summary.ducking_updated,
                "ducking_removed": summary.ducking_removed
            }));
            Ok(())
        }
        Command::AudioSpeed {
            project,
            audio,
            rate,
        } => edit(&project, |value| {
            validate_speed(rate)?;
            let item = value
                .audio_clips
                .iter_mut()
                .find(|item| item.id == audio)
                .ok_or_else(|| message(format!("audio clip {audio} does not exist")))?;
            item.speed = rate;
            Ok(format!("Set audio clip {audio} speed to {rate}x"))
        }),
        Command::MediaRefresh {
            project,
            media,
            all,
        } => media_refresh(&project, media.as_deref(), all),
        Command::DuckAdd {
            project,
            key,
            target_track,
            trigger_track,
            reduction_db,
            attack,
            release,
        } => edit(&project, |value| {
            let id = value.next_ducking_id();
            value.audio_ducking.push(AudioDucking {
                id: id.clone(),
                key,
                target_track,
                trigger_tracks: trigger_track,
                reduction_db,
                attack,
                release,
            });
            Ok(format!("Added ducking rule {id}"))
        }),
        Command::DuckSet {
            project,
            ducking,
            target_track,
            trigger_track,
            reduction_db,
            attack,
            release,
        } => edit(&project, |value| {
            let item = value
                .audio_ducking
                .iter_mut()
                .find(|item| item.id == ducking)
                .ok_or_else(|| message(format!("ducking rule {ducking} does not exist")))?;
            let mut changed = false;
            set_option(&mut item.target_track, target_track, &mut changed);
            set_option(&mut item.reduction_db, reduction_db, &mut changed);
            set_option(&mut item.attack, attack, &mut changed);
            set_option(&mut item.release, release, &mut changed);
            if !trigger_track.is_empty() {
                item.trigger_tracks = trigger_track;
                changed = true;
            }
            if !changed {
                return Err(message("duck-set requires at least one change"));
            }
            Ok(format!("Updated ducking rule {ducking}"))
        }),
        Command::DuckRemove { project, ducking } => edit(&project, |value| {
            remove_by_id(
                &mut value.audio_ducking,
                &ducking,
                |item| &item.id,
                "ducking rule",
            )?;
            Ok(format!("Removed ducking rule {ducking}"))
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
            hprintln!("Undid last edit");
            Ok(())
        }
        Command::Redo { project } => {
            history::redo(&project)?;
            hprintln!("Redid last edit");
            Ok(())
        }
        Command::Describe { project } => describe(&project),
        Command::Render {
            project,
            output,
            quality,
            dry_run,
            overwrite,
        } => render(&project, &output, quality, dry_run, overwrite),
        Command::Play {
            project,
            range,
            dry_run,
        } => play(&project, range, dry_run),
        Command::Frame {
            project,
            time,
            output,
            overwrite,
        } => frame(&project, time, &output, overwrite),
        Command::ExportRange {
            project,
            from,
            to,
            out,
            quality,
            overwrite,
        } => export_range(&project, from, to, &out, quality, overwrite),
    };
    if result.is_ok()
        && OUTPUT_MODE.load(Ordering::Relaxed) == OUTPUT_JSON
        && !JSON_EMITTED.load(Ordering::Relaxed)
    {
        machine_json(serde_json::json!({}));
    }
    result
}

fn new_project(
    path: &Path,
    preset: Option<Preset>,
    width: Option<u32>,
    height: Option<u32>,
    fps: u32,
) -> Result<()> {
    if path.exists() {
        return Err(message(format!(
            "project already exists: {}",
            path.display()
        )));
    }
    if fps == 0 {
        return Err(message("--fps must be greater than zero"));
    }
    let (width, height) = match (width, height) {
        (Some(width), Some(height)) => (width, height),
        (None, None) => match preset.unwrap_or(Preset::Vertical) {
            Preset::Vertical => (1080, 1920),
            Preset::Horizontal => (1920, 1080),
            Preset::Square => (1080, 1080),
        },
        _ => return Err(message("--width and --height must be supplied together")),
    };
    if width == 0 || height == 0 {
        return Err(message("--width and --height must be greater than zero"));
    }
    project::save(path, &Project::new(Canvas { width, height, fps }))?;
    hprintln!(
        "Created {} ({}x{} @ {}fps)",
        path.display(),
        width,
        height,
        fps
    );
    machine_json(serde_json::json!({
        "project": path.to_string_lossy(),
        "duration": 0.0,
        "canvas": { "width": width, "height": height, "fps": fps },
        "changed": true
    }));
    Ok(())
}

fn doctor() -> Result<()> {
    let statuses = ffmpeg::doctor();
    hprintln!("ved {}", env!("CARGO_PKG_VERSION"));
    for status in &statuses {
        hprintln!(
            "{:<12} {:<11} {}",
            status.name,
            if status.available {
                "ok"
            } else {
                "unavailable"
            },
            status
                .version
                .as_deref()
                .or(status.detail.as_deref())
                .unwrap_or_default()
        );
    }
    machine_json(serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "tools": statuses
    }));
    Ok(())
}

fn capabilities(schema: Option<&str>) -> Result<()> {
    let value = crate::phase4::capabilities(schema)?;
    if schema.is_some() {
        hprintln!("Work-order schema version 1");
    } else {
        hprintln!("ved {} executor contract", env!("CARGO_PKG_VERSION"));
        hprintln!("Network default: deny");
    }
    machine_json(value);
    Ok(())
}

fn inspect(project_path: &Path, sections: &[crate::phase4::InspectSection]) -> Result<()> {
    let project = project::load(project_path)?;
    let value = crate::phase4::inspect(
        project_path,
        &project,
        sections,
        VERBOSE.load(Ordering::Relaxed),
    )?;
    hprintln!(
        "{}  hash={}  duration={:.3}s",
        project_path.display(),
        value["project_hash"].as_str().unwrap_or("unknown"),
        timeline::duration(&project)
    );
    hprintln!(
        "timeline={} text={} images={} audio={} ducking={}",
        project.timeline.len(),
        project.text_overlays.len(),
        project.image_overlays.len(),
        project.audio_clips.len(),
        project.audio_ducking.len()
    );
    machine_json(value);
    Ok(())
}

fn brief_check(project_path: &Path, brief_path: &Path) -> Result<()> {
    let project = project::load(project_path)?;
    let value = crate::phase4::brief_check(brief_path, &project)?;
    hprintln!(
        "Valid work order for {} ({} changes)",
        project_path.display(),
        value["changes"].as_u64().unwrap_or(0)
    );
    machine_json(value);
    Ok(())
}

fn check(project_path: &Path, profile: &str) -> Result<()> {
    let project = project::load(project_path)?;
    let report = crate::phase4::check(project_path, &project, profile)?;
    hprintln!(
        "Checks: {} failed, {} warnings, {} info",
        report.counts.fail,
        report.counts.warn,
        report.counts.info
    );
    for issue in &report.issues {
        hprintln!(
            "{} {} {}: {}",
            issue.severity,
            issue.code,
            issue.target,
            issue.message
        );
    }
    machine_json(report);
    Ok(())
}

fn loudness(
    project_path: &Path,
    target: &LoudnessTarget,
    from: Option<f64>,
    to: Option<f64>,
) -> Result<()> {
    let project = project::load(project_path)?;
    let report = crate::audio_analysis::loudness(
        project_path,
        &project,
        crate::audio_analysis::LoudnessOptions {
            audio: target.audio.as_deref(),
            track: target.track.as_deref(),
            mix: target.mix,
            from,
            to,
        },
    )?;
    hprintln!(
        "{}: integrated={} LUFS, true peak={} dBTP, LRA={} LU{}",
        report.source,
        report
            .integrated_lufs
            .map_or_else(|| "-inf".into(), |value| format!("{value:.2}")),
        report
            .true_peak_dbtp
            .map_or_else(|| "-inf".into(), |value| format!("{value:.2}")),
        report
            .loudness_range_lu
            .map_or_else(|| "0".into(), |value| format!("{value:.2}")),
        if report.cached { " (cached)" } else { "" }
    );
    machine_json(report);
    Ok(())
}

fn waveform(
    project_path: &Path,
    track: &str,
    output: &Path,
    width: u32,
    peaks_output: Option<&Path>,
    overwrite: bool,
) -> Result<()> {
    let project = project::load(project_path)?;
    let summary = crate::audio_analysis::waveform(
        project_path,
        &project,
        track,
        output,
        width,
        peaks_output,
        overwrite,
    )?;
    hprintln!(
        "Wrote {}x{} waveform for track {} to {}",
        summary.width,
        summary.height,
        summary.track,
        summary.output
    );
    machine_json(summary);
    Ok(())
}

fn markers(
    project_path: &Path,
    includes: &[String],
    format: crate::phase4::MarkerFormat,
    output: &Path,
    overwrite: bool,
) -> Result<()> {
    let project = project::load(project_path)?;
    let summary = crate::phase4::export_markers(&project, includes, format, output, overwrite)?;
    hprintln!(
        "Wrote {} {} markers to {}",
        summary.markers,
        summary.format,
        summary.output
    );
    machine_json(summary);
    Ok(())
}

fn review_build(
    project_path: &Path,
    output_root: &Path,
    notes: Option<&Path>,
    profile: &str,
    since: Option<&Path>,
    overwrite: bool,
) -> Result<()> {
    let project = project::load(project_path)?;
    let summary = crate::phase4::review_build(
        project_path,
        &project,
        crate::phase4::ReviewBuildOptions {
            output_root,
            notes,
            profile,
            since,
            overwrite,
        },
    )?;
    hprintln!(
        "{} review {}",
        if summary.changed { "Built" } else { "Reused" },
        summary.review_id
    );
    hprintln!("Video: {}", summary.video);
    hprintln!("Report: {}", summary.report);
    hprintln!("Storyboard: {}", summary.storyboard);
    machine_json(summary);
    Ok(())
}

fn voices(provider: Option<&str>) -> Result<()> {
    let values = crate::audio_sync::voices(provider)?;
    for voice in &values {
        hprintln!("{:<12} {:<8} {}", voice.provider, voice.key, voice.name);
    }
    machine_json(serde_json::json!({ "voices": values }));
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
        hprintln!("Already imported as {}: {}", existing.id, existing.path);
        machine_json(serde_json::json!({
            "project": project_path.to_string_lossy(),
            "duration": timeline::duration(&value),
            "changed": false,
            "media_id": existing.id,
            "path": existing.path
        }));
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
    hprintln!("Imported {id}: {stored}");
    machine_json(serde_json::json!({
        "project": project_path.to_string_lossy(),
        "duration": timeline::duration(&value),
        "changed": true,
        "media_id": id,
        "path": stored
    }));
    Ok(())
}

fn media_refresh(project_path: &Path, media_ref: Option<&str>, all: bool) -> Result<()> {
    if !all && media_ref.is_none() {
        return Err(message("media-refresh requires a media reference or --all"));
    }
    let mut value = project::load(project_path)?;
    let before = value.clone();
    let ids = if all {
        value
            .media
            .iter()
            .map(|item| item.id.clone())
            .collect::<Vec<_>>()
    } else {
        vec![resolve_media_id(project_path, &value, media_ref.unwrap())?]
    };
    let mut refreshed = Vec::new();
    for id in ids {
        let index = value
            .media
            .iter()
            .position(|item| item.id == id)
            .ok_or_else(|| message(format!("media {id} does not exist")))?;
        let path = resolved_media_path(project_path, &value.media[index].path);
        let (kind, probe) = ffmpeg::probe(&path)?;
        if kind != value.media[index].kind {
            return Err(message(format!(
                "media {} changed kind from {:?} to {:?}",
                id, value.media[index].kind, kind
            )));
        }
        if probe != value.media[index].probe {
            value.media[index].probe = probe;
            refreshed.push(id);
        }
    }
    value.validate()?;
    if refreshed.is_empty() {
        hprintln!("Media probe data is already current");
        machine_json(serde_json::json!({
            "project": project_path.to_string_lossy(),
            "duration": timeline::duration(&value),
            "changed": false,
            "refreshed": 0
        }));
        return Ok(());
    }
    save_edit(project_path, &before, &value)?;
    hprintln!("Refreshed {} media item(s)", refreshed.len());
    machine_json(serde_json::json!({
        "project": project_path.to_string_lossy(),
        "duration": timeline::duration(&value),
        "changed": true,
        "refreshed": refreshed.len(),
        "media_ids": if VERBOSE.load(Ordering::Relaxed) { refreshed } else { Vec::new() }
    }));
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
    hprintln!("{output}");
    hprintln!("Timeline duration: {:.3}s", timeline::duration(&value));
    machine_json(serde_json::json!({
        "message": output,
        "project": project_path.to_string_lossy(),
        "duration": timeline::duration(&value),
        "changed": true
    }));
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
    hprintln!("{output}");
    hprintln!("Timeline duration: {:.3}s", timeline::duration(&value));
    machine_json(serde_json::json!({
        "message": output,
        "project": project_path.to_string_lossy(),
        "duration": timeline::duration(&value),
        "changed": true
    }));
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
        .find(|clip| clip.id() == clip_id)
        .ok_or_else(|| message(format!("clip {clip_id} does not exist")))?;
    let clip = clip.clip_mut().ok_or_else(|| {
        message(format!(
            "{clip_id} is a hold; audio options require a video clip"
        ))
    })?;
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

fn resolve_audio_end(value: Option<&str>, project: &Project) -> Result<Option<f64>> {
    let Some(value) = value else { return Ok(None) };
    if value.eq_ignore_ascii_case("timeline") {
        return Ok(Some(timeline::duration(project)));
    }
    let end = value
        .parse::<f64>()
        .map_err(|_| message("--to must be a timeline second or 'timeline'"))?;
    if !end.is_finite() || end < 0.0 {
        return Err(message("--to must be a finite non-negative number"));
    }
    Ok(Some(end))
}

fn validate_retime_tracks(values: &[String]) -> Result<()> {
    for value in values {
        validate_agent_key(value, "audio track")?;
    }
    Ok(())
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
            .find(|item| item.id() == *id)
            .ok_or_else(|| message(format!("clip {id} does not exist")))?;
        let clip = clip.clip_mut().ok_or_else(|| {
            message(format!(
                "{id} is a hold; audio options require a video clip"
            ))
        })?;
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
    hprintln!("Duration: {:.3}s", timeline::duration(&value));
    hprintln!(
        "Canvas: {}x{} @ {}fps",
        value.canvas.width,
        value.canvas.height,
        value.canvas.fps
    );
    hprintln!("\nTimeline:");
    if value.timeline.is_empty() {
        hprintln!("(empty)");
    }
    for item in timeline::resolve(&value) {
        match &item.item {
            crate::project::TimelineItem::Clip(clip) => {
                let media = value.media_by_id(&clip.media_id)?;
                hprintln!(
                    "{}  {} - {}",
                    clip.id,
                    timestamp(item.timeline_start),
                    timestamp(item.timeline_end)
                );
                hprintln!(
                    "    {} [{} - {}] speed={}x volume={}{}",
                    media.path,
                    timestamp(clip.source_in),
                    timestamp(clip.source_out),
                    clip.speed,
                    clip.volume,
                    if clip.mute { " mute" } else { "" }
                );
            }
            crate::project::TimelineItem::Hold(hold) => {
                let media = value.media_by_id(&hold.media_id)?;
                hprintln!(
                    "{}  {} - {}\n    {} freeze={} duration={}",
                    hold.id,
                    timestamp(item.timeline_start),
                    timestamp(item.timeline_end),
                    media.path,
                    timestamp(hold.freeze_at),
                    hold.duration
                );
            }
        }
    }
    hprintln!("\nText:");
    if value.text_overlays.is_empty() {
        hprintln!("(none)");
    }
    for item in &value.text_overlays {
        hprintln!(
            "{}  {} - {}\n    {:?} x={} y={}",
            item.id,
            timestamp(item.start),
            timestamp(item.end),
            item.text,
            item.position.x.0,
            item.position.y.0
        );
    }
    hprintln!("\nImages:");
    if value.image_overlays.is_empty() {
        hprintln!("(none)");
    }
    for item in &value.image_overlays {
        hprintln!(
            "{}  {} - {}\n    {} x={} y={}",
            item.id,
            timestamp(item.start),
            timestamp(item.end),
            value.media_by_id(&item.media_id)?.path,
            item.position.x.0,
            item.position.y.0
        );
    }
    hprintln!("\nAudio:");
    if value.audio_clips.is_empty() {
        hprintln!("(none)");
    }
    for item in &value.audio_clips {
        hprintln!(
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
    machine_json(serde_json::json!({
        "project": project_path.to_string_lossy(),
        "duration": timeline::duration(&value),
        "canvas": value.canvas,
        "counts": {
            "media": value.media.len(),
            "timeline": value.timeline.len(),
            "text": value.text_overlays.len(),
            "images": value.image_overlays.len(),
            "audio": value.audio_clips.len(),
            "ducking": value.audio_ducking.len()
        }
    }));
    Ok(())
}

fn render(
    project_path: &Path,
    output: &Path,
    quality: Quality,
    dry_run: bool,
    overwrite: bool,
) -> Result<()> {
    let value = project::load(project_path)?;
    let plan = compiler::compile(project_path, &value, None)?;
    if dry_run {
        return print_dry_run(&plan, output, OutputKind::Mp4(quality));
    }
    ffmpeg::run_overwrite(&plan, output, OutputKind::Mp4(quality), overwrite)?;
    hprintln!("Rendered {} ({:.3}s)", output.display(), plan.duration);
    machine_json(serde_json::json!({
        "output": output.to_string_lossy(),
        "duration": plan.duration
    }));
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

fn frame(project_path: &Path, time: f64, output: &Path, overwrite: bool) -> Result<()> {
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
    ffmpeg::run_overwrite(&plan, output, OutputKind::Frame, overwrite)?;
    hprintln!("Wrote frame at {} to {}", timestamp(time), output.display());
    machine_json(serde_json::json!({
        "output": output.to_string_lossy(),
        "time": time,
        "duration": total
    }));
    Ok(())
}

fn export_range(
    project_path: &Path,
    from: f64,
    to: f64,
    output: &Path,
    quality: Quality,
    overwrite: bool,
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
    ffmpeg::run_overwrite(&plan, output, OutputKind::Mp4(quality), overwrite)?;
    hprintln!(
        "Exported {} - {} to {}",
        timestamp(from),
        timestamp(to),
        output.display()
    );
    machine_json(serde_json::json!({
        "output": output.to_string_lossy(),
        "from": from,
        "to": to,
        "duration": plan.duration
    }));
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

fn resolved_media_path(project_path: &Path, stored: &str) -> PathBuf {
    let path = PathBuf::from(stored.replace('/', std::path::MAIN_SEPARATOR_STR));
    if path.is_absolute() {
        path
    } else {
        project_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(path)
    }
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
    if OUTPUT_MODE.load(Ordering::Relaxed) == OUTPUT_JSON {
        if VERBOSE.load(Ordering::Relaxed) {
            machine_json(&data);
        } else {
            let bytes = serde_json::to_vec(plan).map_err(|error| message(error.to_string()))?;
            machine_json(serde_json::json!({
                "output": output.to_string_lossy(),
                "duration": plan.duration,
                "canvas": plan.canvas,
                "inputs": {
                    "video": plan.video_segments.len(),
                    "images": plan.image_overlays.len(),
                    "audio": plan.audio_clips.len()
                },
                "overlays": {
                    "text": plan.text_overlays.len(),
                    "images": plan.image_overlays.len()
                },
                "plan_hash": format!("{:016x}", stable_hash(&bytes))
            }));
        }
    } else if OUTPUT_MODE.load(Ordering::Relaxed) == OUTPUT_HUMAN {
        println!(
            "{}",
            serde_json::to_string_pretty(&data).map_err(|error| message(error.to_string()))?
        );
    }
    Ok(())
}

fn stable_hash(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn timestamp(seconds: f64) -> String {
    let total_ms = (seconds * 1000.0).round() as u64;
    let minutes = total_ms / 60_000;
    let seconds = (total_ms % 60_000) / 1000;
    let millis = total_ms % 1000;
    format!("{minutes:02}:{seconds:02}.{millis:03}")
}

fn machine_json<T: Serialize>(value: T) {
    if OUTPUT_MODE.load(Ordering::Relaxed) != OUTPUT_JSON {
        return;
    }
    let mut data = serde_json::to_value(value).unwrap_or_else(|_| serde_json::json!({}));
    let mut object = match data.take() {
        serde_json::Value::Object(value) => value,
        value => {
            let mut object = serde_json::Map::new();
            object.insert("result".into(), value);
            object
        }
    };
    object.insert("ok".into(), serde_json::Value::Bool(true));
    object.insert(
        "command".into(),
        serde_json::Value::String(COMMAND_NAME.get().copied().unwrap_or("unknown").into()),
    );
    println!("{}", serde_json::Value::Object(object));
    JSON_EMITTED.store(true, Ordering::Relaxed);
}

pub fn print_error(error: &VedError) {
    if OUTPUT_MODE.load(Ordering::Relaxed) == OUTPUT_JSON {
        eprintln!(
            "{}",
            serde_json::json!({
                "ok": false,
                "command": COMMAND_NAME.get().copied().unwrap_or("unknown"),
                "code": error_code(error),
                "message": error.to_string()
            })
        );
    } else {
        eprintln!("error: {error}");
    }
}

fn error_code(error: &VedError) -> &'static str {
    match error {
        VedError::Message(_) => "invalid_operation",
        VedError::Coded { code, .. } => code,
        VedError::Io { .. } => "io_error",
        VedError::Json { .. } => "invalid_json",
        VedError::Process { .. } => "process_start_failed",
        VedError::ProcessFailed { .. } => "process_failed",
    }
}
