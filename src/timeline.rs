use crate::error::{Result, message};
use crate::project::{Clip, HoldClip, MediaKind, Project, TimelineItem, validate_speed};

const EPSILON: f64 = 0.000_001;

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedTimelineItem {
    pub item: TimelineItem,
    pub timeline_start: f64,
    pub timeline_end: f64,
}

impl ResolvedTimelineItem {
    pub fn hold(&self) -> Option<&HoldClip> {
        self.item.hold()
    }
}

pub fn resolve(project: &Project) -> Vec<ResolvedTimelineItem> {
    let mut cursor = 0.0;
    project
        .timeline
        .iter()
        .map(|item| {
            let duration = item.duration();
            let resolved = ResolvedTimelineItem {
                item: item.clone(),
                timeline_start: cursor,
                timeline_end: cursor + duration,
            };
            cursor += duration;
            resolved
        })
        .collect()
}

pub fn duration(project: &Project) -> f64 {
    project.timeline.iter().map(TimelineItem::duration).sum()
}

pub fn add(
    project: &mut Project,
    media_ref: &str,
    source_in: Option<f64>,
    source_out: Option<f64>,
) -> Result<String> {
    let clip = new_clip(project, media_ref, source_in, source_out)?;
    let id = clip.id.clone();
    project.timeline.push(clip.into());
    Ok(id)
}

pub fn insert(
    project: &mut Project,
    media_ref: &str,
    at: f64,
    source_in: Option<f64>,
    source_out: Option<f64>,
) -> Result<String> {
    validate_time(at, "--at")?;
    let total = duration(project);
    if at > total + EPSILON {
        return Err(message(format!(
            "--at {:.3}s is past timeline duration {:.3}s",
            at, total
        )));
    }
    let inserted = new_clip(project, media_ref, source_in, source_out)?;
    let inserted_id = inserted.id.clone();

    if project.timeline.is_empty() || (at - total).abs() <= EPSILON {
        project.timeline.push(inserted.into());
        return Ok(inserted_id);
    }

    let resolved = resolve(project);
    let index = resolved
        .iter()
        .position(|item| at < item.timeline_end - EPSILON)
        .unwrap_or(project.timeline.len());

    if index == project.timeline.len() {
        project.timeline.push(inserted.into());
        return Ok(inserted_id);
    }

    let item = &resolved[index];
    if (at - item.timeline_start).abs() <= EPSILON {
        project.timeline.insert(index, inserted.into());
        return Ok(inserted_id);
    }

    match &item.item {
        TimelineItem::Clip(clip) => {
            let split_source = clip.source_in + (at - item.timeline_start) * clip.speed;
            let mut right = clip.clone();
            right.id = next_clip_id_excluding(project, &[&inserted_id]);
            right.source_in = split_source;
            project.timeline[index].clip_mut().unwrap().source_out = split_source;
            project.timeline.insert(index + 1, inserted.into());
            project.timeline.insert(index + 2, right.into());
        }
        TimelineItem::Hold(hold) => {
            let left_duration = at - item.timeline_start;
            let right_duration = item.timeline_end - at;
            let mut right = hold.clone();
            right.id = project.next_hold_id();
            right.duration = right_duration;
            project.timeline[index].hold_mut().unwrap().duration = left_duration;
            project.timeline.insert(index + 1, inserted.into());
            project.timeline.insert(index + 2, right.into());
        }
    }
    Ok(inserted_id)
}

pub fn trim(
    project: &mut Project,
    clip_id: &str,
    source_in: Option<f64>,
    source_out: Option<f64>,
) -> Result<()> {
    let index = project
        .timeline
        .iter()
        .position(|item| item.id() == clip_id)
        .ok_or_else(|| message(format!("clip {clip_id} does not exist")))?;
    let old = project.timeline[index]
        .clip()
        .ok_or_else(|| message(format!("{clip_id} is a hold; trim requires a video clip")))?;
    let new_in = source_in.unwrap_or(old.source_in);
    let new_out = source_out.unwrap_or(old.source_out);
    validate_source_range(project, &old.media_id, new_in, new_out)?;
    let clip = project.timeline[index].clip_mut().unwrap();
    clip.source_in = new_in;
    clip.source_out = new_out;
    Ok(())
}

