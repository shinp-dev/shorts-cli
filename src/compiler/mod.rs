mod model;

pub use model::*;

use std::path::{Path, PathBuf};

use crate::error::{Result, message};
use crate::project::{Axis, Project};
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

    let mut video_segments = Vec::new();
    for item in timeline::resolve(project) {
        let overlap_start = item.timeline_start.max(range.start);
        let overlap_end = item.timeline_end.min(range.end);
        if overlap_end <= overlap_start + 0.000_001 {
            continue;
        }
        let media = project.media_by_id(&item.clip.media_id)?;
        let path = media_path(project_path, &media.path)?;
        video_segments.push(RenderVideoSegment {
            clip_id: item.clip.id,
            media_id: media.id.clone(),
            path,
            source_in: item.clip.source_in
                + (overlap_start - item.timeline_start) * item.clip.speed,
            source_out: item.clip.source_in + (overlap_end - item.timeline_start) * item.clip.speed,
            speed: item.clip.speed,
            has_audio: media.probe.has_audio && !item.clip.mute,
            volume: item.clip.volume,
        });
    }
    if video_segments.is_empty() {
        return Err(message("render range contains no video"));
    }

    let mut text_overlays = Vec::new();
    for text in &project.text_overlays {
        if let Some((start, end)) = intersect(text.start, text.end, range) {
            text_overlays.push(RenderTextOverlay {
                id: text.id.clone(),
                text: text.text.clone(),
                start,
                end,
                position: RenderPosition {
                    x: text.position.x.value(Axis::X)?,
                    y: text.position.y.value(Axis::Y)?,
                },
                font: text
                    .font
                    .as_ref()
                    .map(|font| resolve_font(project_path, font)),
                font_size: text.font_size,
                color: text.color.clone(),
                outline: text.outline.clone(),
                background: text.background.clone(),
                opacity: text.opacity,
            });
        }
    }

    let mut image_overlays = Vec::new();
    for image in &project.image_overlays {
        if let Some((start, end)) = intersect(image.start, image.end, range) {
            let media = project.media_by_id(&image.media_id)?;
            image_overlays.push(RenderImageOverlay {
                id: image.id.clone(),
                media_id: media.id.clone(),
                path: media_path(project_path, &media.path)?,
                start,
                end,
                position: RenderPosition {
                    x: image.position.x.value(Axis::X)?,
                    y: image.position.y.value(Axis::Y)?,
                },
                width: image
                    .width
                    .as_ref()
                    .map(|value| value.value())
                    .transpose()?,
                height: image
                    .height
                    .as_ref()
                    .map(|value| value.value())
                    .transpose()?,
                opacity: image.opacity,
            });
        }
    }

    let mut audio_clips = Vec::new();
    for audio in &project.audio_clips {
        let media = project.media_by_id(&audio.media_id)?;
        let source_out = audio.resolved_source_out(media)?;
        let clip_duration = source_out - audio.source_in;
        if let Some((start, end)) = intersect(audio.start, audio.start + clip_duration, range) {
            let absolute_start = start + range.start;
            let clip_offset = absolute_start - audio.start;
            audio_clips.push(RenderAudioClip {
                id: audio.id.clone(),
                media_id: media.id.clone(),
                path: media_path(project_path, &media.path)?,
                source_in: audio.source_in + clip_offset,
                source_out: audio.source_in + clip_offset + (end - start),
                timeline_start: start,
                timeline_end: end,
                clip_offset,
                clip_duration,
                volume: audio.volume,
                mute: audio.mute,
                fade_in: audio.fade_in,
                fade_out: audio.fade_out,
            });
        }
    }

    Ok(RenderPlan {
        canvas: project.canvas,
        range,
        duration: range.end - range.start,
        video_segments,
        text_overlays,
        image_overlays,
        audio_clips,
    })
}

fn intersect(start: f64, end: f64, range: TimeRange) -> Option<(f64, f64)> {
    let start = start.max(range.start);
    let end = end.min(range.end);
    (end > start + 0.000_001).then_some((start - range.start, end - range.start))
}

fn media_path(project_path: &Path, stored: &str) -> Result<PathBuf> {
    let base = project_path.parent().unwrap_or_else(|| Path::new("."));
    let path = PathBuf::from(stored.replace('/', std::path::MAIN_SEPARATOR_STR));
    let path = if path.is_absolute() {
        path
    } else {
        base.join(path)
    };
    if !path.is_file() {
        return Err(message(format!(
            "media is missing or is not a file: {}",
            path.display()
        )));
    }
    Ok(path)
}

fn resolve_font(project_path: &Path, stored: &str) -> String {
    let path = PathBuf::from(stored.replace('/', std::path::MAIN_SEPARATOR_STR));
    if path.is_absolute() {
        return path.to_string_lossy().into_owned();
    }
    let candidate = project_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(path);
    if candidate.is_file() {
        candidate.to_string_lossy().into_owned()
    } else {
        stored.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::*;

    #[test]
    fn speed_and_range_map_all_final_timeline_items() {
        let directory = tempfile::tempdir().unwrap();
        let media_path = directory.path().join("fixture.bin");
        std::fs::write(&media_path, b"fixture").unwrap();
        let project_path = directory.path().join("project.json");
        let mut project = Project::new(Canvas {
            width: 100,
            height: 200,
            fps: 30,
        });
        project.media.push(Media {
            id: "m1".into(),
            path: media_path.to_string_lossy().into_owned(),
            kind: MediaKind::Video,
            probe: MediaProbe {
                duration: Some(9.0),
                width: Some(100),
                height: Some(200),
                fps: Some(30.0),
                has_audio: true,
                video_codec: Some("h264".into()),
                audio_codec: Some("aac".into()),
            },
        });
        project.timeline.push(Clip {
            id: "c1".into(),
            media_id: "m1".into(),
            source_in: 0.0,
            source_out: 9.0,
            speed: 1.5,
            volume: 1.0,
            mute: false,
        });
        project.text_overlays.push(TextOverlay {
            id: "t1".into(),
            text: "日本語".into(),
            start: 2.0,
            end: 4.0,
            position: Position::center(),
            font: None,
            font_size: 40,
            color: "white".into(),
            outline: None,
            background: None,
            opacity: 1.0,
        });
        let plan = compile(
            &project_path,
            &project,
            Some(TimeRange {
                start: 1.0,
                end: 3.0,
            }),
        )
        .unwrap();
        assert_eq!(plan.duration, 2.0);
        assert_eq!(plan.video_segments[0].source_in, 1.5);
        assert_eq!(plan.video_segments[0].source_out, 4.5);
        assert_eq!(plan.text_overlays[0].start, 1.0);
        assert_eq!(plan.text_overlays[0].end, 2.0);
    }
}
