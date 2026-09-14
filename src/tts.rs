use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use reqwest::StatusCode;
use reqwest::blocking::{Client, Response};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{Result, VedError};
use crate::ffmpeg;
use crate::project::{MediaKind, MediaProbe, Project, TtsProviderKind, VoiceClip};

const VOICEVOX_ENDPOINT: &str = "http://127.0.0.1:50021";
const AIVIS_ENDPOINT: &str = "http://127.0.0.1:10101";

#[derive(Debug, thiserror::Error)]
pub enum TtsError {
    #[error("engine not running: {provider} at {endpoint}")]
    EngineNotRunning { provider: String, endpoint: String },
    #[error("connection refused: {provider} at {endpoint}")]
    ConnectionRefused { provider: String, endpoint: String },
    #[error("invalid voice ID: {0}")]
    InvalidVoice(String),
    #[error("invalid parameter: {0}")]
    InvalidParameter(String),
    #[error("synthesis failed: {0}")]
    SynthesisFailed(String),
    #[error("invalid returned audio: {0}")]
    InvalidReturnedAudio(String),
    #[error("cache write failure: {0}")]
    CacheWriteFailure(String),
    #[error("cache validation failure: {0}")]
    CacheValidationFailure(String),
}

impl From<TtsError> for VedError {
    fn from(value: TtsError) -> Self {
        VedError::Message(value.to_string())
    }
}

type TtsResult<T> = std::result::Result<T, TtsError>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VoiceInfo {
    pub provider: String,
    pub voice_id: String,
    pub speaker_id: Option<String>,
    pub display_name: String,
    pub style_name: String,
}

#[derive(Debug, Clone)]
pub struct SynthesizedAudio {
    pub engine_identity: String,
    pub cache_key: String,
    pub endpoint: String,
    pub stored_path: String,
    pub probe: MediaProbe,
}

#[derive(Debug, Clone)]
pub struct DoctorStatus {
    pub name: &'static str,
    pub endpoint: String,
    pub available: bool,
    pub detail: String,
}

#[derive(Debug, Clone)]
pub struct SynthesisRequest<'a> {
    pub voice: &'a str,
    pub text: &'a str,
    pub speed: f64,
    pub pitch: f64,
}

trait TtsProvider {
    fn kind(&self) -> TtsProviderKind;
    fn display_name(&self) -> &'static str;
    fn default_endpoint(&self) -> &'static str;
    fn validate_parameters(&self, request: &SynthesisRequest<'_>) -> TtsResult<()>;
    fn list_voices(&self, endpoint: &str) -> TtsResult<Vec<VoiceInfo>>;
    fn engine_identity(&self, endpoint: &str) -> TtsResult<String>;
    fn synthesize(&self, endpoint: &str, request: &SynthesisRequest<'_>) -> TtsResult<Vec<u8>>;
}

struct VoicevoxProvider;
struct AivisSpeechProvider;

impl TtsProvider for VoicevoxProvider {
    fn kind(&self) -> TtsProviderKind {
        TtsProviderKind::Voicevox
    }

    fn display_name(&self) -> &'static str {
        "VOICEVOX"
    }

    fn default_endpoint(&self) -> &'static str {
        VOICEVOX_ENDPOINT
    }

    fn validate_parameters(&self, request: &SynthesisRequest<'_>) -> TtsResult<()> {
        validate_common_request(request)?;
        if !(0.5..=2.0).contains(&request.speed) {
            return Err(TtsError::InvalidParameter(
                "VOICEVOX speed must be between 0.5 and 2.0".into(),
            ));
        }
        if !(-0.15..=0.15).contains(&request.pitch) {
            return Err(TtsError::InvalidParameter(
                "VOICEVOX pitch must be between -0.15 and 0.15".into(),
            ));
        }
        Ok(())
    }

    fn list_voices(&self, endpoint: &str) -> TtsResult<Vec<VoiceInfo>> {
        list_speakers(self, endpoint)
    }

    fn engine_identity(&self, endpoint: &str) -> TtsResult<String> {
        version_identity(self, endpoint)
    }

    fn synthesize(&self, endpoint: &str, request: &SynthesisRequest<'_>) -> TtsResult<Vec<u8>> {
        self.validate_parameters(request)?;
        synthesize_voicevox_compatible(self, endpoint, request, QueryFlavor::Voicevox)
    }
}

