use serde::{Deserialize, Serialize};

use crate::error::{Result, message};

pub const PROJECT_VERSION: u32 = 3;
pub const DEFAULT_SPEED: f64 = 1.0;
pub const DEFAULT_VOLUME: f64 = 1.0;
pub const DEFAULT_OPACITY: f64 = 1.0;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Project {
    pub version: u32,
    pub canvas: Canvas,
    pub media: Vec<Media>,
    pub timeline: Vec<Clip>,
    #[serde(default)]
    pub text_overlays: Vec<TextOverlay>,
    #[serde(default)]
    pub image_overlays: Vec<ImageOverlay>,
    #[serde(default)]
    pub audio_clips: Vec<AudioClip>,
    #[serde(default)]
    pub voice_clips: Vec<VoiceClip>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Canvas {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Media {
    pub id: String,
    pub path: String,
    pub kind: MediaKind,
    pub probe: MediaProbe,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MediaKind {
    Video,
    Image,
    Audio,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MediaProbe {
    pub duration: Option<f64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps: Option<f64>,
    pub has_audio: bool,
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Clip {
    pub id: String,
    pub media_id: String,
    pub source_in: f64,
    pub source_out: f64,
    #[serde(default = "default_speed", skip_serializing_if = "is_one")]
    pub speed: f64,
    #[serde(default, skip_serializing_if = "is_false")]
    pub mute: bool,
    #[serde(default = "default_volume", skip_serializing_if = "is_one")]
    pub volume: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TextOverlay {
    pub id: String,
    pub text: String,
    pub start: f64,
    pub end: f64,
    pub position: Position,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font: Option<String>,
    pub font_size: u32,
    pub color: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outline: Option<Outline>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
    #[serde(default = "default_opacity", skip_serializing_if = "is_one")]
    pub opacity: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Outline {
    pub color: String,
    pub width: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ImageOverlay {
    pub id: String,
    pub media_id: String,
    pub start: f64,
    pub end: f64,
    pub position: Position,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<Dimension>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<Dimension>,
    #[serde(default = "default_opacity", skip_serializing_if = "is_one")]
    pub opacity: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AudioClip {
    pub id: String,
    pub media_id: String,
    pub start: f64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub source_in: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_out: Option<f64>,
    #[serde(default = "default_volume", skip_serializing_if = "is_one")]
    pub volume: f64,
    #[serde(default, skip_serializing_if = "is_false")]
    pub mute: bool,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub fade_in: f64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub fade_out: f64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum TtsProviderKind {
    Voicevox,
    Aivis,
}

impl TtsProviderKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Voicevox => "voicevox",
            Self::Aivis => "aivis",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct VoiceClip {
    pub id: String,
    pub provider: TtsProviderKind,
    pub voice: String,
    pub text: String,
    pub speed: f64,
    pub pitch: f64,
    pub endpoint: String,
    pub engine_identity: String,
    pub cache_key: String,
    pub audio_clip_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Position {
    pub x: Coordinate,
    pub y: Coordinate,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(transparent)]
pub struct Coordinate(pub String);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(transparent)]
pub struct Dimension(pub String);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    X,
    Y,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum CoordinateValue {
    Start,
    Center,
    End,
    Percent(f64),
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum DimensionValue {
    Pixels(f64),
    Percent(f64),
}

impl Coordinate {
    pub fn parse(value: &str, axis: Axis) -> Result<Self> {
        Ok(Self(normalize_coordinate(value, axis)?))
    }
    pub fn value(&self, axis: Axis) -> Result<CoordinateValue> {
        coordinate_value(&self.0, axis)
    }
}

impl Dimension {
    pub fn parse(value: &str) -> Result<Self> {
        Ok(Self(normalize_dimension(value)?))
    }
    pub fn value(&self) -> Result<DimensionValue> {
        dimension_value(&self.0)
    }
}

impl Position {
    pub fn center() -> Self {
        Self {
            x: Coordinate("center".into()),
            y: Coordinate("center".into()),
        }
    }
}

impl Project {
    pub fn new(canvas: Canvas) -> Self {
        Self {
            version: PROJECT_VERSION,
            canvas,
            media: Vec::new(),
            timeline: Vec::new(),
            text_overlays: Vec::new(),
            image_overlays: Vec::new(),
            audio_clips: Vec::new(),
            voice_clips: Vec::new(),
        }
    }

    pub fn validate(&self) -> Result<()> {
        if self.version != PROJECT_VERSION {
            return Err(message(format!(
                "unsupported project version {}; this build supports version {PROJECT_VERSION}",
                self.version
            )));
        }
        if self.canvas.width == 0 || self.canvas.height == 0 || self.canvas.fps == 0 {
            return Err(message(
                "canvas width, height, and fps must be greater than zero",
            ));
        }
        validate_unique_ids("media", self.media.iter().map(|item| item.id.as_str()))?;
        for media in &self.media {
            validate_finite_optional(media.probe.duration, "media duration")?;
            validate_finite_optional(media.probe.fps, "media fps")?;
        }
        validate_unique_ids("clip", self.timeline.iter().map(|item| item.id.as_str()))?;
        for clip in &self.timeline {
            let media = self.media_by_id(&clip.media_id)?;
            if media.kind != MediaKind::Video {
                return Err(message(format!(
                    "clip {} does not refer to video media",
                    clip.id
                )));
            }
            validate_source_range(clip.source_in, clip.source_out, media, &clip.id)?;
            validate_speed(clip.speed)?;
            validate_non_negative(clip.volume, &format!("clip {} volume", clip.id))?;
        }
        validate_unique_ids(
            "text overlay",
            self.text_overlays.iter().map(|item| item.id.as_str()),
        )?;
        for text in &self.text_overlays {
            validate_interval(text.start, text.end, &text.id)?;
            validate_position(&text.position)?;
            if text.text.contains('\0') {
                return Err(message(format!(
                    "text overlay {} contains a NUL character",
                    text.id
                )));
            }
            if text.font.as_ref().is_some_and(|font| font.contains('\0')) {
                return Err(message(format!(
                    "text overlay {} font contains a NUL character",
                    text.id
                )));
            }
            if text.font_size == 0 {
                return Err(message(format!(
                    "text overlay {} font size must be positive",
                    text.id
                )));
            }
            validate_color(&text.color)?;
            if let Some(outline) = &text.outline {
                validate_color(&outline.color)?;
            }
            if let Some(background) = &text.background {
                validate_color(background)?;
            }
            validate_opacity(text.opacity, &text.id)?;
        }
        validate_unique_ids(
            "image overlay",
            self.image_overlays.iter().map(|item| item.id.as_str()),
        )?;
        for image in &self.image_overlays {
            validate_interval(image.start, image.end, &image.id)?;
            validate_position(&image.position)?;
            let media = self.media_by_id(&image.media_id)?;
            if media.kind != MediaKind::Image {
                return Err(message(format!(
                    "image overlay {} does not refer to image media",
                    image.id
                )));
            }
            if let Some(width) = &image.width {
                width.value()?;
            }
            if let Some(height) = &image.height {
                height.value()?;
            }
            validate_opacity(image.opacity, &image.id)?;
        }
        validate_unique_ids(
            "audio clip",
            self.audio_clips.iter().map(|item| item.id.as_str()),
        )?;
        for audio in &self.audio_clips {
            validate_time(audio.start, &format!("audio clip {} start", audio.id))?;
            let media = self.media_by_id(&audio.media_id)?;
            if media.kind != MediaKind::Audio && !media.probe.has_audio {
                return Err(message(format!(
                    "audio clip {} refers to media without audio",
                    audio.id
                )));
            }
            let source_out = audio.resolved_source_out(media)?;
            validate_source_range(audio.source_in, source_out, media, &audio.id)?;
            validate_non_negative(audio.volume, &format!("audio clip {} volume", audio.id))?;
            validate_non_negative(audio.fade_in, &format!("audio clip {} fade-in", audio.id))?;
            validate_non_negative(audio.fade_out, &format!("audio clip {} fade-out", audio.id))?;
            let duration = source_out - audio.source_in;
            if audio.fade_in > duration || audio.fade_out > duration {
                return Err(message(format!(
                    "audio clip {} fade exceeds its duration",
                    audio.id
                )));
            }
        }
        validate_unique_ids(
            "voice clip",
            self.voice_clips.iter().map(|item| item.id.as_str()),
        )?;
        let mut linked_audio = std::collections::HashSet::new();
        for voice in &self.voice_clips {
            if voice.voice.is_empty() {
                return Err(message(format!("voice clip {} has an empty voice id", voice.id)));
            }
            if voice.text.is_empty() || voice.text.contains('\0') {
                return Err(message(format!("voice clip {} has invalid text", voice.id)));
            }
            if !voice.speed.is_finite() || voice.speed <= 0.0 {
                return Err(message(format!("voice clip {} has invalid speed", voice.id)));
            }
            if !voice.pitch.is_finite() {
                return Err(message(format!("voice clip {} has invalid pitch", voice.id)));
            }
            if voice.endpoint.is_empty() || voice.engine_identity.is_empty() || voice.cache_key.is_empty() {
                return Err(message(format!("voice clip {} has incomplete TTS identity", voice.id)));
            }
            if !linked_audio.insert(voice.audio_clip_id.as_str()) {
                return Err(message(format!(
                    "audio clip {} is linked by more than one voice clip",
                    voice.audio_clip_id
                )));
            }
            let audio = self
                .audio_clips
                .iter()
                .find(|item| item.id == voice.audio_clip_id)
                .ok_or_else(|| {
                    message(format!(
                        "voice clip {} refers to missing audio clip {}",
                        voice.id, voice.audio_clip_id
                    ))
                })?;
            let media = self.media_by_id(&audio.media_id)?;
            if media.kind != MediaKind::Audio {
                return Err(message(format!(
                    "voice clip {} does not resolve to audio media",
                    voice.id
                )));
            }
        }
        Ok(())
    }

    pub fn media_by_id(&self, id: &str) -> Result<&Media> {
        self.media
            .iter()
            .find(|media| media.id == id)
            .ok_or_else(|| message(format!("media {id} does not exist")))
    }
    pub fn media_ref(&self, value: &str) -> Result<&Media> {
        self.media
            .iter()
            .find(|media| media.id == value || media.path.eq_ignore_ascii_case(value))
            .ok_or_else(|| message(format!("media {value:?} is not imported")))
    }
    pub fn next_media_id(&self) -> String {
        next_id("m", self.media.iter().map(|item| item.id.as_str()))
    }
    pub fn next_clip_id(&self) -> String {
        next_id("c", self.timeline.iter().map(|item| item.id.as_str()))
    }
    pub fn next_text_id(&self) -> String {
        next_id("t", self.text_overlays.iter().map(|item| item.id.as_str()))
    }
    pub fn next_image_id(&self) -> String {
        next_id("i", self.image_overlays.iter().map(|item| item.id.as_str()))
    }
    pub fn next_audio_id(&self) -> String {
        next_id("a", self.audio_clips.iter().map(|item| item.id.as_str()))
    }
    pub fn next_voice_id(&self) -> String {
        next_id("v", self.voice_clips.iter().map(|item| item.id.as_str()))
    }
}

impl AudioClip {
    pub fn resolved_source_out(&self, media: &Media) -> Result<f64> {
        self.source_out.or(media.probe.duration).ok_or_else(|| {
            message(format!(
                "audio clip {} has no source-out and media duration is unknown",
                self.id
            ))
        })
    }
}

pub fn validate_speed(speed: f64) -> Result<()> {
    if !speed.is_finite() || !(0.05..=20.0).contains(&speed) {
        return Err(message("speed must be finite and between 0.05 and 20.0"));
    }
    Ok(())
}

pub fn validate_color(color: &str) -> Result<()> {
    if color.is_empty()
        || !color.chars().all(|value| {
            value.is_ascii_alphanumeric() || matches!(value, '#' | '@' | '.' | '_' | '-')
        })
    {
        return Err(message(format!("invalid color {color:?}")));
    }
    Ok(())
}

fn validate_position(position: &Position) -> Result<()> {
    position.x.value(Axis::X)?;
    position.y.value(Axis::Y)?;
    Ok(())
}

fn normalize_coordinate(value: &str, axis: Axis) -> Result<String> {
    let normalized = value.trim().to_ascii_lowercase();
    let named = match (axis, normalized.as_str()) {
        (Axis::X, "left") | (Axis::Y, "top") => Some(normalized.as_str()),
        (_, "center") => Some("center"),
        (Axis::X, "right") | (Axis::Y, "bottom") => Some(normalized.as_str()),
        _ => None,
    };
    if let Some(named) = named {
        return Ok(named.to_owned());
    }
    let percent = normalized
        .strip_suffix('%')
        .ok_or_else(|| message(format!("invalid position {value:?}")))?
        .parse::<f64>()
        .map_err(|_| message(format!("invalid position {value:?}")))?;
    if !percent.is_finite() || !(0.0..=100.0).contains(&percent) {
        return Err(message(format!(
            "position percentage is outside 0%..100%: {value:?}"
        )));
    }
    Ok(format!("{}%", format_decimal(percent)))
}

fn coordinate_value(value: &str, axis: Axis) -> Result<CoordinateValue> {
    let normalized = normalize_coordinate(value, axis)?;
    match normalized.as_str() {
        "left" | "top" => Ok(CoordinateValue::Start),
        "center" => Ok(CoordinateValue::Center),
        "right" | "bottom" => Ok(CoordinateValue::End),
        _ => Ok(CoordinateValue::Percent(
            normalized.trim_end_matches('%').parse::<f64>().unwrap() / 100.0,
        )),
    }
}

fn normalize_dimension(value: &str) -> Result<String> {
    let normalized = value.trim().to_ascii_lowercase();
    let (number, suffix) = if let Some(value) = normalized.strip_suffix('%') {
        (value, "%")
    } else if let Some(value) = normalized.strip_suffix("px") {
        (value, "px")
    } else {
        (normalized.as_str(), "px")
    };
    let number = number
        .parse::<f64>()
        .map_err(|_| message(format!("invalid dimension {value:?}")))?;
    if !number.is_finite() || number <= 0.0 || (suffix == "%" && number > 100.0) {
        return Err(message(format!("invalid dimension {value:?}")));
    }
    Ok(format!("{}{suffix}", format_decimal(number)))
}

fn dimension_value(value: &str) -> Result<DimensionValue> {
    let normalized = normalize_dimension(value)?;
    if let Some(value) = normalized.strip_suffix('%') {
        Ok(DimensionValue::Percent(
            value.parse::<f64>().unwrap() / 100.0,
        ))
    } else {
        Ok(DimensionValue::Pixels(
            normalized.trim_end_matches("px").parse::<f64>().unwrap(),
        ))
    }
}

fn validate_source_range(start: f64, end: f64, media: &Media, owner: &str) -> Result<()> {
    validate_time(start, &format!("{owner} source-in"))?;
    validate_time(end, &format!("{owner} source-out"))?;
    if end <= start {
        return Err(message(format!("{owner} has an invalid source range")));
    }
    if let Some(duration) = media.probe.duration
        && end > duration + 0.000_001
    {
        return Err(message(format!(
            "{owner} ends at {end:.3}s, past media duration {duration:.3}s"
        )));
    }
    Ok(())
}

fn validate_interval(start: f64, end: f64, owner: &str) -> Result<()> {
    validate_time(start, &format!("{owner} start"))?;
    validate_time(end, &format!("{owner} end"))?;
    if end <= start {
        return Err(message(format!("{owner} end must be greater than start")));
    }
    Ok(())
}

fn validate_time(value: f64, name: &str) -> Result<()> {
    if !value.is_finite() || value < 0.0 {
        return Err(message(format!(
            "{name} must be a finite non-negative number"
        )));
    }
    Ok(())
}

fn validate_opacity(value: f64, owner: &str) -> Result<()> {
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(message(format!("{owner} opacity must be between 0 and 1")));
    }
    Ok(())
}

fn validate_non_negative(value: f64, name: &str) -> Result<()> {
    if !value.is_finite() || value < 0.0 {
        return Err(message(format!(
            "{name} must be a finite non-negative number"
        )));
    }
    Ok(())
}

fn validate_finite_optional(value: Option<f64>, name: &str) -> Result<()> {
    if value.is_some_and(|value| !value.is_finite() || value < 0.0) {
        return Err(message(format!(
            "{name} must be a finite non-negative number"
        )));
    }
    Ok(())
}

fn validate_unique_ids<'a>(kind: &str, ids: impl Iterator<Item = &'a str>) -> Result<()> {
    let mut seen = std::collections::HashSet::new();
    for id in ids {
        if !seen.insert(id) {
            return Err(message(format!("duplicate {kind} id {id}")));
        }
    }
    Ok(())
}

fn is_false(value: &bool) -> bool {
    !*value
}
fn default_speed() -> f64 {
    DEFAULT_SPEED
}
fn default_volume() -> f64 {
    DEFAULT_VOLUME
}
fn default_opacity() -> f64 {
    DEFAULT_OPACITY
}
fn is_one(value: &f64) -> bool {
    (*value - 1.0).abs() < f64::EPSILON
}
fn is_zero(value: &f64) -> bool {
    value.abs() < f64::EPSILON
}

fn next_id<'a>(prefix: &str, ids: impl Iterator<Item = &'a str>) -> String {
    let max = ids
        .filter_map(|id| id.strip_prefix(prefix)?.parse::<u64>().ok())
        .max()
        .unwrap_or(0);
    format!("{prefix}{}", max + 1)
}

fn format_decimal(value: f64) -> String {
    let value = format!("{value:.6}");
    value.trim_end_matches('0').trim_end_matches('.').to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn coordinates_and_dimensions_are_normalized() {
        assert_eq!(Coordinate::parse("50.000%", Axis::X).unwrap().0, "50%");
        assert_eq!(Coordinate::parse(" Bottom ", Axis::Y).unwrap().0, "bottom");
        assert!(Coordinate::parse("bottom", Axis::X).is_err());
        assert_eq!(Dimension::parse("320").unwrap().0, "320px");
        assert_eq!(Dimension::parse("12.50%").unwrap().0, "12.5%");
    }
    #[test]
    fn speed_bounds_are_enforced() {
        assert!(validate_speed(0.05).is_ok());
        assert!(validate_speed(20.0).is_ok());
        for invalid in [0.0, -1.0, 0.049, 20.1, f64::NAN, f64::INFINITY] {
            assert!(validate_speed(invalid).is_err());
        }
    }
}