#[cfg(test)]
pub fn speed(project: &mut Project, clip_id: &str, rate: f64) -> Result<()> {
    speed_with_retime(project, clip_id, rate, false, &[])
}

pub fn speed_with_retime(
    project: &mut Project,
    clip_id: &str,
    rate: f64,
    retime_overlays: bool,
    retime_tracks: &[String],
) -> Result<()> {
    validate_speed(rate)?;
    let resolved = resolve(project);
    let item = resolved
        .iter()
        .find(|item| item.item.id() == clip_id)
        .ok_or_else(|| message(format!("clip {clip_id} does not exist")))?;
    let old_clip = item
        .item
        .clip()
        .ok_or_else(|| message(format!("{clip_id} is a hold; speed requires a video clip")))?;
    let start = item.timeline_start;
    let old_end = item.timeline_end;
    let new_end = start + (old_clip.source_out - old_clip.source_in) / rate;
    let ratio = (new_end - start) / (old_end - start);
    let clip = project
        .timeline
        .iter_mut()
        .find(|item| item.id() == clip_id)
        .ok_or_else(|| message(format!("clip {clip_id} does not exist")))?;
    let clip = clip
        .clip_mut()
        .ok_or_else(|| message(format!("{clip_id} is a hold; speed requires a video clip")))?;
    clip.speed = rate;
    retime_items(project, retime_overlays, retime_tracks, |time| {
        if time < start {
            time
        } else if time <= old_end {
            start + (time - start) * ratio
        } else {
            time + (new_end - old_end)
        }
    })?;
    Ok(())
}

pub fn hold_at_source(
    project: &mut Project,
    clip_id: &str,
    source: f64,
    hold_duration: f64,
    retime_overlays: bool,
    retime_tracks: &[String],
) -> Result<String> {
    validate_hold_duration(hold_duration)?;
    let resolved = resolve(project);
    let (index, item) = resolved
        .iter()
        .enumerate()
        .find(|(_, item)| item.item.id() == clip_id)
        .ok_or_else(|| message(format!("clip {clip_id} does not exist")))?;
    let clip = item
        .item
        .clip()
        .ok_or_else(|| message(format!("{clip_id} is a hold; hold requires a video clip")))?
        .clone();
    if !source.is_finite()
        || source < clip.source_in - EPSILON
        || source > clip.source_out + EPSILON
    {
        return Err(message(format!(
            "--source must be within {:.3}..={:.3}",
            clip.source_in, clip.source_out
        )));
    }
    let hold_id = project.next_hold_id();
    let (insert_index, point, freeze_at) = if (source - clip.source_in).abs() <= EPSILON {
        (index, item.timeline_start, clip.source_in)
    } else if (source - clip.source_out).abs() <= EPSILON {
        (
            index + 1,
            item.timeline_end,
            last_complete_frame(project, &clip)?,
        )
    } else {
        let point = item.timeline_start + (source - clip.source_in) / clip.speed;
        let mut right = clip.clone();
        right.id = next_clip_id_excluding(project, &[]);
        right.source_in = source;
        project.timeline[index].clip_mut().unwrap().source_out = source;
        project.timeline.insert(index + 1, right.into());
        (index + 1, point, source)
    };
    project.timeline.insert(
        insert_index,
        HoldClip {
            id: hold_id.clone(),
            media_id: clip.media_id,
            freeze_at,
            duration: hold_duration,
        }
        .into(),
    );
    retime_after_point(
        project,
        point,
        hold_duration,
        retime_overlays,
        retime_tracks,
    )?;
    Ok(hold_id)
}