impl TtsProvider for AivisSpeechProvider {
    fn kind(&self) -> TtsProviderKind {
        TtsProviderKind::Aivis
    }

    fn display_name(&self) -> &'static str {
        "AivisSpeech"
    }

    fn default_endpoint(&self) -> &'static str {
        AIVIS_ENDPOINT
    }

    fn validate_parameters(&self, request: &SynthesisRequest<'_>) -> TtsResult<()> {
        validate_common_request(request)?;
        if !(0.5..=2.0).contains(&request.speed) {
            return Err(TtsError::InvalidParameter(
                "AivisSpeech speed must be between 0.5 and 2.0".into(),
            ));
        }
        if !(-0.15..=0.15).contains(&request.pitch) {
            return Err(TtsError::InvalidParameter(
                "AivisSpeech pitch must be between -0.15 and 0.15".into(),
            ));
        }
        Ok(())
    }

    fn list_voices(&self, endpoint: &str) -> TtsResult<Vec<VoiceInfo>> {
        list_speakers(self, endpoint)
    }

    fn engine_identity(&self, endpoint: &str) -> TtsResult<String> {
        version_identity(self, endpoint)
    }

    fn synthesize(&self, endpoint: &str, request: &SynthesisRequest<'_>) -> TtsResult<Vec<u8>> {
        self.validate_parameters(request)?;
        // AivisSpeech is broadly VOICEVOX-API compatible, but AudioQuery semantics differ.
        // Keep its query mutation in this provider boundary so future Aivis-specific fields do
        // not leak into VOICEVOX behavior.
        synthesize_voicevox_compatible(self, endpoint, request, QueryFlavor::Aivis)
    }
}

fn provider(kind: TtsProviderKind) -> Box<dyn TtsProvider> {
    match kind {
        TtsProviderKind::Voicevox => Box::new(VoicevoxProvider),
        TtsProviderKind::Aivis => Box::new(AivisSpeechProvider),
    }
}

pub fn list_voices(kind: TtsProviderKind, endpoint: Option<&str>) -> Result<Vec<VoiceInfo>> {
    let provider = provider(kind);
    let endpoint = normalize_endpoint(endpoint.unwrap_or(provider.default_endpoint()))?;
    provider.list_voices(&endpoint).map_err(Into::into)
}

pub fn doctor() -> Vec<DoctorStatus> {
    [TtsProviderKind::Voicevox, TtsProviderKind::Aivis]
        .into_iter()
        .map(|kind| {
            let provider = provider(kind);
            let endpoint = provider.default_endpoint().to_owned();
            match provider.engine_identity(&endpoint).and_then(|identity| {
                provider
                    .list_voices(&endpoint)
                    .map(|voices| (identity, voices.len()))
            }) {
                Ok((identity, voice_count)) => DoctorStatus {
                    name: provider.display_name(),
                    endpoint,
                    available: true,
                    detail: format!("{identity}; {voice_count} voices"),
                },
                Err(error) => DoctorStatus {
                    name: provider.display_name(),
                    endpoint,
                    available: false,
                    detail: error.to_string(),
                },
            }
        })
        .collect()
}

