use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand, ValueEnum};

use crate::error::{Result, message};
use crate::history;
use crate::project::{AudioClip, Media, MediaKind, Project, TtsProviderKind, VoiceClip};
use crate::tts::{self, SynthesisRequest};

#[derive(Debug, Parser)]
#[command(name = "ved", disable_help_subcommand = true)]
struct Phase3Cli {
    #[command(subcommand)]
    command: Phase3Command,
}

#[derive(Debug, Subcommand)]
enum Phase3Command {
    /// TTS voice discovery commands.
    Voice {
        #[command(subcommand)]
        command: VoiceCommand,
    },
    /// Synthesize and add a TTS clip.
    VoiceAdd {
        project: PathBuf,
        #[arg(long, value_enum)]
        provider: ProviderArg,
        #[arg(long)]
        voice: String,
        #[arg(long)]
        text: String,
        #[arg(long)]
        at: f64,
        #[arg(long, default_value_t = 1.0)]
        speed: f64,
        #[arg(long, default_value_t = 0.0)]
        pitch: f64,
        #[arg(long, default_value_t = 1.0)]
        volume: f64,
        #[arg(long)]
        endpoint: Option<String>,
    },
    /// Change TTS synthesis metadata or its normal AudioClip placement.
    VoiceSet {
        project: PathBuf,
        id: String,
        #[arg(long)]
        text: Option<String>,
        #[arg(long)]
        voice: Option<String>,
        #[arg(long)]
        speed: Option<f64>,
        #[arg(long)]
        pitch: Option<f64>,
        #[arg(long)]
        volume: Option<f64>,
        #[arg(long)]
        at: Option<f64>,
    },
    /// Remove a TTS clip without deleting shared cache files.
    VoiceRemove { project: PathBuf, id: String },
}