pub fn hold_before(
    project: &mut Project,
    clip_id: &str,
    duration: f64,
    retime_overlays: bool,
    retime_tracks: &[String],
) -> Result<String> {
    let clip = project
        .timeline
        .iter()
        .find(|item| item.id() == clip_id)
        .and_then(TimelineItem::clip)
        .ok_or_else(|| message(format!("video clip {clip_id} does not exist")))?;
    hold_at_source(
        project,
        clip_id,
        clip.source_in,
        duration,
        retime_overlays,
        retime_tracks,
    )
}

pub fn hold_after(
    project: &mut Project,
    clip_id: &str,
    duration: f64,
    retime_overlays: bool,
    retime_tracks: &[String],
) -> Result<String> {
    let clip = project
        .timeline
        .iter()
        .find(|item| item.id() == clip_id)
        .and_then(TimelineItem::clip)
        .ok_or_else(|| message(format!("video clip {clip_id} does not exist")))?;
    hold_at_source(
        project,
        clip_id,
        clip.source_out,
        duration,
        retime_overlays,
        retime_tracks,
    )
}

pub fn hold_set(
    project: &mut Project,
    hold_id: &str,
    duration: f64,
    retime_overlays: bool,
    retime_tracks: &[String],
) -> Result<()> {
    validate_hold_duration(duration)?;
    let resolved = resolve(project);
    let item = resolved
        .iter()
        .find(|item| item.item.id() == hold_id)
        .ok_or_else(|| message(format!("hold {hold_id} does not exist")))?;
    let old = item
        .hold()
        .ok_or_else(|| message(format!("{hold_id} is not a hold")))?
        .duration;
    let point = item.timeline_start;
    project
        .timeline
        .iter_mut()
        .find(|item| item.id() == hold_id)
        .and_then(TimelineItem::hold_mut)
        .unwrap()
        .duration = duration;
    retime_after_point(
        project,
        point,
        duration - old,
        retime_overlays,
        retime_tracks,
    )
}

pub fn hold_remove(
    project: &mut Project,
    hold_id: &str,
    retime_overlays: bool,
    retime_tracks: &[String],
) -> Result<()> {
    let resolved = resolve(project);
    let item = resolved
        .iter()
        .find(|item| item.item.id() == hold_id)
        .ok_or_else(|| message(format!("hold {hold_id} does not exist")))?;
    let hold = item
        .hold()
        .ok_or_else(|| message(format!("{hold_id} is not a hold")))?;
    let point = item.timeline_start;
    let delta = -hold.duration;
    let index = project
        .timeline
        .iter()
        .position(|item| item.id() == hold_id)
        .unwrap();
    project.timeline.remove(index);
    retime_after_point(project, point, delta, retime_overlays, retime_tracks)
}

pub fn remove(project: &mut Project, from: f64, to: f64) -> Result<()> {
    validate_time(from, "--from")?;
    validate_time(to, "--to")?;
    if to <= from + EPSILON {
        return Err(message("--to must be greater than --from"));
    }
    let total = duration(project);
    if to > total + EPSILON {
        return Err(message(format!(
            "--to {:.3}s is past timeline duration {:.3}s",
            to, total
        )));
    }

    let resolved = resolve(project);
    let mut output = Vec::new();
    let mut reserved_ids: Vec<String> = project
        .timeline
        .iter()
        .map(|item| item.id().to_owned())
        .collect();
    for item in resolved {
        if item.timeline_end <= from + EPSILON || item.timeline_start >= to - EPSILON {
            output.push(item.item);
            continue;
        }

        let left_duration =
            (from - item.timeline_start).clamp(0.0, item.timeline_end - item.timeline_start);
        let right_duration =
            (item.timeline_end - to).clamp(0.0, item.timeline_end - item.timeline_start);
        match item.item {
            TimelineItem::Clip(clip) => {
                if left_duration > EPSILON {
                    let mut left = clip.clone();
                    left.source_out = left.source_in + left_duration * left.speed;
                    output.push(left.into());
                }
                if right_duration > EPSILON {
                    let mut right = clip;
                    right.source_in = right.source_out - right_duration * right.speed;
                    if left_duration > EPSILON {
                        right.id = next_id_from_reserved("c", &reserved_ids);
                        reserved_ids.push(right.id.clone());
                    }
                    output.push(right.into());
                }
            }
            TimelineItem::Hold(mut hold) => {
                let remaining = left_duration + right_duration;
                if remaining > EPSILON {
                    hold.duration = remaining;
                    output.push(hold.into());
                }
            }
        }
    }
    project.timeline = output;
    Ok(())
}