pub fn synthesize_cached(
    project_path: &Path,
    kind: TtsProviderKind,
    endpoint: Option<&str>,
    request: &SynthesisRequest<'_>,
) -> Result<SynthesizedAudio> {
    let provider = provider(kind);
    provider
        .validate_parameters(request)
        .map_err(VedError::from)?;
    let endpoint = normalize_endpoint(endpoint.unwrap_or(provider.default_endpoint()))?;
    let engine_identity = provider
        .engine_identity(&endpoint)
        .map_err(VedError::from)?;
    let cache_key = cache_key(kind, &engine_identity, request)?;
    let cache_root = cache_root(project_path)?;
    let target = cache_root.join(format!("{cache_key}.wav"));

    if target.is_file() {
        match validate_wav(&target) {
            Ok(probe) => {
                return Ok(SynthesizedAudio {
                    engine_identity,
                    cache_key,
                    endpoint,
                    stored_path: stored_cache_path(&target, project_path),
                    probe,
                });
            }
            Err(_) => {
                fs::remove_file(&target).map_err(|error| {
                    TtsError::CacheWriteFailure(format!(
                        "could not remove invalid cache {}: {error}",
                        target.display()
                    ))
                })?;
            }
        }
    }

    let bytes = provider
        .synthesize(&endpoint, request)
        .map_err(VedError::from)?;
    if bytes.is_empty() {
        return Err(TtsError::InvalidReturnedAudio("engine returned zero bytes".into()).into());
    }

    let temp = cache_root.join(format!(
        ".{cache_key}.{}.{}.tmp.wav",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let write_result = (|| -> TtsResult<MediaProbe> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(|error| TtsError::CacheWriteFailure(error.to_string()))?;
        file.write_all(&bytes)
            .map_err(|error| TtsError::CacheWriteFailure(error.to_string()))?;
        file.sync_all()
            .map_err(|error| TtsError::CacheWriteFailure(error.to_string()))?;
        drop(file);
        validate_wav(&temp)
    })();

    let probe = match write_result {
        Ok(probe) => probe,
        Err(error) => {
            let _ = fs::remove_file(&temp);
            return Err(error.into());
        }
    };

    match fs::rename(&temp, &target) {
        Ok(()) => {}
        Err(error) if target.is_file() && validate_wav(&target).is_ok() => {
            let _ = fs::remove_file(&temp);
            let target_probe = validate_wav(&target).map_err(VedError::from)?;
            return Ok(SynthesizedAudio {
                engine_identity,
                cache_key,
                endpoint,
                stored_path: stored_cache_path(&target, project_path),
                probe: target_probe,
            });
        }
        Err(error) => {
            let _ = fs::remove_file(&temp);
            return Err(TtsError::CacheWriteFailure(format!(
                "could not publish {}: {error}",
                target.display()
            ))
            .into());
        }
    }

    Ok(SynthesizedAudio {
        engine_identity,
        cache_key,
        endpoint,
        stored_path: stored_cache_path(&target, project_path),
        probe,
    })
}

pub fn ensure_project_audio(project_path: &Path, project: &mut Project) -> Result<()> {
    let voices = project.voice_clips.clone();
    for voice in voices {
        let expected = cache_root(project_path)?.join(format!("{}.wav", voice.cache_key));
        let synthesized = match validate_wav(&expected) {
            Ok(probe) => SynthesizedAudio {
                engine_identity: voice.engine_identity.clone(),
                cache_key: voice.cache_key.clone(),
                endpoint: voice.endpoint.clone(),
                stored_path: stored_cache_path(&expected, project_path),
                probe,
            },
            Err(_) => synthesize_cached(
                project_path,
                voice.provider,
                Some(&voice.endpoint),
                &SynthesisRequest {
                    voice: &voice.voice,
                    text: &voice.text,
                    speed: voice.speed,
                    pitch: voice.pitch,
                },
            )?,
        };
        update_materialized_voice(project, &voice, &synthesized)?;
    }
    project.validate()
}

fn update_materialized_voice(
    project: &mut Project,
    original: &VoiceClip,
    synthesized: &SynthesizedAudio,
) -> Result<()> {
    let audio = project
        .audio_clips
        .iter()
        .find(|item| item.id == original.audio_clip_id)
        .ok_or_else(|| {
            VedError::Message(format!(
                "audio clip {} does not exist",
                original.audio_clip_id
            ))
        })?;
    let media_id = audio.media_id.clone();
    let media = project
        .media
        .iter_mut()
        .find(|item| item.id == media_id)
        .ok_or_else(|| VedError::Message(format!("media {media_id} does not exist")))?;
    media.path = synthesized.stored_path.clone();
    media.kind = MediaKind::Audio;
    media.probe = synthesized.probe.clone();

    if let Some(voice) = project
        .voice_clips
        .iter_mut()
        .find(|item| item.id == original.id)
    {
        voice.engine_identity = synthesized.engine_identity.clone();
        voice.cache_key = synthesized.cache_key.clone();
        voice.endpoint = synthesized.endpoint.clone();
    }
    Ok(())
}

pub fn cache_key(
    kind: TtsProviderKind,
    engine_identity: &str,
    request: &SynthesisRequest<'_>,
) -> Result<String> {
    provider(kind)
        .validate_parameters(request)
        .map_err(VedError::from)?;
    #[derive(Serialize)]
    struct Identity<'a> {
        schema: u8,
        provider: &'a str,
        engine_identity: &'a str,
        voice: &'a str,
        text: &'a str,
        speed: String,
        pitch: String,
    }
    let identity = Identity {
        schema: 1,
        provider: kind.as_str(),
        engine_identity,
        voice: request.voice,
        text: request.text,
        speed: canonical_float(request.speed),
        pitch: canonical_float(request.pitch),
    };
    let canonical = serde_json::to_vec(&identity)
        .map_err(|error| VedError::Message(format!("could not build TTS cache key: {error}")))?;
    let digest = Sha256::digest(canonical);
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

pub fn validate_wav(path: &Path) -> TtsResult<MediaProbe> {
    let metadata = fs::metadata(path).map_err(|error| {
        TtsError::CacheValidationFailure(format!("{}: {error}", path.display()))
    })?;
    if !metadata.is_file() || metadata.len() == 0 {
        return Err(TtsError::CacheValidationFailure(format!(
            "{} is empty or not a file",
            path.display()
        )));
    }
    let (kind, probe) = ffmpeg::probe(path).map_err(|error| {
        TtsError::CacheValidationFailure(format!("{}: {error}", path.display()))
    })?;
    if kind != MediaKind::Audio || !probe.has_audio {
        return Err(TtsError::InvalidReturnedAudio(format!(
            "{} has no audio stream",
            path.display()
        )));
    }
    if !probe
        .duration
        .is_some_and(|duration| duration.is_finite() && duration > 0.0)
    {
        return Err(TtsError::InvalidReturnedAudio(format!(
            "{} has no positive duration",
            path.display()
        )));
    }
    Ok(probe)
}

fn validate_common_request(request: &SynthesisRequest<'_>) -> TtsResult<()> {
    if request.voice.trim().is_empty() {
        return Err(TtsError::InvalidVoice(request.voice.into()));
    }
    if request.text.is_empty() || request.text.contains('\0') {
        return Err(TtsError::InvalidParameter(
            "text must be non-empty and must not contain NUL".into(),
        ));
    }
    if !request.speed.is_finite() || !request.pitch.is_finite() {
        return Err(TtsError::InvalidParameter(
            "speed and pitch must be finite".into(),
        ));
    }
    request
        .voice
        .parse::<u32>()
        .map_err(|_| TtsError::InvalidVoice(request.voice.into()))?;
    Ok(())
}

fn normalize_endpoint(value: &str) -> Result<String> {
    let normalized = value.trim().trim_end_matches('/');
    if normalized.is_empty() {
        return Err(TtsError::InvalidParameter("endpoint is empty".into()).into());
    }
    let url = reqwest::Url::parse(normalized)
        .map_err(|_| TtsError::InvalidParameter(format!("invalid endpoint {value:?}")))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(TtsError::InvalidParameter(format!("invalid endpoint {value:?}")).into());
    }
    Ok(normalized.to_owned())
}