#[derive(Debug, Subcommand)]
enum VoiceCommand {
    /// List voices exposed by a local TTS provider.
    List {
        #[arg(long, value_enum)]
        provider: Option<ProviderArg>,
        #[arg(long, requires = "provider")]
        endpoint: Option<String>,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum ProviderArg {
    Voicevox,
    Aivis,
}

impl From<ProviderArg> for TtsProviderKind {
    fn from(value: ProviderArg) -> Self {
        match value {
            ProviderArg::Voicevox => Self::Voicevox,
            ProviderArg::Aivis => Self::Aivis,
        }
    }
}

pub fn is_phase3_command() -> bool {
    matches!(
        std::env::args().nth(1).as_deref(),
        Some("voice" | "voice-add" | "voice-set" | "voice-remove")
    )
}

pub fn run() -> Result<()> {
    match Phase3Cli::parse().command {
        Phase3Command::Voice { command } => match command {
            VoiceCommand::List {
                provider,
                endpoint,
                json,
            } => voice_list(provider, endpoint.as_deref(), json),
        },
        Phase3Command::VoiceAdd {
            project,
            provider,
            voice,
            text,
            at,
            speed,
            pitch,
            volume,
            endpoint,
        } => voice_add(
            &project,
            provider.into(),
            endpoint.as_deref(),
            voice,
            text,
            at,
            speed,
            pitch,
            volume,
        ),
        Phase3Command::VoiceSet {
            project,
            id,
            text,
            voice,
            speed,
            pitch,
            volume,
            at,
        } => voice_set(&project, &id, text, voice, speed, pitch, volume, at),
        Phase3Command::VoiceRemove { project, id } => voice_remove(&project, &id),
    }
}

fn voice_list(provider: Option<ProviderArg>, endpoint: Option<&str>, json: bool) -> Result<()> {
    let kinds = match provider {
        Some(value) => vec![TtsProviderKind::from(value)],
        None => vec![TtsProviderKind::Voicevox, TtsProviderKind::Aivis],
    };
    let mut voices = Vec::new();
    let mut failures = Vec::new();
    for kind in kinds {
        match tts::list_voices(kind, endpoint) {
            Ok(mut result) => voices.append(&mut result),
            Err(error) => failures.push(format!("{}: {error}", kind.as_str())),
        }
    }
    if voices.is_empty() && !failures.is_empty() {
        return Err(message(failures.join("; ")));
    }
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&voices)
                .map_err(|error| message(format!("could not serialize voice list: {error}")))?
        );
    } else {
        println!("{:<10} {:<10} {:<24} style", "provider", "voice", "name");
        for voice in &voices {
            println!(
                "{:<10} {:<10} {:<24} {}",
                voice.provider, voice.voice_id, voice.display_name, voice.style_name
            );
        }
        for failure in failures {
            eprintln!("warning: {failure}");
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn voice_add(
    project_path: &Path,
    provider: TtsProviderKind,
    endpoint: Option<&str>,
    voice: String,
    text: String,
    at: f64,
    speed: f64,
    pitch: f64,
    volume: f64,
) -> Result<()> {
    validate_timeline_value(at, "--at")?;
    validate_volume(volume)?;
    let mut project = crate::project::load(project_path)?;
    let before = project.clone();
    let synthesized = tts::synthesize_cached(
        project_path,
        provider,
        endpoint,
        &SynthesisRequest {
            voice: &voice,
            text: &text,
            speed,
            pitch,
        },
    )?;

    let media_id = project.next_media_id();
    let audio_id = project.next_audio_id();
    let voice_id = project.next_voice_id();
    project.media.push(Media {
        id: media_id.clone(),
        path: synthesized.stored_path,
        kind: MediaKind::Audio,
        probe: synthesized.probe,
    });
    project.audio_clips.push(AudioClip {
        id: audio_id.clone(),
        media_id,
        start: at,
        source_in: 0.0,
        source_out: None,
        volume,
        mute: false,
        fade_in: 0.0,
        fade_out: 0.0,
    });
    project.voice_clips.push(VoiceClip {
        id: voice_id.clone(),
        provider,
        voice,
        text,
        speed,
        pitch,
        endpoint: synthesized.endpoint,
        engine_identity: synthesized.engine_identity,
        cache_key: synthesized.cache_key,
        audio_clip_id: audio_id,
    });
    save_edit(project_path, &before, &project)?;
    println!("Added voice clip {voice_id}");
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn voice_set(
    project_path: &Path,
    id: &str,
    text: Option<String>,
    voice: Option<String>,
    speed: Option<f64>,
    pitch: Option<f64>,
    volume: Option<f64>,
    at: Option<f64>,
) -> Result<()> {
    if text.is_none()
        && voice.is_none()
        && speed.is_none()
        && pitch.is_none()
        && volume.is_none()
        && at.is_none()
    {
        return Err(message("voice-set requires at least one change"));
    }
    if let Some(value) = at {
        validate_timeline_value(value, "--at")?;
    }
    if let Some(value) = volume {
        validate_volume(value)?;
    }

    let mut project = crate::project::load(project_path)?;
    let before = project.clone();
    let voice_index = project
        .voice_clips
        .iter()
        .position(|item| item.id == id)
        .ok_or_else(|| message(format!("voice clip {id} does not exist")))?;
    let original = project.voice_clips[voice_index].clone();
    let new_text = text.unwrap_or_else(|| original.text.clone());
    let new_voice = voice.unwrap_or_else(|| original.voice.clone());
    let new_speed = speed.unwrap_or(original.speed);
    let new_pitch = pitch.unwrap_or(original.pitch);
    let synthesis_changed = new_text != original.text
        || new_voice != original.voice
        || new_speed != original.speed
        || new_pitch != original.pitch;

    let audio_index = project
        .audio_clips
        .iter()
        .position(|item| item.id == original.audio_clip_id)
        .ok_or_else(|| {
            message(format!(
                "audio clip {} does not exist",
                original.audio_clip_id
            ))
        })?;
    if let Some(value) = at {
        project.audio_clips[audio_index].start = value;
    }
    if let Some(value) = volume {
        project.audio_clips[audio_index].volume = value;
    }

    if synthesis_changed {
        let synthesized = tts::synthesize_cached(
            project_path,
            original.provider,
            Some(&original.endpoint),
            &SynthesisRequest {
                voice: &new_voice,
                text: &new_text,
                speed: new_speed,
                pitch: new_pitch,
            },
        )?;
        let media_id = project.audio_clips[audio_index].media_id.clone();
        let media = project
            .media
            .iter_mut()
            .find(|item| item.id == media_id)
            .ok_or_else(|| message(format!("media {media_id} does not exist")))?;
        media.path = synthesized.stored_path;
        media.kind = MediaKind::Audio;
        media.probe = synthesized.probe;
        let item = &mut project.voice_clips[voice_index];
        item.engine_identity = synthesized.engine_identity;
        item.cache_key = synthesized.cache_key;
        item.endpoint = synthesized.endpoint;
    }

    let item = &mut project.voice_clips[voice_index];
    item.text = new_text;
    item.voice = new_voice;
    item.speed = new_speed;
    item.pitch = new_pitch;
    save_edit(project_path, &before, &project)?;
    println!("Updated voice clip {id}");
    Ok(())
}

fn voice_remove(project_path: &Path, id: &str) -> Result<()> {
    let mut project = crate::project::load(project_path)?;
    let before = project.clone();
    let index = project
        .voice_clips
        .iter()
        .position(|item| item.id == id)
        .ok_or_else(|| message(format!("voice clip {id} does not exist")))?;
    let voice = project.voice_clips.remove(index);
    let audio_index = project
        .audio_clips
        .iter()
        .position(|item| item.id == voice.audio_clip_id)
        .ok_or_else(|| message(format!("audio clip {} does not exist", voice.audio_clip_id)))?;
    let audio = project.audio_clips.remove(audio_index);
    let media_still_used = project
        .audio_clips
        .iter()
        .any(|item| item.media_id == audio.media_id)
        || project
            .timeline
            .iter()
            .any(|item| item.media_id == audio.media_id)
        || project
            .image_overlays
            .iter()
            .any(|item| item.media_id == audio.media_id);
    if !media_still_used {
        project.media.retain(|item| item.id != audio.media_id);
    }
    save_edit(project_path, &before, &project)?;
    println!("Removed voice clip {id}");
    Ok(())
}

fn save_edit(path: &Path, before: &Project, after: &Project) -> Result<()> {
    after.validate()?;
    history::record_before_edit(path, before)?;
    crate::project::save(path, after)
}

fn validate_timeline_value(value: f64, name: &str) -> Result<()> {
    if !value.is_finite() || value < 0.0 {
        return Err(message(format!(
            "{name} must be a finite non-negative number"
        )));
    }
    Ok(())
}

fn validate_volume(value: f64) -> Result<()> {
    if !value.is_finite() || value < 0.0 {
        return Err(message("--volume must be a finite non-negative number"));
    }
    Ok(())
}
