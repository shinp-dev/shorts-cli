use serde::{Deserialize, Serialize};

use crate::error::{Result, message};

pub const PROJECT_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Project {
    pub version: u32,
    pub canvas: Canvas,
    pub media: Vec<Media>,
    pub timeline: Vec<Clip>,
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
    #[serde(default, skip_serializing_if = "is_false")]
    pub mute: bool,
    #[serde(default = "default_volume", skip_serializing_if = "is_one")]
    pub volume: f64,
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn default_volume() -> f64 {
    1.0
}

fn is_one(value: &f64) -> bool {
    (*value - 1.0).abs() < f64::EPSILON
}

impl Project {
    pub fn new(canvas: Canvas) -> Self {
        Self {
            version: PROJECT_VERSION,
            canvas,
            media: Vec::new(),
            timeline: Vec::new(),
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

        let mut ids = std::collections::HashSet::new();
        for media in &self.media {
            if !ids.insert(media.id.as_str()) {
                return Err(message(format!("duplicate media id {}", media.id)));
            }
            validate_finite_optional(media.probe.duration, "media duration")?;
            validate_finite_optional(media.probe.fps, "media fps")?;
        }

        ids.clear();
        for clip in &self.timeline {
            if !ids.insert(clip.id.as_str()) {
                return Err(message(format!("duplicate clip id {}", clip.id)));
            }
            if self.media.iter().all(|media| media.id != clip.media_id) {
                return Err(message(format!(
                    "clip {} refers to missing media {}",
                    clip.id, clip.media_id
                )));
            }
            if !clip.source_in.is_finite()
                || !clip.source_out.is_finite()
                || clip.source_in < 0.0
                || clip.source_out <= clip.source_in
            {
                return Err(message(format!(
                    "clip {} has an invalid source range",
                    clip.id
                )));
            }
            if !clip.volume.is_finite() || clip.volume < 0.0 {
                return Err(message(format!("clip {} has an invalid volume", clip.id)));
            }
            let media = self.media_by_id(&clip.media_id)?;
            if let Some(duration) = media.probe.duration
                && clip.source_out > duration + 0.000_001
            {
                return Err(message(format!(
                    "clip {} ends at {:.3}s, past media duration {:.3}s",
                    clip.id, clip.source_out, duration
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
}

fn validate_finite_optional(value: Option<f64>, name: &str) -> Result<()> {
    if value.is_some_and(|value| !value.is_finite() || value < 0.0) {
        return Err(message(format!(
            "{name} must be a finite non-negative number"
        )));
    }
    Ok(())
}

fn next_id<'a>(prefix: &str, ids: impl Iterator<Item = &'a str>) -> String {
    let max = ids
        .filter_map(|id| id.strip_prefix(prefix)?.parse::<u64>().ok())
        .max()
        .unwrap_or(0);
    format!("{prefix}{}", max + 1)
}