fn client() -> TtsResult<Client> {
    Client::builder()
        .connect_timeout(Duration::from_millis(750))
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|error| TtsError::ConnectionRefused {
            provider: "TTS".into(),
            endpoint: error.to_string(),
        })
}

fn classify_send_error(
    provider: &dyn TtsProvider,
    endpoint: &str,
    error: reqwest::Error,
) -> TtsError {
    if error.is_connect() {
        let lower = error.to_string().to_ascii_lowercase();
        if lower.contains("refused") {
            TtsError::ConnectionRefused {
                provider: provider.display_name().into(),
                endpoint: endpoint.into(),
            }
        } else {
            TtsError::EngineNotRunning {
                provider: provider.display_name().into(),
                endpoint: endpoint.into(),
            }
        }
    } else {
        TtsError::SynthesisFailed(error.to_string())
    }
}

fn get(provider: &dyn TtsProvider, endpoint: &str, path: &str) -> TtsResult<Response> {
    client()?
        .get(format!("{endpoint}{path}"))
        .send()
        .map_err(|error| classify_send_error(provider, endpoint, error))
}

#[derive(Debug, Deserialize)]
struct SpeakerResponse {
    name: String,
    #[serde(default)]
    speaker_uuid: Option<String>,
    #[serde(default)]
    styles: Vec<SpeakerStyle>,
}

#[derive(Debug, Deserialize)]
struct SpeakerStyle {
    id: serde_json::Value,
    name: String,
}

