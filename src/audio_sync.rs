use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::error::{Result, VedError, message};
use crate::ffmpeg;
use crate::hashing::sha256_hex;
use crate::history;
use crate::project::{
    self, AudioClip, AudioDucking, Media, MediaKind, Project, validate_agent_key, validate_speed,
};
use crate::timeline;

#[derive(Debug, Serialize)]
pub struct SyncSummary {
    pub changed: bool,
    pub media_created: usize,
    pub media_updated: usize,
    pub audio_created: usize,
    pub audio_updated: usize,
    pub audio_removed: usize,
    pub ducking_created: usize,
    pub ducking_updated: usize,
    pub ducking_removed: usize,
}

#[derive(Debug, Serialize)]
pub struct VoiceInfo {
    pub provider: String,
    pub key: String,
    pub name: String,
    pub language: Option<String>,
}

pub fn voices(provider: Option<&str>) -> Result<Vec<VoiceInfo>> {
    let optional = provider.is_none();
    let providers = provider
        .map(|value| vec![value.to_ascii_lowercase()])
        .unwrap_or_else(|| vec!["sapi".into(), "voicevox".into(), "aivisspeech".into()]);
    let mut output = Vec::new();
    for provider in providers {
        let result = match provider.as_str() {
            "sapi" => sapi_voices(),
            "voicevox" => http_voices("voicevox", 50021),
            "aivisspeech" | "aivis" => http_voices("aivisspeech", 10101),
            _ => return Err(message(format!("unsupported TTS provider {provider:?}"))),
        };
        match result {
            Ok(mut values) => output.append(&mut values),
            Err(error) if optional => {
                let _ = error;
            }
            Err(error) => return Err(error),
        }
    }
    if let Some(provider) = provider
        && output.is_empty()
    {
        return Err(message(format!(
            "TTS provider {:?} is unavailable or has no voices",
            provider
        )));
    }
    Ok(output)
}

#[cfg(windows)]
fn sapi_voices() -> Result<Vec<VoiceInfo>> {
    let script =
        "$v=New-Object -ComObject SAPI.SpVoice;$v.GetVoices()|ForEach-Object{$_.GetDescription()}";
    let result = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .output()
        .map_err(|source| VedError::Process {
            program: "powershell.exe".into(),
            source,
        })?;
    if !result.status.success() {
        return Err(message("SAPI voice discovery failed"));
    }
    Ok(String::from_utf8_lossy(&result.stdout)
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| VoiceInfo {
            provider: "sapi".into(),
            key: line.trim().to_owned(),
            name: line.trim().to_owned(),
            language: None,
        })
        .collect())
}

#[cfg(not(windows))]
fn sapi_voices() -> Result<Vec<VoiceInfo>> {
    Err(message("SAPI is only available on Windows"))
}