fn new_clip(
    project: &Project,
    media_ref: &str,
    source_in: Option<f64>,
    source_out: Option<f64>,
) -> Result<Clip> {
    let media = project.media_ref(media_ref)?;
    if media.kind != MediaKind::Video {
        return Err(message(format!(
            "media {} is {:?}; timeline clips require video",
            media.id, media.kind
        )));
    }
    let source_in = source_in.unwrap_or(0.0);
    let source_out = source_out.or(media.probe.duration).ok_or_else(|| {
        message(format!(
            "media {} has no known duration; specify --out",
            media.id
        ))
    })?;
    validate_source_range(project, &media.id, source_in, source_out)?;
    Ok(Clip {
        id: project.next_clip_id(),
        media_id: media.id.clone(),
        source_in,
        source_out,
        speed: 1.0,
        mute: false,
        volume: 1.0,
    })
}

fn validate_source_range(project: &Project, media_id: &str, start: f64, end: f64) -> Result<()> {
    validate_time(start, "--in")?;
    validate_time(end, "--out")?;
    if end <= start + EPSILON {
        return Err(message("--out must be greater than --in"));
    }
    let media = project.media_by_id(media_id)?;
    if let Some(duration) = media.probe.duration
        && end > duration + EPSILON
    {
        return Err(message(format!(
            "--out {:.3}s is past media duration {:.3}s",
            end, duration
        )));
    }
    Ok(())
}

fn validate_time(value: f64, option: &str) -> Result<()> {
    if !value.is_finite() || value < 0.0 {
        return Err(message(format!(
            "{option} must be a finite non-negative number"
        )));
    }
    Ok(())
}

fn validate_hold_duration(value: f64) -> Result<()> {
    if !value.is_finite() || value <= EPSILON {
        return Err(message("--duration must be a finite positive number"));
    }
    Ok(())
}

fn last_complete_frame(project: &Project, clip: &Clip) -> Result<f64> {
    let media = project.media_by_id(&clip.media_id)?;
    let fps = media
        .probe
        .fps
        .unwrap_or(f64::from(project.canvas.fps))
        .max(1.0);
    Ok((clip.source_out - 1.0 / fps).max(clip.source_in))
}

fn retime_after_point(
    project: &mut Project,
    point: f64,
    delta: f64,
    retime_overlays: bool,
    retime_tracks: &[String],
) -> Result<()> {
    retime_items(project, retime_overlays, retime_tracks, |time| {
        if time <= point { time } else { time + delta }
    })
}

fn retime_items<F>(
    project: &mut Project,
    retime_overlays: bool,
    retime_tracks: &[String],
    map: F,
) -> Result<()>
where
    F: Fn(f64) -> f64,
{
    if retime_overlays {
        for text in &mut project.text_overlays {
            text.start = map(text.start);
            text.end = map(text.end);
            if text.start < 0.0 || text.end <= text.start + EPSILON {
                return Err(message(format!(
                    "retiming collapses text overlay {}",
                    text.id
                )));
            }
        }
        for image in &mut project.image_overlays {
            image.start = map(image.start);
            image.end = map(image.end);
            if image.start < 0.0 || image.end <= image.start + EPSILON {
                return Err(message(format!(
                    "retiming collapses image overlay {}",
                    image.id
                )));
            }
        }
    }
    for audio in &mut project.audio_clips {
        if !audio
            .track
            .as_ref()
            .is_some_and(|track| retime_tracks.contains(track))
        {
            continue;
        }
        audio.start = map(audio.start);
        if let Some(end) = audio.end.as_mut() {
            *end = map(*end);
            if *end <= audio.start + EPSILON {
                return Err(message(format!(
                    "retiming collapses audio clip {}",
                    audio.id
                )));
            }
        }
        if audio.start < 0.0 {
            return Err(message(format!(
                "retiming moves audio clip {} below zero",
                audio.id
            )));
        }
    }
    Ok(())
}