fn list_speakers(provider: &dyn TtsProvider, endpoint: &str) -> TtsResult<Vec<VoiceInfo>> {
    let response = get(provider, endpoint, "/speakers")?;
    let status = response.status();
    if !status.is_success() {
        return Err(TtsError::SynthesisFailed(format!(
            "{} /speakers returned HTTP {status}",
            provider.display_name()
        )));
    }
    let speakers: Vec<SpeakerResponse> = response.json().map_err(|error| {
        TtsError::SynthesisFailed(format!("invalid /speakers response: {error}"))
    })?;
    let mut voices = Vec::new();
    for speaker in speakers {
        for style in speaker.styles {
            let Some(voice_id) = json_id(&style.id) else {
                continue;
            };
            voices.push(VoiceInfo {
                provider: provider.kind().as_str().into(),
                voice_id,
                speaker_id: speaker.speaker_uuid.clone(),
                display_name: speaker.name.clone(),
                style_name: style.name,
            });
        }
    }
    Ok(voices)
}

fn json_id(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::Number(number) => Some(number.to_string()),
        serde_json::Value::String(value) if !value.is_empty() => Some(value.clone()),
        _ => None,
    }
}

fn version_identity(provider: &dyn TtsProvider, endpoint: &str) -> TtsResult<String> {
    let response = get(provider, endpoint, "/version")?;
    let status = response.status();
    if !status.is_success() {
        return Err(TtsError::EngineNotRunning {
            provider: provider.display_name().into(),
            endpoint: endpoint.into(),
        });
    }
    let text = response.text().map_err(|error| {
        TtsError::SynthesisFailed(format!("invalid /version response: {error}"))
    })?;
    let version = serde_json::from_str::<String>(&text).unwrap_or_else(|_| text.trim().to_owned());
    if version.is_empty() {
        return Err(TtsError::SynthesisFailed("empty engine version".into()));
    }
    Ok(format!("{}:{version}", provider.kind().as_str()))
}

#[derive(Debug, Clone, Copy)]
enum QueryFlavor {
    Voicevox,
    Aivis,
}

fn synthesize_voicevox_compatible(
    provider: &dyn TtsProvider,
    endpoint: &str,
    request: &SynthesisRequest<'_>,
    flavor: QueryFlavor,
) -> TtsResult<Vec<u8>> {
    let speaker = request
        .voice
        .parse::<u32>()
        .map_err(|_| TtsError::InvalidVoice(request.voice.into()))?;
    let response = client()?
        .post(format!("{endpoint}/audio_query"))
        .query(&[
            ("speaker", speaker.to_string()),
            ("text", request.text.to_owned()),
        ])
        .send()
        .map_err(|error| classify_send_error(provider, endpoint, error))?;
    if !response.status().is_success() {
        return Err(classify_audio_query_status(
            response.status(),
            request.voice,
        ));
    }
    let mut query: serde_json::Value = response
        .json()
        .map_err(|error| TtsError::SynthesisFailed(format!("invalid audio_query JSON: {error}")))?;
    apply_query_parameters(&mut query, request, flavor)?;

    let response = client()?
        .post(format!("{endpoint}/synthesis"))
        .query(&[("speaker", speaker.to_string())])
        .json(&query)
        .send()
        .map_err(|error| classify_send_error(provider, endpoint, error))?;
    if !response.status().is_success() {
        return Err(TtsError::SynthesisFailed(format!(
            "{} /synthesis returned HTTP {}",
            provider.display_name(),
            response.status()
        )));
    }
    let bytes = response
        .bytes()
        .map_err(|error| TtsError::SynthesisFailed(error.to_string()))?;
    Ok(bytes.to_vec())
}

fn classify_audio_query_status(status: StatusCode, voice: &str) -> TtsError {
    if matches!(
        status,
        StatusCode::BAD_REQUEST | StatusCode::NOT_FOUND | StatusCode::UNPROCESSABLE_ENTITY
    ) {
        TtsError::InvalidVoice(voice.into())
    } else {
        TtsError::SynthesisFailed(format!("/audio_query returned HTTP {status}"))
    }
}

