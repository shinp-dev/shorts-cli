use std::path::PathBuf;

use serde::Serialize;

use crate::project::{Canvas, CoordinateValue, DimensionValue, Outline};

#[derive(Debug, Clone, Copy, Serialize)]
pub struct TimeRange {
    pub start: f64,
    pub end: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct RenderVideoSegment {
    pub clip_id: String,
    pub media_id: String,
    pub path: PathBuf,
    pub source_in: f64,
    pub source_out: f64,
    pub speed: f64,
    pub hold_duration: Option<f64>,
    pub has_audio: bool,
    pub volume: f64,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct RenderPosition {
    pub x: CoordinateValue,
    pub y: CoordinateValue,
}

#[derive(Debug, Clone, Serialize)]
pub struct RenderTextOverlay {
    pub id: String,
    pub text: String,
    pub start: f64,
    pub end: f64,
    pub position: RenderPosition,
    pub font: Option<String>,
    pub font_size: u32,
    pub color: String,
    pub outline: Option<Outline>,
    pub background: Option<String>,
    pub opacity: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct RenderImageOverlay {
    pub id: String,
    pub media_id: String,
    pub path: PathBuf,
    pub start: f64,
    pub end: f64,
    pub position: RenderPosition,
    pub width: Option<DimensionValue>,
    pub height: Option<DimensionValue>,
    pub opacity: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct RenderAudioClip {
    pub id: String,
    pub media_id: String,
    pub path: PathBuf,
    pub source_in: f64,
    pub source_out: f64,
    pub speed: f64,
    pub r#loop: bool,
    pub track: Option<String>,
    pub timeline_start: f64,
    pub timeline_end: f64,
    pub clip_offset: f64,
    pub input_trim_offset: f64,
    pub clip_duration: f64,
    pub volume: f64,
    pub mute: bool,
    pub fade_in: f64,
    pub fade_out: f64,
    pub ducking: Vec<RenderDucking>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RenderDucking {
    pub reduction_db: f64,
    pub attack: f64,
    pub release: f64,
    pub intervals: Vec<TimeRange>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RenderPlan {
    pub canvas: Canvas,
    pub range: TimeRange,
    pub duration: f64,
    pub video_segments: Vec<RenderVideoSegment>,
    pub text_overlays: Vec<RenderTextOverlay>,
    pub image_overlays: Vec<RenderImageOverlay>,
    pub audio_clips: Vec<RenderAudioClip>,
}
