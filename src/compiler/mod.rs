mod model;

pub use model::*;

use std::path::{Path, PathBuf};

use crate::error::{Result, message};
use crate::project::{Axis, Project, TimelineItem};
use crate::timeline;

pub fn compile(
    project_path: &Path,
    project: &Project,
    range: Option<TimeRange>,
) -> Result<RenderPlan> {
    let mut project = project.clone();
    crate::tts::ensure_project_audio(project_path, &mut project)?;
    compile_materialized(project_path, &project, range)
}

fn compile_materialized(
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
        match &item.item {
            TimelineItem::Clip(clip) => {
                let media = project.media_by_id(&clip.media_id)?;
                let path = media_path(project_path, &media.path)?;
                video_segments.push(RenderVideoSegment {
                    clip_id: clip.id.clone(),
                    media_id: media.id.clone(),
                    path,
                    source_in: clip.source_in + (overlap_start - item.timeline_start) * clip.speed,
                    source_out: clip.source_in + (overlap_end - item.timeline_start) * clip.speed,
                    speed: clip.speed,
                    hold_duration: None,
                    has_audio: media.probe.has_audio && !clip.mute,
                    volume: clip.volume,
                });
            }
            TimelineItem::Hold(hold) => {
                let media = project.media_by_id(&hold.media_id)?;
                let path = media_path(project_path, &media.path)?;
                let frame_duration = 1.0
                    / media
                        .probe
                        .fps
                        .unwrap_or(f64::from(project.canvas.fps))
                        .max(1.0);
                let media_end = media
                    .probe
                    .duration
                    .unwrap_or(hold.freeze_at + frame_duration);
                video_segments.push(RenderVideoSegment {
                    clip_id: hold.id.clone(),
                    media_id: media.id.clone(),
                    path,
                    source_in: hold.freeze_at,
                    source_out: (hold.freeze_at + frame_duration).min(media_end),
                    speed: 1.0,
                    hold_duration: Some(overlap_end - overlap_start),
                    has_audio: false,
                    volume: 0.0,
                });
            }
        }
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
        let clip_duration = audio.resolved_duration(media)?;
        let resolved_end = audio.resolved_end(media)?;
        if let Some((start, end)) = intersect(audio.start, resolved_end, range) {
            let absolute_start = start + range.start;
            let clip_offset = absolute_start - audio.start;
            let render_duration = end - start;
            let (render_source_in, render_source_out, input_trim_offset) = if audio.r#loop {
                (audio.source_in, source_out, clip_offset)
            } else {
                let render_source_in = audio.source_in + clip_offset * audio.speed;
                (
                    render_source_in,
                    (render_source_in + render_duration * audio.speed).min(source_out),
                    0.0,
                )
            };
            audio_clips.push(RenderAudioClip {
                id: audio.id.clone(),
                media_id: media.id.clone(),
                path: media_path(project_path, &media.path)?,
                source_in: render_source_in,
                source_out: render_source_out,
                speed: audio.speed,
                r#loop: audio.r#loop,
                track: audio.track.clone(),
                timeline_start: start,
                timeline_end: end,
                clip_offset,
                input_trim_offset,
                clip_duration,
                volume: audio.volume,
                mute: audio.mute,
                fade_in: audio.fade_in,
                fade_out: audio.fade_out,
                ducking: Vec::new(),
            });
        }
    }

    for duck in &project.audio_ducking {
        let mut intervals = Vec::new();
        for trigger in &project.audio_clips {
            if trigger.mute
                || trigger.volume <= 0.0
                || !trigger
                    .track
                    .as_ref()
                    .is_some_and(|track| duck.trigger_tracks.contains(track))
            {
                continue;
            }
            let media = project.media_by_id(&trigger.media_id)?;
            let start = trigger.start;
            let end = trigger.resolved_end(media)?;
            if end + duck.release <= range.start || start - duck.attack >= range.end {
                continue;
            }
            intervals.push(TimeRange {
                start: start - range.start,
                end: end - range.start,
            });
        }
        if intervals.is_empty() {
            continue;
        }
        for audio in &mut audio_clips {
            if audio.track.as_deref() == Some(duck.target_track.as_str()) {
                audio.ducking.push(RenderDucking {
                    reduction_db: duck.reduction_db,
                    attack: duck.attack,
                    release: duck.release,
                    intervals: intervals.clone(),
                });
            }
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
        project.timeline.push(
            Clip {
                id: "c1".into(),
                media_id: "m1".into(),
                source_in: 0.0,
                source_out: 9.0,
                speed: 1.5,
                volume: 1.0,
                mute: false,
            }
            .into(),
        );
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