fn next_clip_id_excluding(project: &Project, additional: &[&str]) -> String {
    let mut ids: Vec<String> = project
        .timeline
        .iter()
        .map(|item| item.id().to_owned())
        .collect();
    ids.extend(additional.iter().map(|id| (*id).to_owned()));
    next_id_from_reserved("c", &ids)
}

fn next_id_from_reserved(prefix: &str, ids: &[String]) -> String {
    let max = ids
        .iter()
        .filter_map(|id| id.strip_prefix(prefix)?.parse::<u64>().ok())
        .max()
        .unwrap_or(0);
    format!("{prefix}{}", max + 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{Canvas, Media, MediaProbe};

    fn project() -> Project {
        let mut project = Project::new(Canvas {
            width: 1080,
            height: 1920,
            fps: 30,
        });
        project.media.push(Media {
            id: "m1".into(),
            path: "one.mp4".into(),
            kind: MediaKind::Video,
            probe: MediaProbe {
                duration: Some(10.0),
                width: Some(1920),
                height: Some(1080),
                fps: Some(30.0),
                has_audio: true,
                video_codec: Some("h264".into()),
                audio_codec: Some("aac".into()),
            },
        });
        project
    }

    fn clip(project: &Project, index: usize) -> &Clip {
        project.timeline[index].clip().unwrap()
    }

    #[test]
    fn insert_inside_clip_splits_and_ripples() {
        let mut p = project();
        add(&mut p, "m1", Some(0.0), Some(10.0)).unwrap();
        insert(&mut p, "m1", 4.0, Some(1.0), Some(3.0)).unwrap();
        assert_eq!(
            p.timeline.iter().map(TimelineItem::id).collect::<Vec<_>>(),
            ["c1", "c2", "c3"]
        );
        assert_eq!(resolve(&p).last().unwrap().timeline_end, 12.0);
        assert_eq!((clip(&p, 2).source_in, clip(&p, 2).source_out), (4.0, 10.0));
    }

    #[test]
    fn add_defaults_to_full_media_range() {
        let mut p = project();
        let id = add(&mut p, "one.mp4", None, None).unwrap();
        assert_eq!(id, "c1");
        assert_eq!(duration(&p), 10.0);
    }

    #[test]
    fn trim_uses_source_time_and_ripples_duration() {
        let mut p = project();
        add(&mut p, "m1", Some(1.0), Some(8.0)).unwrap();
        add(&mut p, "m1", Some(0.0), Some(2.0)).unwrap();
        trim(&mut p, "c1", Some(3.0), Some(7.0)).unwrap();
        let resolved = resolve(&p);
        assert_eq!(resolved[0].timeline_end, 4.0);
        assert_eq!(resolved[1].timeline_start, 4.0);
        assert_eq!(duration(&p), 6.0);
    }

    #[test]
    fn invalid_edits_do_not_mutate_project() {
        let mut p = project();
        add(&mut p, "m1", Some(0.0), Some(2.0)).unwrap();
        let before = p.clone();
        assert!(insert(&mut p, "m1", 3.0, None, None).is_err());
        assert_eq!(p, before);
        assert!(remove(&mut p, 0.0, 3.0).is_err());
        assert_eq!(p, before);
    }

    #[test]
    fn speed_changes_final_duration_and_split_source_mapping() {
        let mut p = project();
        add(&mut p, "m1", Some(0.0), Some(9.0)).unwrap();
        speed(&mut p, "c1", 1.5).unwrap();
        assert_eq!(duration(&p), 6.0);
        insert(&mut p, "m1", 2.0, Some(0.0), Some(1.0)).unwrap();
        assert_eq!(clip(&p, 0).source_out, 3.0);
        assert_eq!(clip(&p, 2).source_in, 3.0);
        assert_eq!(duration(&p), 7.0);
    }

    #[test]
    fn remove_maps_final_time_through_speed() {
        let mut p = project();
        add(&mut p, "m1", Some(0.0), Some(9.0)).unwrap();
        speed(&mut p, "c1", 1.5).unwrap();
        remove(&mut p, 1.0, 3.0).unwrap();
        assert_eq!(clip(&p, 0).source_out, 1.5);
        assert_eq!(clip(&p, 1).source_in, 4.5);
        assert_eq!(duration(&p), 4.0);
    }

    #[test]
    fn remove_middle_splits_and_ripples() {
        let mut p = project();
        add(&mut p, "m1", Some(0.0), Some(10.0)).unwrap();
        remove(&mut p, 3.0, 7.0).unwrap();
        assert_eq!(duration(&p), 6.0);
        assert_eq!((clip(&p, 0).source_in, clip(&p, 0).source_out), (0.0, 3.0));
        assert_eq!((clip(&p, 1).source_in, clip(&p, 1).source_out), (7.0, 10.0));
        assert_ne!(p.timeline[0].id(), p.timeline[1].id());
    }

    #[test]
    fn interior_hold_splits_video_and_retimes_selected_items() {
        let mut p = project();
        add(&mut p, "m1", Some(0.0), Some(10.0)).unwrap();
        p.text_overlays.push(crate::project::TextOverlay {
            id: "t1".into(),
            text: "hold".into(),
            start: 4.0,
            end: 6.0,
            position: crate::project::Position::center(),
            font: None,
            font_size: 40,
            color: "white".into(),
            outline: None,
            background: None,
            opacity: 1.0,
        });
        p.media.push(Media {
            id: "m2".into(),
            path: "voice.wav".into(),
            kind: MediaKind::Audio,
            probe: MediaProbe {
                duration: Some(1.0),
                width: None,
                height: None,
                fps: None,
                has_audio: true,
                video_codec: None,
                audio_codec: Some("pcm_s16le".into()),
            },
        });
        p.audio_clips.push(crate::project::AudioClip {
            id: "a1".into(),
            key: Some("cue".into()),
            track: Some("sfx".into()),
            media_id: "m2".into(),
            start: 4.0,
            end: Some(5.0),
            source_in: 0.0,
            source_out: Some(1.0),
            speed: 1.0,
            r#loop: false,
            volume: 1.0,
            mute: false,
            fade_in: 0.0,
            fade_out: 0.0,
        });

        let hold = hold_at_source(&mut p, "c1", 4.0, 2.0, true, &["sfx".into()]).unwrap();
        assert_eq!(hold, "h1");
        assert_eq!(
            p.timeline.iter().map(TimelineItem::id).collect::<Vec<_>>(),
            ["c1", "h1", "c2"]
        );
        assert_eq!((clip(&p, 0).source_in, clip(&p, 0).source_out), (0.0, 4.0));
        assert_eq!((clip(&p, 2).source_in, clip(&p, 2).source_out), (4.0, 10.0));
        assert_eq!(p.text_overlays[0].start, 4.0, "insertion cue remains fixed");
        assert_eq!(p.text_overlays[0].end, 8.0);
        assert_eq!(p.audio_clips[0].start, 4.0);
        assert_eq!(p.audio_clips[0].end, Some(7.0));
        assert_eq!(duration(&p), 12.0);

        hold_set(&mut p, "h1", 1.0, true, &["sfx".into()]).unwrap();
        assert_eq!(duration(&p), 11.0);
        assert_eq!(p.text_overlays[0].end, 7.0);
        assert_eq!(p.audio_clips[0].end, Some(6.0));
        hold_remove(&mut p, "h1", true, &["sfx".into()]).unwrap();
        assert_eq!(duration(&p), 10.0);
        assert_eq!(p.text_overlays[0].end, 6.0);
        assert_eq!(p.audio_clips[0].end, Some(5.0));
    }
}