fn apply_query_parameters(
    query: &mut serde_json::Value,
    request: &SynthesisRequest<'_>,
    flavor: QueryFlavor,
) -> TtsResult<()> {
    let object = query
        .as_object_mut()
        .ok_or_else(|| TtsError::SynthesisFailed("audio_query was not a JSON object".into()))?;
    object.insert("speedScale".into(), serde_json::Value::from(request.speed));
    object.insert("pitchScale".into(), serde_json::Value::from(request.pitch));
    match flavor {
        QueryFlavor::Voicevox => {}
        QueryFlavor::Aivis => {
            // AivisSpeech reuses the `kana` field as normal source text. Preserve the engine's
            // value if present and populate it only when omitted/null so synthesis stays natural.
            if object.get("kana").is_none_or(serde_json::Value::is_null) {
                object.insert("kana".into(), serde_json::Value::from(request.text));
            }
        }
    }
    Ok(())
}

fn cache_root(project_path: &Path) -> Result<PathBuf> {
    let parent = project_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let components = [
        parent.join(".ved"),
        parent.join(".ved/cache"),
        parent.join(".ved/cache/tts"),
    ];
    for component in &components {
        if component.exists() {
            let metadata = fs::symlink_metadata(component).map_err(|error| {
                TtsError::CacheWriteFailure(format!("{}: {error}", component.display()))
            })?;
            if metadata.file_type().is_symlink() {
                return Err(TtsError::CacheWriteFailure(format!(
                    "cache path must not be a symlink: {}",
                    component.display()
                ))
                .into());
            }
        }
    }
    let root = components.last().unwrap().clone();
    fs::create_dir_all(&root)
        .map_err(|error| TtsError::CacheWriteFailure(format!("{}: {error}", root.display())))?;
    Ok(root)
}

