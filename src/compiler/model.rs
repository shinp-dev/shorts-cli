use std::path::PathBuf;

use serde::Serialize;

use crate::project::Canvas;

#[derive(Debug, Clone, Copy, Serialize)]
pub struct TimeRange {
    pub start: f64,
    pub end: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct RenderSegment {
    pub clip_id: String,
    pub media_id: String,
    pub path: PathBuf,
    pub source_in: f64,
    pub source_out: f64,
    pub has_audio: bool,
    pub volume: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct RenderPlan {
    pub canvas: Canvas,
    pub range: TimeRange,
    pub duration: f64,
    pub segments: Vec<RenderSegment>,
}
