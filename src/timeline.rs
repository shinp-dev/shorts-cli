use crate::error::{Result, message};
use crate::project::{Clip, MediaKind, Project, validate_speed};

const EPSILON: f64 = 0.000_001;

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedClip {
    pub clip: Clip,
    pub timeline_start: f64,
    pub timeline_end: f64,
}

pub fn resolve(project: &Project) -> Vec<ResolvedClip> {
    let mut cursor = 0.0;
    project
        .timeline
        .iter()
        .map(|clip| {
            let duration = (clip.source_out - clip.source_in) / clip.speed;
            let resolved = ResolvedClip {
                clip: clip.clone(),
                timeline_start: cursor,
                timeline_end: cursor + duration,
            };
            cursor += duration;
            resolved
        })
        .collect()
}

pub fn duration(project: &Project) -> f64 {
    project
        .timeline
        .iter()
        .map(|clip| (clip.source_out - clip.source_in) / clip.speed)
        .sum()
}

pub fn add(
    project: &mut Project,
    media_ref: &str,
    source_in: Option<f64>,
    source_out: Option<f64>,
) -> Result<String> {
    let clip = new_clip(project, media_ref, source_in, source_out)?;
    let id = clip.id.clone();
    project.timeline.push(clip);
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
        project.timeline.push(inserted);
        return Ok(inserted_id);
    }

    let resolved = resolve(project);
    let index = resolved
        .iter()
        .position(|item| at < item.timeline_end - EPSILON)
        .unwrap_or(project.timeline.len());

    if index == project.timeline.len() {
        project.timeline.push(inserted);
        return Ok(inserted_id);
    }

    let item = &resolved[index];
    if (at - item.timeline_start).abs() <= EPSILON {
        project.timeline.insert(index, inserted);
        return Ok(inserted_id);
    }

    let split_source = item.clip.source_in + (at - item.timeline_start) * item.clip.speed;
    let mut right = item.clip.clone();
    right.id = next_clip_id_excluding(project, &[&inserted_id]);
    right.source_in = split_source;
    project.timeline[index].source_out = split_source;
    project.timeline.insert(index + 1, inserted);
    project.timeline.insert(index + 2, right);
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
        .position(|clip| clip.id == clip_id)
        .ok_or_else(|| message(format!("clip {clip_id} does not exist")))?;
    let old = &project.timeline[index];
    let new_in = source_in.unwrap_or(old.source_in);
    let new_out = source_out.unwrap_or(old.source_out);
    validate_source_range(project, &old.media_id, new_in, new_out)?;
    project.timeline[index].source_in = new_in;
    project.timeline[index].source_out = new_out;
    Ok(())
}

pub fn speed(project: &mut Project, clip_id: &str, rate: f64) -> Result<()> {
    validate_speed(rate)?;
    let clip = project
        .timeline
        .iter_mut()
        .find(|clip| clip.id == clip_id)
        .ok_or_else(|| message(format!("clip {clip_id} does not exist")))?;
    clip.speed = rate;
    Ok(())
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
    let mut reserved_ids: Vec<String> = project.timeline.iter().map(|c| c.id.clone()).collect();
    for item in resolved {
        if item.timeline_end <= from + EPSILON || item.timeline_start >= to - EPSILON {
            output.push(item.clip);
            continue;
        }

        let left_duration =
            (from - item.timeline_start).clamp(0.0, item.timeline_end - item.timeline_start);
        let right_duration =
            (item.timeline_end - to).clamp(0.0, item.timeline_end - item.timeline_start);
        if left_duration > EPSILON {
            let mut left = item.clip.clone();
            left.source_out = left.source_in + left_duration * left.speed;
            output.push(left);
        }
        if right_duration > EPSILON {
            let mut right = item.clip.clone();
            right.source_in = right.source_out - right_duration * right.speed;
            if left_duration > EPSILON {
                right.id = next_id_from_reserved("c", &reserved_ids);
                reserved_ids.push(right.id.clone());
            }
            output.push(right);
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
            "media {} is {:?}; Phase 1 timeline clips require video",
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

fn next_clip_id_excluding(project: &Project, additional: &[&str]) -> String {
    let mut ids: Vec<String> = project.timeline.iter().map(|c| c.id.clone()).collect();
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

    #[test]
    fn insert_inside_clip_splits_and_ripples() {
        let mut p = project();
        add(&mut p, "m1", Some(0.0), Some(10.0)).unwrap();
        insert(&mut p, "m1", 4.0, Some(1.0), Some(3.0)).unwrap();
        assert_eq!(
            p.timeline.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
            ["c1", "c2", "c3"]
        );
        assert_eq!(resolve(&p).last().unwrap().timeline_end, 12.0);
        assert_eq!(
            (p.timeline[2].source_in, p.timeline[2].source_out),
            (4.0, 10.0)
        );
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
        assert_eq!(p.timeline[0].source_out, 3.0);
        assert_eq!(p.timeline[2].source_in, 3.0);
        assert_eq!(duration(&p), 7.0);
    }

    #[test]
    fn remove_maps_final_time_through_speed() {
        let mut p = project();
        add(&mut p, "m1", Some(0.0), Some(9.0)).unwrap();
        speed(&mut p, "c1", 1.5).unwrap();
        remove(&mut p, 1.0, 3.0).unwrap();
        assert_eq!(p.timeline[0].source_out, 1.5);
        assert_eq!(p.timeline[1].source_in, 4.5);
        assert_eq!(duration(&p), 4.0);
    }

    #[test]
    fn remove_middle_splits_and_ripples() {
        let mut p = project();
        add(&mut p, "m1", Some(0.0), Some(10.0)).unwrap();
        remove(&mut p, 3.0, 7.0).unwrap();
        assert_eq!(duration(&p), 6.0);
        assert_eq!(
            (p.timeline[0].source_in, p.timeline[0].source_out),
            (0.0, 3.0)
        );
        assert_eq!(
            (p.timeline[1].source_in, p.timeline[1].source_out),
            (7.0, 10.0)
        );
        assert_ne!(p.timeline[0].id, p.timeline[1].id);
    }
}