fn stored_cache_path(path: &Path, project_path: &Path) -> String {
    let parent = project_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    path.strip_prefix(parent)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn canonical_float(value: f64) -> String {
    let value = format!("{value:.9}");
    let trimmed = value.trim_end_matches('0').trim_end_matches('.');
    if trimmed == "-0" {
        "0".into()
    } else {
        trimmed.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::{Arc, Mutex};
    use std::thread;

    #[test]
    fn cache_key_matrix_excludes_timeline_and_volume() {
        let base = SynthesisRequest {
            voice: "1",
            text: "日本語 & | > < % \\\n二行目",
            speed: 1.0,
            pitch: 0.0,
        };
        let key = cache_key(TtsProviderKind::Voicevox, "voicevox:1", &base).unwrap();
        assert_eq!(
            key,
            cache_key(TtsProviderKind::Voicevox, "voicevox:1", &base).unwrap()
        );
        for changed in [
            SynthesisRequest {
                text: "changed",
                ..base.clone()
            },
            SynthesisRequest {
                voice: "2",
                ..base.clone()
            },
            SynthesisRequest {
                speed: 1.1,
                ..base.clone()
            },
            SynthesisRequest {
                pitch: 0.1,
                ..base.clone()
            },
        ] {
            assert_ne!(
                key,
                cache_key(TtsProviderKind::Voicevox, "voicevox:1", &changed).unwrap()
            );
        }
        assert_ne!(
            key,
            cache_key(TtsProviderKind::Aivis, "aivis:1", &base).unwrap()
        );
        // Timeline position, playback volume, video/overlay state are intentionally absent from
        // SynthesisRequest, making accidental inclusion in this identity impossible.
    }

    #[test]
    fn providers_reject_invalid_parameters_before_http() {
        let bad_speed = SynthesisRequest {
            voice: "1",
            text: "x",
            speed: 2.1,
            pitch: 0.0,
        };
        assert!(VoicevoxProvider.validate_parameters(&bad_speed).is_err());
        assert!(AivisSpeechProvider.validate_parameters(&bad_speed).is_err());
        let bad_voice = SynthesisRequest {
            voice: "abc",
            text: "x",
            speed: 1.0,
            pitch: 0.0,
        };
        assert!(VoicevoxProvider.validate_parameters(&bad_voice).is_err());
    }

    #[test]
    fn mock_provider_lists_and_synthesizes_special_text() {
        if std::process::Command::new("ffprobe")
            .arg("-version")
            .output()
            .is_err()
        {
            return;
        }
        let (endpoint, requests, handle) = spawn_mock_engine();
        let voices = list_voices(TtsProviderKind::Voicevox, Some(&endpoint)).unwrap();
        assert_eq!(voices[0].voice_id, "1");
        let directory = tempfile::tempdir().unwrap();
        let project = directory.path().join("project.json");
        fs::write(&project, b"fixture").unwrap();
        let text = "\" ' & | > < % \\ 日本語\n改行";
        let first = synthesize_cached(
            &project,
            TtsProviderKind::Voicevox,
            Some(&endpoint),
            &SynthesisRequest {
                voice: "1",
                text,
                speed: 1.0,
                pitch: 0.0,
            },
        )
        .unwrap();
        assert!(Path::new(&first.stored_path).ends_with(format!("{}.wav", first.cache_key)));
        let before = requests
            .lock()
            .unwrap()
            .iter()
            .filter(|p| p.as_str() == "/synthesis")
            .count();
        let second = synthesize_cached(
            &project,
            TtsProviderKind::Voicevox,
            Some(&endpoint),
            &SynthesisRequest {
                voice: "1",
                text,
                speed: 1.0,
                pitch: 0.0,
            },
        )
        .unwrap();
        let after = requests
            .lock()
            .unwrap()
            .iter()
            .filter(|p| p.as_str() == "/synthesis")
            .count();
        assert_eq!(first.cache_key, second.cache_key);
        assert_eq!(before, after, "cache hit must not synthesize again");
        drop(handle);
    }

    #[test]
    fn invalid_cached_audio_is_rejected() {
        if std::process::Command::new("ffprobe")
            .arg("-version")
            .output()
            .is_err()
        {
            return;
        }
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("bad.wav");
        fs::write(&path, b"not wav").unwrap();
        assert!(validate_wav(&path).is_err());
    }

    fn spawn_mock_engine() -> (String, Arc<Mutex<Vec<String>>>, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = requests.clone();
        let handle = thread::spawn(move || {
            let started = std::time::Instant::now();
            while started.elapsed() < Duration::from_secs(5) {
                match listener.accept() {
                    Ok((mut stream, _)) => handle_request(&mut stream, &captured),
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });
        (format!("http://{address}"), requests, handle)
    }

    fn handle_request(stream: &mut TcpStream, requests: &Arc<Mutex<Vec<String>>>) {
        let mut buffer = vec![0_u8; 64 * 1024];
        let mut read = 0;
        loop {
            match stream.read(&mut buffer[read..]) {
                Ok(0) => break,
                Ok(count) => {
                    read += count;
                    if buffer[..read].windows(4).any(|value| value == b"\r\n\r\n") {
                        break;
                    }
                    if read == buffer.len() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        let head = String::from_utf8_lossy(&buffer[..read]);
        let first = head.lines().next().unwrap_or_default();
        let path = first.split_whitespace().nth(1).unwrap_or("/");
        let path_only = path.split('?').next().unwrap_or(path).to_owned();
        requests.lock().unwrap().push(path_only.clone());
        let (content_type, body) = match path_only.as_str() {
            "/version" => ("application/json", b"\"1.0.0\"".to_vec()),
            "/speakers" => (
                "application/json",
                br#"[{"name":"Mock","speaker_uuid":"uuid","styles":[{"id":1,"name":"Normal"}]}]"#
                    .to_vec(),
            ),
            "/audio_query" => (
                "application/json",
                br#"{"speedScale":1.0,"pitchScale":0.0,"volumeScale":1.0,"kana":null}"#.to_vec(),
            ),
            "/synthesis" => ("audio/wav", tiny_wav()),
            _ => ("text/plain", b"not found".to_vec()),
        };
        let status = if path_only == "/" {
            "404 Not Found"
        } else {
            "200 OK"
        };
        let header = format!(
            "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = stream.write_all(header.as_bytes());
        let _ = stream.write_all(&body);
    }

    fn tiny_wav() -> Vec<u8> {
        let sample_rate = 8_000_u32;
        let samples = 800_u32;
        let data_size = samples * 2;
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data_size).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16_u32.to_le_bytes());
        wav.extend_from_slice(&1_u16.to_le_bytes());
        wav.extend_from_slice(&1_u16.to_le_bytes());
        wav.extend_from_slice(&sample_rate.to_le_bytes());
        wav.extend_from_slice(&(sample_rate * 2).to_le_bytes());
        wav.extend_from_slice(&2_u16.to_le_bytes());
        wav.extend_from_slice(&16_u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_size.to_le_bytes());
        wav.resize((44 + data_size) as usize, 0);
        wav
    }
}
