mod model;

pub use model::*;

use std::path::{Path, PathBuf};

use crate::error::{Result, message};
use crate::project::Project;
use crate::timeline;

pub fn compile(
    project_path: &Path,
    project: &Project,
    range: Option<TimeRange>,
) -> Result<RenderPlan> {
    let total = timeline::duration(project);
    if total <= 0.0 {
        return Err(message("timeline is empty"));
    }
    let range = range.unwrap_or(TimeRange {
        start: 0.0,
        end: total,
    });
    if !range.start.is_finite()
        || !range.end.is_finite()
        || range.start < 0.0
        || range.end <= range.start
    {
        return Err(message(
            "render range must be finite and --to must be greater than --from",
        ));
    }
    if range.end > total + 0.000_001 {
        return Err(message(format!(
            "range ends at {:.3}s, past timeline duration {:.3}s",
            range.end, total
        )));
    }

    let base = project_path.parent().unwrap_or_else(|| Path::new("."));
    let mut segments = Vec::new();
    for item in timeline::resolve(project) {
        let overlap_start = item.timeline_start.max(range.start);
        let overlap_end = item.timeline_end.min(range.end);
        if overlap_end <= overlap_start + 0.000_001 {
            continue;
        }
        let media = project.media_by_id(&item.clip.media_id)?;
        let path = PathBuf::from(media.path.replace('/', std::path::MAIN_SEPARATOR_STR));
        let path = if path.is_absolute() {
            path
        } else {
            base.join(path)
        };
        if !path.is_file() {
            return Err(message(format!(
                "media {} is missing or is not a file: {}",
                media.id,
                path.display()
            )));
        }
        segments.push(RenderSegment {
            clip_id: item.clip.id,
            media_id: media.id.clone(),
            path,
            source_in: item.clip.source_in + (overlap_start - item.timeline_start),
            source_out: item.clip.source_in + (overlap_end - item.timeline_start),
            has_audio: media.probe.has_audio && !item.clip.mute,
            volume: item.clip.volume,
        });
    }
    if segments.is_empty() {
        return Err(message("render range contains no video"));
    }
    Ok(RenderPlan {
        canvas: project.canvas,
        range,
        duration: range.end - range.start,
        segments,
    })
}