fn http_voices(provider: &str, port: u16) -> Result<Vec<VoiceInfo>> {
    let result = Command::new("curl")
        .args(["-sS", "-f", &format!("http://127.0.0.1:{port}/speakers")])
        .output()
        .map_err(|source| VedError::Process {
            program: "curl".into(),
            source,
        })?;
    if !result.status.success() {
        return Err(message(format!("{provider} is unavailable")));
    }
    let data: serde_json::Value = serde_json::from_slice(&result.stdout).map_err(|error| {
        message(format!(
            "{provider} returned invalid speakers JSON: {error}"
        ))
    })?;
    let mut voices = Vec::new();
    for speaker in data.as_array().into_iter().flatten() {
        let speaker_name = speaker["name"].as_str().unwrap_or("voice");
        for style in speaker["styles"].as_array().into_iter().flatten() {
            let Some(id) = style["id"].as_u64() else {
                continue;
            };
            let style_name = style["name"].as_str().unwrap_or("default");
            voices.push(VoiceInfo {
                provider: provider.into(),
                key: id.to_string(),
                name: format!("{speaker_name} / {style_name}"),
                language: Some("ja-JP".into()),
            });
        }
    }
    Ok(voices)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    version: u32,
    #[serde(default)]
    defaults: AudioDefaults,
    clips: Vec<AudioEntry>,
    #[serde(default)]
    ducking: Vec<DuckingEntry>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct AudioDefaults {
    volume: Option<f64>,
    speed: Option<f64>,
    fade_in: Option<f64>,
    fade_out: Option<f64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AudioEntry {
    key: String,
    track: String,
    file: Option<PathBuf>,
    tts: Option<TtsRequest>,
    at: TimeSpec,
    to: Option<EndSpec>,
    source_in: Option<f64>,
    source_out: Option<f64>,
    speed: Option<f64>,
    #[serde(default)]
    r#loop: bool,
    volume: Option<f64>,
    #[serde(default)]
    mute: bool,
    fade_in: Option<f64>,
    fade_out: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TtsRequest {
    provider: String,
    voice: String,
    text: String,
    #[serde(default = "one")]
    rate: f64,
    #[serde(default)]
    pitch: f64,
    #[serde(default = "one")]
    intonation: f64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum TimeSpec {
    Seconds(f64),
    Source {
        clip: String,
        source: f64,
        #[serde(default)]
        offset: f64,
    },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum EndSpec {
    Seconds(f64),
    Name(String),
    Source {
        clip: String,
        source: f64,
        #[serde(default)]
        offset: f64,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DuckingEntry {
    key: String,
    target_track: String,
    trigger_tracks: Vec<String>,
    #[serde(default = "default_reduction")]
    reduction_db: f64,
    #[serde(default = "default_attack")]
    attack: f64,
    #[serde(default = "default_release")]
    release: f64,
}

pub fn sync(
    project_path: &Path,
    manifest_path: &Path,
    replace_tracks: &[String],
    replace_ducking: bool,
) -> Result<SyncSummary> {
    for track in replace_tracks {
        validate_agent_key(track, "audio track")?;
    }
    let bytes = fs::read(manifest_path).map_err(|source| VedError::Io {
        path: manifest_path.to_path_buf(),
        source,
    })?;
    let manifest: Manifest = serde_json::from_slice(&bytes).map_err(|source| VedError::Json {
        path: manifest_path.to_path_buf(),
        source,
    })?;
    if manifest.version != 1 {
        return Err(message(format!(
            "unsupported audio manifest version {}",
            manifest.version
        )));
    }
    validate_manifest_keys(&manifest)?;

    let before = project::load(project_path)?;
    let mut candidate = before.clone();
    let manifest_base = manifest_path.parent().unwrap_or_else(|| Path::new("."));
    let mut summary = SyncSummary {
        changed: false,
        media_created: 0,
        media_updated: 0,
        audio_created: 0,
        audio_updated: 0,
        audio_removed: 0,
        ducking_created: 0,
        ducking_updated: 0,
        ducking_removed: 0,
    };
    let mut present_audio = HashSet::new();

    for entry in &manifest.clips {
        let source_path = match (&entry.file, &entry.tts) {
            (Some(file), None) => absolute_from(manifest_base, file),
            (None, Some(tts)) => generate_tts(project_path, tts)?,
            _ => {
                return Err(message(format!(
                    "audio entry {}/{} requires exactly one of file or tts",
                    entry.track, entry.key
                )));
            }
        };
        let media_id = import_or_refresh(project_path, &mut candidate, &source_path, &mut summary)?;
        let start = resolve_time(&candidate, &entry.at)?;
        let end = entry
            .to
            .as_ref()
            .map(|value| resolve_end(&candidate, value))
            .transpose()?;
        let speed = entry.speed.or(manifest.defaults.speed).unwrap_or(1.0);
        validate_speed(speed)?;
        let desired = AudioClip {
            id: String::new(),
            key: Some(entry.key.clone()),
            track: Some(entry.track.clone()),
            media_id,
            start,
            end,
            source_in: entry.source_in.unwrap_or(0.0),
            source_out: entry.source_out,
            speed,
            r#loop: entry.r#loop,
            volume: entry.volume.or(manifest.defaults.volume).unwrap_or(1.0),
            mute: entry.mute,
            fade_in: entry.fade_in.or(manifest.defaults.fade_in).unwrap_or(0.0),
            fade_out: entry.fade_out.or(manifest.defaults.fade_out).unwrap_or(0.0),
        };
        let identity = (entry.track.clone(), entry.key.clone());
        present_audio.insert(identity.clone());
        if let Some(existing) = candidate.audio_clips.iter_mut().find(|item| {
            item.track.as_deref() == Some(identity.0.as_str())
                && item.key.as_deref() == Some(identity.1.as_str())
        }) {
            let id = existing.id.clone();
            let mut updated = desired;
            updated.id = id;
            if *existing != updated {
                *existing = updated;
                summary.audio_updated += 1;
            }
        } else {
            let mut created = desired;
            created.id = candidate.next_audio_id();
            candidate.audio_clips.push(created);
            summary.audio_created += 1;
        }
    }

    if !replace_tracks.is_empty() {
        let before_len = candidate.audio_clips.len();
        candidate.audio_clips.retain(|item| {
            let Some(track) = item.track.as_deref() else {
                return true;
            };
            if !replace_tracks.iter().any(|value| value == track) {
                return true;
            }
            let Some(key) = item.key.as_deref() else {
                return true;
            };
            present_audio.contains(&(track.to_owned(), key.to_owned()))
        });
        summary.audio_removed = before_len - candidate.audio_clips.len();
    }

    let mut present_ducking = HashSet::new();
    for entry in &manifest.ducking {
        present_ducking.insert(entry.key.clone());
        let desired = AudioDucking {
            id: String::new(),
            key: entry.key.clone(),
            target_track: entry.target_track.clone(),
            trigger_tracks: entry.trigger_tracks.clone(),
            reduction_db: entry.reduction_db,
            attack: entry.attack,
            release: entry.release,
        };
        if let Some(existing) = candidate
            .audio_ducking
            .iter_mut()
            .find(|item| item.key == entry.key)
        {
            let id = existing.id.clone();
            let mut updated = desired;
            updated.id = id;
            if *existing != updated {
                *existing = updated;
                summary.ducking_updated += 1;
            }
        } else {
            let mut created = desired;
            created.id = candidate.next_ducking_id();
            candidate.audio_ducking.push(created);
            summary.ducking_created += 1;
        }
    }
    if replace_ducking {
        let before_len = candidate.audio_ducking.len();
        candidate
            .audio_ducking
            .retain(|item| present_ducking.contains(&item.key));
        summary.ducking_removed = before_len - candidate.audio_ducking.len();
    }

    candidate.validate()?;
    summary.changed = candidate != before;
    if summary.changed {
        history::record_before_edit(project_path, &before)?;
        project::save(project_path, &candidate)?;
    }
    Ok(summary)
}

fn validate_manifest_keys(manifest: &Manifest) -> Result<()> {
    let mut audio = HashSet::new();
    for entry in &manifest.clips {
        validate_agent_key(&entry.key, "audio key")?;
        validate_agent_key(&entry.track, "audio track")?;
        if !audio.insert((entry.track.as_str(), entry.key.as_str())) {
            return Err(message(format!(
                "duplicate manifest audio key {}/{}",
                entry.track, entry.key
            )));
        }
    }
    let mut ducking = HashSet::new();
    for entry in &manifest.ducking {
        validate_agent_key(&entry.key, "ducking key")?;
        if !ducking.insert(entry.key.as_str()) {
            return Err(message(format!(
                "duplicate manifest ducking key {}",
                entry.key
            )));
        }
    }
    Ok(())
}

fn resolve_time(project: &Project, value: &TimeSpec) -> Result<f64> {
    match value {
        TimeSpec::Seconds(value) => validate_resolved_time(*value),
        TimeSpec::Source {
            clip,
            source,
            offset,
        } => resolve_source_time(project, clip, *source, *offset),
    }
}

fn resolve_end(project: &Project, value: &EndSpec) -> Result<f64> {
    match value {
        EndSpec::Seconds(value) => validate_resolved_time(*value),
        EndSpec::Name(value) if value.eq_ignore_ascii_case("timeline") => {
            Ok(timeline::duration(project))
        }
        EndSpec::Name(value) => Err(message(format!("invalid audio end {value:?}"))),
        EndSpec::Source {
            clip,
            source,
            offset,
        } => resolve_source_time(project, clip, *source, *offset),
    }
}

fn resolve_source_time(project: &Project, clip_id: &str, source: f64, offset: f64) -> Result<f64> {
    if !source.is_finite() || !offset.is_finite() {
        return Err(message("source-linked time must be finite"));
    }
    let item = timeline::resolve(project)
        .into_iter()
        .find(|item| item.item.id() == clip_id)
        .ok_or_else(|| message(format!("video clip {clip_id} does not exist")))?;
    let clip = item
        .item
        .clip()
        .ok_or_else(|| message(format!("{clip_id} is a hold, not a video clip")))?;
    if source < clip.source_in - 0.000_001 || source > clip.source_out + 0.000_001 {
        return Err(message(format!(
            "source time {source:.3} is outside clip {clip_id} range {:.3}..={:.3}",
            clip.source_in, clip.source_out
        )));
    }
    validate_resolved_time(item.timeline_start + (source - clip.source_in) / clip.speed + offset)
}

fn validate_resolved_time(value: f64) -> Result<f64> {
    if !value.is_finite() || value < 0.0 {
        return Err(message(
            "resolved timeline time must be finite and non-negative",
        ));
    }
    Ok(value)
}

fn import_or_refresh(
    project_path: &Path,
    project: &mut Project,
    path: &Path,
    summary: &mut SyncSummary,
) -> Result<String> {
    let canonical = fs::canonicalize(path).map_err(|source| VedError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    if !canonical.is_file() {
        return Err(message(format!("media is not a file: {}", path.display())));
    }
    let stored = stored_media_path(project_path, &canonical)?;
    let (kind, probe) = ffmpeg::probe(&canonical)?;
    if kind != MediaKind::Audio && !probe.has_audio {
        return Err(message(format!("{} has no audio stream", path.display())));
    }
    if let Some(index) = project
        .media
        .iter()
        .position(|item| media_paths_equal(&item.path, &stored))
    {
        if project.media[index].kind != kind {
            return Err(message(format!(
                "media {} changed kind",
                project.media[index].id
            )));
        }
        if project.media[index].probe != probe {
            project.media[index].probe = probe;
            summary.media_updated += 1;
        }
        return Ok(project.media[index].id.clone());
    }
    let id = project.next_media_id();
    project.media.push(Media {
        id: id.clone(),
        path: stored,
        kind,
        probe,
    });
    summary.media_created += 1;
    Ok(id)
}

fn generate_tts(project_path: &Path, request: &TtsRequest) -> Result<PathBuf> {
    if request.text.contains('\0') || request.text.is_empty() {
        return Err(message("TTS text must be non-empty and contain no NUL"));
    }
    if !request.rate.is_finite()
        || request.rate <= 0.0
        || !request.pitch.is_finite()
        || !request.intonation.is_finite()
        || request.intonation <= 0.0
    {
        return Err(message("TTS rate, pitch, and intonation are invalid"));
    }
    let canonical = serde_json::to_vec(request)
        .map_err(|error| message(format!("could not serialize TTS request: {error}")))?;
    let hash = sha256_hex(&canonical);
    let base = project_path.parent().unwrap_or_else(|| Path::new("."));
    let cache = base.join(".ved").join("cache").join("tts");
    fs::create_dir_all(&cache).map_err(|source| VedError::Io {
        path: cache.clone(),
        source,
    })?;
    let output = cache.join(format!("{hash}.wav"));
    if output.is_file() {
        if ffmpeg::probe(&output)
            .ok()
            .and_then(|(_, probe)| probe.duration)
            .is_some_and(|duration| duration > 0.0)
        {
            return Ok(output);
        }
        fs::remove_file(&output).map_err(|source| VedError::Io {
            path: output.clone(),
            source,
        })?;
    }
    let nonce = format!(
        "{}.{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );
    let raw = cache.join(format!(".{hash}.{nonce}.raw.wav"));
    let normalized = cache.join(format!(".{hash}.{nonce}.normalized.wav"));
    let result = (|| {
        match request.provider.to_ascii_lowercase().as_str() {
            "sapi" => generate_sapi(request, &raw)?,
            "voicevox" => generate_voicevox(request, 50021, &raw)?,
            "aivisspeech" | "aivis" => generate_voicevox(request, 10101, &raw)?,
            value => return Err(message(format!("unsupported TTS provider {value:?}"))),
        }
        let normalized_output = Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
            .arg(&raw)
            .args(["-ar", "48000", "-ac", "2", "-c:a", "pcm_s16le"])
            .arg(&normalized)
            .output()
            .map_err(|source| VedError::Process {
                program: "ffmpeg".into(),
                source,
            })?;
        if !normalized_output.status.success() {
            return Err(message(format!(
                "ffmpeg failed to normalize generated TTS audio: {}",
                String::from_utf8_lossy(&normalized_output.stderr).trim()
            )));
        }
        let (kind, probe) = ffmpeg::probe(&normalized)?;
        if kind != MediaKind::Audio
            || !probe.has_audio
            || !probe.duration.is_some_and(|duration| duration > 0.0)
        {
            return Err(message("generated TTS cache entry is not valid audio"));
        }
        if output.is_file() {
            return Ok(());
        }
        match fs::rename(&normalized, &output) {
            Ok(()) => Ok(()),
            Err(_) if output.is_file() => Ok(()),
            Err(source) => Err(VedError::Io {
                path: output.clone(),
                source,
            }),
        }
    })();
    let _ = fs::remove_file(&raw);
    let _ = fs::remove_file(&normalized);
    result?;
    Ok(output)
}

#[cfg(windows)]
fn generate_sapi(request: &TtsRequest, output: &Path) -> Result<()> {
    let parent = output.parent().unwrap();
    let nonce = output
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("tts");
    let text_path = parent.join(format!(".{nonce}-text.txt"));
    let script_path = parent.join(format!(".{nonce}-sapi.ps1"));
    fs::write(&text_path, request.text.as_bytes()).map_err(|source| VedError::Io {
        path: text_path.clone(),
        source,
    })?;
    let rate = ((request.rate - 1.0) * 5.0).round().clamp(-10.0, 10.0) as i32;
    if request.pitch != 0.0 || request.intonation != 1.0 {
        let _ = fs::remove_file(&text_path);
        return Err(message(
            "SAPI does not support non-default pitch or intonation",
        ));
    }
    let script = "param([string]$TextPath,[string]$OutputPath,[string]$Voice,[int]$Rate)\n$ErrorActionPreference='Stop'\n$text=[IO.File]::ReadAllText($TextPath)\n$v=New-Object -ComObject SAPI.SpVoice\n$choice=$v.GetVoices()|Where-Object{$_.GetDescription() -eq $Voice}|Select-Object -First 1\nif($null -eq $choice){throw 'voice not found'}\n$v.Voice=$choice\n$v.Rate=$Rate\n$s=New-Object -ComObject SAPI.SpFileStream\n$s.Format.Type=18\n$s.Open($OutputPath,3,$false)\n$v.AudioOutputStream=$s\n[void]$v.Speak($text,0)\n[void]$v.WaitUntilDone(60000)\n$v.AudioOutputStream=$null\n$s.Close()\n";
    fs::write(&script_path, script.as_bytes()).map_err(|source| VedError::Io {
        path: script_path.clone(),
        source,
    })?;
    let text_path = fs::canonicalize(&text_path).map_err(|source| VedError::Io {
        path: text_path.clone(),
        source,
    })?;
    let script_path = fs::canonicalize(&script_path).map_err(|source| VedError::Io {
        path: script_path.clone(),
        source,
    })?;
    let output_path = fs::canonicalize(parent)
        .map_err(|source| VedError::Io {
            path: parent.to_path_buf(),
            source,
        })?
        .join(output.file_name().unwrap());
    let result = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(&script_path)
        .arg(&text_path)
        .arg(&output_path)
        .arg(&request.voice)
        .arg(rate.to_string())
        .output()
        .map_err(|source| VedError::Process {
            program: "powershell.exe".into(),
            source,
        })?;
    let _ = fs::remove_file(&text_path);
    let _ = fs::remove_file(&script_path);
    if !result.status.success() {
        return Err(message(format!(
            "SAPI synthesis failed: {}",
            String::from_utf8_lossy(&result.stderr).trim()
        )));
    }
    Ok(())
}

#[cfg(not(windows))]
fn generate_sapi(_request: &TtsRequest, _output: &Path) -> Result<()> {
    Err(message("SAPI is only available on Windows"))
}

fn generate_voicevox(request: &TtsRequest, port: u16, output: &Path) -> Result<()> {
    let speaker = request
        .voice
        .parse::<u32>()
        .map_err(|_| message("VOICEVOX/Aivis voice must be a numeric speaker ID"))?;
    let query_path = output.with_extension("query.json");
    let query_url = format!(
        "http://127.0.0.1:{port}/audio_query?text={}&speaker={speaker}",
        percent_encode(&request.text)
    );
    let result = (|| {
        run_curl(&[
            "-sS",
            "-f",
            "-X",
            "POST",
            &query_url,
            "-o",
            query_path.to_string_lossy().as_ref(),
        ])?;
        let mut query: serde_json::Value =
            serde_json::from_slice(&fs::read(&query_path).map_err(|source| VedError::Io {
                path: query_path.clone(),
                source,
            })?)
            .map_err(|source| VedError::Json {
                path: query_path.clone(),
                source,
            })?;
        query["speedScale"] = serde_json::Value::from(request.rate);
        query["pitchScale"] = serde_json::Value::from(request.pitch);
        query["intonationScale"] = serde_json::Value::from(request.intonation);
        fs::write(
            &query_path,
            serde_json::to_vec(&query).map_err(|error| message(error.to_string()))?,
        )
        .map_err(|source| VedError::Io {
            path: query_path.clone(),
            source,
        })?;
        let synthesis_url = format!("http://127.0.0.1:{port}/synthesis?speaker={speaker}");
        let data = format!("@{}", query_path.to_string_lossy());
        run_curl_os(&[
            std::ffi::OsString::from("-sS"),
            std::ffi::OsString::from("-f"),
            std::ffi::OsString::from("-X"),
            std::ffi::OsString::from("POST"),
            std::ffi::OsString::from("-H"),
            std::ffi::OsString::from("Content-Type: application/json"),
            std::ffi::OsString::from("--data-binary"),
            std::ffi::OsString::from(data),
            std::ffi::OsString::from(synthesis_url),
            std::ffi::OsString::from("-o"),
            output.as_os_str().to_owned(),
        ])
    })();
    let _ = fs::remove_file(query_path);
    result
}

fn run_curl(args: &[&str]) -> Result<()> {
    run_curl_os(
        &args
            .iter()
            .map(std::ffi::OsString::from)
            .collect::<Vec<_>>(),
    )
}

fn run_curl_os(args: &[std::ffi::OsString]) -> Result<()> {
    let output = Command::new("curl")
        .args(args)
        .output()
        .map_err(|source| VedError::Process {
            program: "curl".into(),
            source,
        })?;
    if !output.status.success() {
        return Err(message(format!(
            "local TTS request failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(())
}

fn percent_encode(value: &str) -> String {
    let mut output = String::new();
    for byte in value.as_bytes() {
        if byte.is_ascii_alphanumeric() || matches!(*byte, b'-' | b'_' | b'.' | b'~') {
            output.push(char::from(*byte));
        } else {
            output.push_str(&format!("%{byte:02X}"));
        }
    }
    output
}

fn absolute_from(base: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}

fn stored_media_path(project_path: &Path, canonical: &Path) -> Result<String> {
    let parent = project_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let base = fs::canonicalize(parent).map_err(|source| VedError::Io {
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

fn one() -> f64 {
    1.0
}
fn default_reduction() -> f64 {
    12.0
}
fn default_attack() -> f64 {
    0.12
}
fn default_release() -> f64 {
    0.35
}

#[cfg(test)]
mod tests {
    use super::{resolve_source_time, sha256_hex};
    use crate::project::{Canvas, Clip, Media, MediaKind, MediaProbe, Project};

    #[test]
    fn cache_hash_is_sha256() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn source_linked_time_accounts_for_trim_speed_and_prior_clips() {
        let mut project = Project::new(Canvas {
            width: 1080,
            height: 1920,
            fps: 30,
        });
        project.media.push(Media {
            id: "m1".into(),
            path: "video.mp4".into(),
            kind: MediaKind::Video,
            probe: MediaProbe {
                duration: Some(10.0),
                width: Some(1080),
                height: Some(1920),
                fps: Some(30.0),
                has_audio: false,
                video_codec: Some("h264".into()),
                audio_codec: None,
            },
        });
        project.timeline.push(
            Clip {
                id: "c1".into(),
                media_id: "m1".into(),
                source_in: 0.0,
                source_out: 1.0,
                speed: 1.0,
                mute: true,
                volume: 1.0,
            }
            .into(),
        );
        project.timeline.push(
            Clip {
                id: "c2".into(),
                media_id: "m1".into(),
                source_in: 2.0,
                source_out: 8.0,
                speed: 2.0,
                mute: true,
                volume: 1.0,
            }
            .into(),
        );
        assert_eq!(
            resolve_source_time(&project, "c2", 4.0, 0.25).unwrap(),
            2.25
        );
    }
}
