use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use clap::ValueEnum;
use serde::{Deserialize, Serialize};

use crate::error::{Result, VedError, coded, message};
use crate::ffmpeg::{self, OutputKind, Quality};
use crate::hashing::sha256_hex;
use crate::project::{Project, TimelineItem};
use crate::timeline;

const MAX_BRIEF_BYTES: usize = 32 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum InspectSection {
    Timeline,
    Audio,
    Visual,
    Analysis,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum MarkerFormat {
    Json,
    Csv,
    Vtt,
    Ffmetadata,
}

pub fn project_hash(project: &Project) -> Result<String> {
    let canonical = serde_json::to_vec(project)
        .map_err(|error| message(format!("could not serialize project for hashing: {error}")))?;
    Ok(format!("sha256:{}", sha256_hex(&canonical)))
}

pub fn capabilities(schema: Option<&str>) -> Result<serde_json::Value> {
    if let Some(name) = schema {
        if name != "work_order" {
            return Err(coded(
                "schema_not_found",
                format!("schema {name:?} is not available"),
            ));
        }
        return Ok(serde_json::json!({
            "schema": "work_order",
            "version": 1,
            "json_schema": {
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "type": "object",
                "additionalProperties": false,
                "required": ["version", "project_hash", "objective", "authorized", "changes"],
                "properties": {
                    "version": { "const": 1 },
                    "project_hash": { "type": "string", "pattern": "^sha256:[0-9a-f]{64}$" },
                    "objective": { "type": "string", "maxLength": 500 },
                    "authorized": {
                        "type": "array", "maxItems": 20, "uniqueItems": true,
                        "items": { "enum": ["read_only", "derived_artifact", "project_mutation", "network"] }
                    },
                    "changes": {
                        "type": "array", "maxItems": 20,
                        "items": {
                            "type": "object", "additionalProperties": false,
                            "required": ["target", "instruction"],
                            "properties": {
                                "target": { "type": "string", "maxLength": 64 },
                                "instruction": { "type": "string", "maxLength": 200 }
                            }
                        }
                    },
                    "constraints": { "type": "array", "maxItems": 20, "items": { "type": "string", "maxLength": 200 } },
                    "review_focus": { "type": "array", "maxItems": 20, "items": { "type": "string", "maxLength": 200 } }
                }
            }
        }));
    }

    Ok(serde_json::json!({
        "tool_version": env!("CARGO_PKG_VERSION"),
        "project_versions": { "read": [1, 2, 3], "write": 3 },
        "artifact_versions": {
            "work_order": 1,
            "loudness": 1,
            "waveform_peaks": 1,
            "markers": 1,
            "review_notes": 1,
            "review_bundle": 1
        },
        "features": [
            "audio_sync",
            "project_hash",
            "compact_inspect",
            "work_order_validation",
            "project_check",
            "loudness",
            "waveform",
            "markers",
            "review_bundle"
        ],
        "safety_classes": ["read_only", "derived_artifact", "project_mutation", "network"],
        "network_default": "deny"
    }))
}

pub fn inspect(
    project_path: &Path,
    project: &Project,
    sections: &[InspectSection],
    verbose: bool,
) -> Result<serde_json::Value> {
    let include = |section| sections.is_empty() || sections.contains(&section);
    let mut result = serde_json::json!({
        "project": project_path.to_string_lossy(),
        "project_hash": project_hash(project)?,
        "version": project.version,
        "duration": timeline::duration(project),
        "canvas": project.canvas,
        "counts": {
            "media": project.media.len(),
            "timeline": project.timeline.len(),
            "text": project.text_overlays.len(),
            "images": project.image_overlays.len(),
            "audio": project.audio_clips.len(),
            "ducking": project.audio_ducking.len()
        }
    });
    let object = result.as_object_mut().unwrap();

    if include(InspectSection::Timeline) {
        let entries = timeline::resolve(project)
            .into_iter()
            .map(|resolved| match resolved.item {
                TimelineItem::Clip(clip) => serde_json::json!({
                    "id": clip.id,
                    "kind": "clip",
                    "range": [resolved.timeline_start, resolved.timeline_end],
                    "media": clip.media_id,
                    "source": [clip.source_in, clip.source_out],
                    "speed": clip.speed
                }),
                TimelineItem::Hold(hold) => serde_json::json!({
                    "id": hold.id,
                    "kind": "hold",
                    "range": [resolved.timeline_start, resolved.timeline_end],
                    "media": hold.media_id,
                    "source": [hold.freeze_at, hold.freeze_at],
                    "duration": hold.duration
                }),
            })
            .collect::<Vec<_>>();
        object.insert("timeline".into(), serde_json::Value::Array(entries));
    }

    if include(InspectSection::Audio) {
        let entries = project
            .audio_clips
            .iter()
            .map(|audio| {
                let media = project.media_by_id(&audio.media_id)?;
                Ok(serde_json::json!({
                    "id": audio.id,
                    "track": audio.track,
                    "key": audio.key,
                    "range": [audio.start, audio.resolved_end(media)?],
                    "media": audio.media_id,
                    "source": [audio.source_in, audio.resolved_source_out(media)?],
                    "speed": audio.speed,
                    "volume": audio.volume,
                    "mute": audio.mute,
                    "loop": audio.r#loop
                }))
            })
            .collect::<Result<Vec<_>>>()?;
        object.insert("audio".into(), serde_json::Value::Array(entries));
        object.insert(
            "ducking".into(),
            serde_json::to_value(&project.audio_ducking).unwrap(),
        );
    }

    if include(InspectSection::Visual) {
        let text = project
            .text_overlays
            .iter()
            .map(|overlay| {
                serde_json::json!({
                    "id": overlay.id,
                    "range": [overlay.start, overlay.end],
                    "position": overlay.position,
                    "text": text_digest(&overlay.text)
                })
            })
            .collect::<Vec<_>>();
        let images = project
            .image_overlays
            .iter()
            .map(|overlay| {
                serde_json::json!({
                    "id": overlay.id,
                    "range": [overlay.start, overlay.end],
                    "media": overlay.media_id,
                    "position": overlay.position
                })
            })
            .collect::<Vec<_>>();
        object.insert("text".into(), serde_json::Value::Array(text));
        object.insert("images".into(), serde_json::Value::Array(images));
    }

    if include(InspectSection::Analysis) {
        let cache = project_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(".ved")
            .join("cache")
            .join("loudness");
        let cached_loudness = fs::read_dir(cache)
            .map(|entries| entries.filter_map(std::result::Result::ok).count())
            .unwrap_or(0);
        object.insert(
            "analysis".into(),
            serde_json::json!({
                "loudness": { "status": "available", "cached": cached_loudness },
                "transcript": "not_available",
                "alignment": "not_available",
                "waveform": "available"
            }),
        );
    }
    if verbose {
        object.insert(
            "full_project".into(),
            serde_json::to_value(project).unwrap(),
        );
    }
    Ok(result)
}

fn text_digest(text: &str) -> serde_json::Value {
    let chars = text.chars().count();
    let mut preview = text.chars().take(80).collect::<String>();
    if chars > 80 {
        preview.push('…');
    }
    serde_json::json!({
        "chars": chars,
        "sha256": format!("sha256:{}", sha256_hex(text.as_bytes())),
        "preview": preview
    })
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkOrder {
    version: u32,
    project_hash: String,
    objective: String,
    authorized: Vec<SafetyClass>,
    changes: Vec<WorkChange>,
    #[serde(default)]
    constraints: Vec<String>,
    #[serde(default)]
    review_focus: Vec<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
enum SafetyClass {
    ReadOnly,
    DerivedArtifact,
    ProjectMutation,
    Network,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkChange {
    target: String,
    instruction: String,
}

pub fn brief_check(brief_path: &Path, project: &Project) -> Result<serde_json::Value> {
    let bytes = fs::read(brief_path).map_err(|source| VedError::Io {
        path: brief_path.to_path_buf(),
        source,
    })?;
    if bytes.len() > MAX_BRIEF_BYTES {
        return Err(coded(
            "brief_too_large",
            format!("work order exceeds {MAX_BRIEF_BYTES} bytes"),
        ));
    }
    let brief: WorkOrder = serde_json::from_slice(&bytes).map_err(|source| VedError::Json {
        path: brief_path.to_path_buf(),
        source,
    })?;
    if brief.version != 1 {
        return Err(coded(
            "brief_version_unsupported",
            format!("unsupported work-order version {}", brief.version),
        ));
    }
    let current_hash = project_hash(project)?;
    if brief.project_hash != current_hash {
        return Err(coded(
            "brief_stale",
            format!(
                "work order targets {}, but the project is {current_hash}",
                brief.project_hash
            ),
        ));
    }
    validate_text(&brief.objective, 500, "objective")?;
    validate_array_len(&brief.authorized, "authorized")?;
    validate_array_len(&brief.changes, "changes")?;
    validate_array_len(&brief.constraints, "constraints")?;
    validate_array_len(&brief.review_focus, "review_focus")?;
    if brief.authorized.is_empty() {
        return Err(coded(
            "brief_invalid_authority",
            "work order must authorize at least one safety class",
        ));
    }
    let unique = brief.authorized.iter().copied().collect::<HashSet<_>>();
    if unique.len() != brief.authorized.len() {
        return Err(coded(
            "brief_invalid_authority",
            "work order contains duplicate safety classes",
        ));
    }
    validate_payload_text(&brief.objective, "objective")?;
    for change in &brief.changes {
        validate_text(&change.target, 64, "change target")?;
        validate_text(&change.instruction, 200, "change instruction")?;
        validate_payload_text(&change.instruction, "change instruction")?;
        if !target_exists(project, &change.target) {
            return Err(coded(
                "brief_target_not_found",
                format!("work-order target {:?} does not exist", change.target),
            ));
        }
    }
    for (kind, values) in [
        ("constraint", brief.constraints.as_slice()),
        ("review focus", brief.review_focus.as_slice()),
    ] {
        for value in values {
            validate_text(value, 200, kind)?;
            validate_payload_text(value, kind)?;
        }
    }

    Ok(serde_json::json!({
        "valid": true,
        "version": brief.version,
        "project_hash": current_hash,
        "brief_hash": format!("sha256:{}", sha256_hex(&bytes)),
        "authorized": brief.authorized,
        "changes": brief.changes.len(),
        "constraints": brief.constraints.len(),
        "review_focus": brief.review_focus.len()
    }))
}

fn validate_array_len<T>(values: &[T], name: &str) -> Result<()> {
    if values.len() > 20 {
        return Err(coded(
            "brief_too_large",
            format!("{name} exceeds 20 entries"),
        ));
    }
    Ok(())
}

fn validate_text(value: &str, maximum: usize, name: &str) -> Result<()> {
    let length = value.chars().count();
    if value.is_empty() || length > maximum {
        return Err(coded(
            "brief_text_invalid",
            format!("{name} must contain 1..={maximum} characters"),
        ));
    }
    Ok(())
}

fn validate_payload_text(value: &str, name: &str) -> Result<()> {
    let lower = value.to_ascii_lowercase();
    let forbidden = [
        "api_key",
        "api-key",
        "authorization:",
        "bearer ",
        "password=",
        "secret=",
        "ffmpeg",
        "powershell",
        "cmd.exe",
        "sh -c",
        "&&",
        "||",
        "`",
        "\0",
        "\r",
        "\n",
    ];
    if forbidden.iter().any(|pattern| lower.contains(pattern)) {
        return Err(coded(
            "brief_payload_rejected",
            format!("{name} contains a secret or executable payload"),
        ));
    }
    Ok(())
}

fn target_exists(project: &Project, target: &str) -> bool {
    matches!(target, "project" | "timeline" | "mix")
        || project.media.iter().any(|item| item.id == target)
        || project.timeline.iter().any(|item| item.id() == target)
        || project.text_overlays.iter().any(|item| item.id == target)
        || project.image_overlays.iter().any(|item| item.id == target)
        || project.audio_clips.iter().any(|item| {
            item.id == target
                || item.key.as_deref() == Some(target)
                || item.track.as_deref() == Some(target)
        })
        || project
            .audio_ducking
            .iter()
            .any(|item| item.id == target || item.key == target || item.target_track == target)
}

#[derive(Debug, Clone, Serialize)]
pub struct CheckReport {
    pub project_hash: String,
    pub profile: String,
    pub duration: f64,
    pub counts: IssueCounts,
    pub issues: Vec<CheckIssue>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct IssueCounts {
    pub fail: usize,
    pub warn: usize,
    pub info: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct CheckIssue {
    pub severity: &'static str,
    pub code: &'static str,
    pub target: String,
    pub message: String,
}

pub fn check(project_path: &Path, project: &Project, profile: &str) -> Result<CheckReport> {
    if profile != "shorts" && profile != "default" {
        return Err(coded(
            "check_profile_not_found",
            format!("check profile {profile:?} is not available"),
        ));
    }
    let duration = timeline::duration(project);
    let mut issues = Vec::new();
    if project.timeline.is_empty() {
        add_issue(
            &mut issues,
            "fail",
            "timeline_empty",
            "timeline",
            "timeline has no video items",
        );
    }
    if profile == "shorts" && duration > 60.0 {
        add_issue(
            &mut issues,
            "warn",
            "shorts_duration_long",
            "timeline",
            format!("duration {duration:.3}s exceeds the 60s review target"),
        );
    }
    for media in &project.media {
        let path = resolved_media_path(project_path, &media.path);
        if !path.is_file() {
            add_issue(
                &mut issues,
                "fail",
                "media_missing",
                &media.id,
                format!("media file is missing: {}", path.display()),
            );
        }
    }
    for overlay in &project.text_overlays {
        if overlay.start >= duration || overlay.end > duration + 0.000_001 {
            add_issue(
                &mut issues,
                "warn",
                "text_outside_timeline",
                &overlay.id,
                "text interval extends outside the completed timeline",
            );
        }
    }
    for overlay in &project.image_overlays {
        if overlay.start >= duration || overlay.end > duration + 0.000_001 {
            add_issue(
                &mut issues,
                "warn",
                "image_outside_timeline",
                &overlay.id,
                "image interval extends outside the completed timeline",
            );
        }
    }
    for audio in &project.audio_clips {
        let media = project.media_by_id(&audio.media_id)?;
        if audio.start >= duration || audio.resolved_end(media)? > duration + 0.000_001 {
            add_issue(
                &mut issues,
                "warn",
                "audio_outside_timeline",
                &audio.id,
                "audio interval extends outside the completed timeline",
            );
        }
    }
    let tracks = project
        .audio_clips
        .iter()
        .filter_map(|audio| audio.track.as_deref())
        .collect::<HashSet<_>>();
    for duck in &project.audio_ducking {
        for trigger in &duck.trigger_tracks {
            if !tracks.contains(trigger.as_str()) {
                add_issue(
                    &mut issues,
                    "warn",
                    "duck_trigger_track_empty",
                    &duck.id,
                    format!("trigger track {trigger:?} has no audio clips"),
                );
            }
        }
    }

    let mut counts = IssueCounts::default();
    for issue in &issues {
        match issue.severity {
            "fail" => counts.fail += 1,
            "warn" => counts.warn += 1,
            _ => counts.info += 1,
        }
    }
    Ok(CheckReport {
        project_hash: project_hash(project)?,
        profile: profile.to_owned(),
        duration,
        counts,
        issues,
    })
}

fn add_issue(
    issues: &mut Vec<CheckIssue>,
    severity: &'static str,
    code: &'static str,
    target: impl Into<String>,
    message: impl Into<String>,
) {
    issues.push(CheckIssue {
        severity,
        code,
        target: target.into(),
        message: message.into(),
    });
}

fn resolved_media_path(project_path: &Path, stored: &str) -> PathBuf {
    let path = PathBuf::from(stored.replace('/', std::path::MAIN_SEPARATOR_STR));
    if path.is_absolute() {
        path
    } else {
        project_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(path)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ReviewNotes {
    version: u32,
    summary: String,
    #[serde(default)]
    changes: Vec<ReviewChange>,
    #[serde(default)]
    decisions: Vec<ReviewDecision>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ReviewChange {
    time: f64,
    text: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ReviewDecision {
    time: f64,
    question: String,
    options: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReviewBuildSummary {
    pub changed: bool,
    pub review_id: String,
    pub project_hash: String,
    pub video: String,
    pub report: String,
    pub storyboard: String,
    pub checks: IssueCounts,
    pub decisions: usize,
}

pub struct ReviewBuildOptions<'a> {
    pub output_root: &'a Path,
    pub notes: Option<&'a Path>,
    pub profile: &'a str,
    pub since: Option<&'a Path>,
    pub overwrite: bool,
}

pub fn review_build(
    project_path: &Path,
    project: &Project,
    options: ReviewBuildOptions<'_>,
) -> Result<ReviewBuildSummary> {
    let mut check_report = check(project_path, project, options.profile)?;
    if check_report.counts.fail > 0 {
        return Err(coded(
            "check_failed",
            format!(
                "review build stopped because {} required checks failed",
                check_report.counts.fail
            ),
        ));
    }
    if timeline::duration(project) <= 0.0 {
        return Err(coded("check_failed", "review build requires video content"));
    }
    let loudness = crate::audio_analysis::loudness(
        project_path,
        project,
        crate::audio_analysis::LoudnessOptions {
            audio: None,
            track: None,
            mix: true,
            from: None,
            to: None,
        },
    )?;
    if loudness
        .integrated_lufs
        .is_none_or(|value| !(-15.0..=-13.0).contains(&value))
    {
        add_issue(
            &mut check_report.issues,
            "warn",
            "mix_loudness_out_of_range",
            "mix",
            "final mix is outside the shorts-mix target of -14 LUFS +/- 1",
        );
        check_report.counts.warn += 1;
    }
    if loudness.true_peak_dbtp.is_some_and(|value| value > -1.0) {
        add_issue(
            &mut check_report.issues,
            "fail",
            "mix_true_peak_exceeded",
            "mix",
            "final mix exceeds the shorts-mix maximum of -1 dBTP",
        );
        check_report.counts.fail += 1;
        return Err(coded(
            "check_failed",
            "review build stopped because final-mix true peak exceeds -1 dBTP",
        ));
    }
    let (notes, notes_bytes) = load_review_notes(options.notes)?;
    let project_hash = project_hash(project)?;
    let identity = serde_json::json!({
        "project_hash": project_hash,
        "render_range": [0.0, timeline::duration(project)],
        "preview_profile": "preview",
        "check_profile": options.profile,
        "notes_hash": format!("sha256:{}", sha256_hex(&notes_bytes)),
        "tool_version": env!("CARGO_PKG_VERSION")
    });
    let identity_bytes =
        serde_json::to_vec(&identity).map_err(|error| message(error.to_string()))?;
    let review_id = sha256_hex(&identity_bytes)[..12].to_owned();
    let project_stem = project_path
        .file_stem()
        .and_then(|value| value.to_str())
        .ok_or_else(|| message("project path must have a valid file stem"))?;
    let parent = options.output_root.join(project_stem);
    let final_dir = parent.join(&review_id);

    if final_dir.exists() && verified_review_bundle(&final_dir, &review_id, &project_hash) {
        return review_summary(
            false,
            &final_dir,
            review_id,
            project_hash,
            check_report.counts,
            notes.decisions.len(),
        );
    }
    if final_dir.exists() && !options.overwrite {
        return Err(coded(
            "output_exists",
            format!(
                "review destination exists but is incomplete; use --overwrite: {}",
                final_dir.display()
            ),
        ));
    }
    fs::create_dir_all(&parent).map_err(|source| VedError::Io {
        path: parent.clone(),
        source,
    })?;
    let temporary = tempfile::Builder::new()
        .prefix(".review-build-")
        .tempdir_in(&parent)
        .map_err(|source| VedError::Io {
            path: parent.clone(),
            source,
        })?;
    let preview_path = temporary.path().join("preview.mp4");
    let storyboard_path = temporary.path().join("storyboard.jpg");
    let markers_path = temporary.path().join("markers.json");
    let review_json_path = temporary.path().join("review.json");
    let report_path = temporary.path().join("review.md");

    let plan = crate::compiler::compile(project_path, project, None)?;
    ffmpeg::run_overwrite(
        &plan,
        &preview_path,
        OutputKind::Mp4(Quality::Preview),
        false,
    )?;
    build_storyboard(&preview_path, &storyboard_path, plan.duration)?;
    let markers = build_markers(project, &project_hash, &[])?;
    write_json(&markers_path, &markers)?;
    let inventory = build_inventory(project)?;
    let previous = load_previous_review(options.since, &inventory, &project_hash)?;
    let review = serde_json::json!({
        "version": 1,
        "review_id": review_id,
        "project_hash": project_hash,
        "tool_version": env!("CARGO_PKG_VERSION"),
        "duration": plan.duration,
        "artifacts": {
            "video": "preview.mp4",
            "storyboard": "storyboard.jpg",
            "markers": "markers.json",
            "report": "review.md"
        },
        "changes": previous,
        "inventory": inventory,
        "loudness": loudness,
        "checks": check_report,
        "notes": notes
    });
    write_json(&review_json_path, &review)?;
    let markdown = build_review_markdown(
        &review_id,
        &project_hash,
        plan.duration,
        &check_report,
        &loudness,
        &notes,
    );
    fs::write(&report_path, markdown.as_bytes()).map_err(|source| VedError::Io {
        path: report_path.clone(),
        source,
    })?;

    publish_review_directory(temporary.path(), &final_dir, options.overwrite)?;
    // The directory was renamed; prevent TempDir's cleanup from following future paths.
    let _ = temporary.keep();
    review_summary(
        true,
        &final_dir,
        review_id,
        project_hash,
        check_report.counts,
        notes.decisions.len(),
    )
}

fn load_review_notes(path: Option<&Path>) -> Result<(ReviewNotes, Vec<u8>)> {
    let Some(path) = path else {
        let notes = ReviewNotes {
            version: 1,
            summary: "No director notes supplied.".into(),
            changes: Vec::new(),
            decisions: Vec::new(),
        };
        return Ok((notes, Vec::new()));
    };
    let bytes = fs::read(path).map_err(|source| VedError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    if bytes.len() > MAX_BRIEF_BYTES {
        return Err(coded(
            "review_notes_too_large",
            "review notes exceed 32 KiB",
        ));
    }
    let notes: ReviewNotes = serde_json::from_slice(&bytes).map_err(|source| VedError::Json {
        path: path.to_path_buf(),
        source,
    })?;
    if notes.version != 1 {
        return Err(coded(
            "review_notes_version_unsupported",
            format!("unsupported review-notes version {}", notes.version),
        ));
    }
    validate_text(&notes.summary, 500, "review summary")?;
    if notes.changes.len() > 20 || notes.decisions.len() > 10 {
        return Err(coded(
            "review_notes_too_large",
            "review notes allow at most 20 changes and 10 decisions",
        ));
    }
    for change in &notes.changes {
        validate_review_time(change.time, "change time")?;
        validate_text(&change.text, 200, "change text")?;
        validate_payload_text(&change.text, "change text")?;
    }
    for decision in &notes.decisions {
        validate_review_time(decision.time, "decision time")?;
        validate_text(&decision.question, 200, "decision question")?;
        if decision.options.is_empty() || decision.options.len() > 10 {
            return Err(coded(
                "review_notes_invalid",
                "each decision requires 1..=10 options",
            ));
        }
        for option in &decision.options {
            validate_text(option, 200, "decision option")?;
        }
    }
    Ok((notes, bytes))
}

fn validate_review_time(value: f64, name: &str) -> Result<()> {
    if !value.is_finite() || value < 0.0 {
        return Err(coded(
            "review_notes_invalid",
            format!("{name} must be a finite non-negative number"),
        ));
    }
    Ok(())
}

fn build_storyboard(preview: &Path, output: &Path, duration: f64) -> Result<()> {
    let fps = 12.0 / duration.max(0.001);
    let filter = format!("fps={fps:.9},scale=320:-2,tile=4x3:padding=4:margin=4");
    let result = std::process::Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
        .arg(preview)
        .args(["-vf", &filter, "-frames:v", "1"])
        .arg(output)
        .output()
        .map_err(|source| VedError::Process {
            program: "ffmpeg".into(),
            source,
        })?;
    if !result.status.success() {
        return Err(VedError::ProcessFailed {
            program: "ffmpeg".into(),
            code: result
                .status
                .code()
                .map_or_else(|| "signal".into(), |code| code.to_string()),
            stderr: String::from_utf8_lossy(&result.stderr).trim().to_owned(),
        });
    }
    Ok(())
}

fn build_markers(
    project: &Project,
    project_hash: &str,
    includes: &[String],
) -> Result<serde_json::Value> {
    for include in includes {
        if !matches!(include.as_str(), "all" | "timeline" | "audio" | "visual") {
            return Err(coded(
                "marker_source_not_available",
                format!("marker source {include:?} is not available"),
            ));
        }
    }
    let include = |kind: &str| {
        includes.is_empty()
            || includes.iter().any(|value| value == "all")
            || includes.iter().any(|value| value == kind)
    };
    let mut markers = Vec::new();
    if include("timeline") {
        for item in timeline::resolve(project) {
            let kind = if item.item.hold().is_some() {
                "hold"
            } else {
                "timeline_item"
            };
            markers.push(marker(
                item.timeline_start,
                kind,
                item.item.id(),
                item.item.id(),
            ));
        }
    }
    if include("visual") {
        for change in &project.text_overlays {
            markers.push(marker(change.start, "text", &change.id, &change.id));
        }
        for image in &project.image_overlays {
            markers.push(marker(image.start, "image", &image.id, &image.id));
        }
    }
    if include("audio") {
        for audio in &project.audio_clips {
            let label = match (&audio.track, &audio.key) {
                (Some(track), Some(key)) => format!("{track}/{key}"),
                _ => audio.id.clone(),
            };
            let mut value = marker(audio.start, "audio", &audio.id, &label);
            value["track"] = serde_json::to_value(&audio.track).unwrap();
            value["key"] = serde_json::to_value(&audio.key).unwrap();
            markers.push(value);
        }
    }
    markers.sort_by(|left, right| {
        left["time"]
            .as_f64()
            .partial_cmp(&right["time"].as_f64())
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left["target"].as_str().cmp(&right["target"].as_str()))
    });
    Ok(serde_json::json!({
        "version": 1,
        "project_hash": project_hash,
        "markers": markers
    }))
}

fn marker(time: f64, kind: &str, target: &str, label: &str) -> serde_json::Value {
    let identity = format!("{kind}\0{target}\0{time:.9}");
    serde_json::json!({
        "id": format!("mk-{}", &sha256_hex(identity.as_bytes())[..16]),
        "time": time,
        "type": kind,
        "target": target,
        "label": label
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct MarkerExportSummary {
    pub project_hash: String,
    pub output: String,
    pub format: &'static str,
    pub markers: usize,
}

pub fn export_markers(
    project: &Project,
    includes: &[String],
    format: MarkerFormat,
    output: &Path,
    overwrite: bool,
) -> Result<MarkerExportSummary> {
    if output.exists() && !overwrite {
        return Err(coded(
            "output_exists",
            format!("output already exists: {}", output.display()),
        ));
    }
    let project_hash = project_hash(project)?;
    let artifact = build_markers(project, &project_hash, includes)?;
    let markers = artifact["markers"].as_array().unwrap();
    let (format_name, bytes) = match format {
        MarkerFormat::Json => (
            "json",
            serde_json::to_vec_pretty(&artifact).map_err(|error| message(error.to_string()))?,
        ),
        MarkerFormat::Csv => ("csv", marker_csv(markers).into_bytes()),
        MarkerFormat::Vtt => ("vtt", marker_vtt(markers).into_bytes()),
        MarkerFormat::Ffmetadata => ("ffmetadata", marker_ffmetadata(markers).into_bytes()),
    };
    write_atomic_output(output, &bytes, overwrite)?;
    Ok(MarkerExportSummary {
        project_hash,
        output: fs::canonicalize(output)
            .map_err(|source| VedError::Io {
                path: output.to_path_buf(),
                source,
            })?
            .to_string_lossy()
            .into_owned(),
        format: format_name,
        markers: markers.len(),
    })
}

fn marker_csv(markers: &[serde_json::Value]) -> String {
    let mut output = "id,time,type,target,label\n".to_owned();
    for marker in markers {
        let values = [
            marker["id"].as_str().unwrap_or_default().to_owned(),
            format!("{:.3}", marker["time"].as_f64().unwrap_or_default()),
            marker["type"].as_str().unwrap_or_default().to_owned(),
            marker["target"].as_str().unwrap_or_default().to_owned(),
            marker["label"].as_str().unwrap_or_default().to_owned(),
        ];
        output.push_str(
            &values
                .iter()
                .map(|value| format!("\"{}\"", value.replace('"', "\"\"")))
                .collect::<Vec<_>>()
                .join(","),
        );
        output.push('\n');
    }
    output
}

fn marker_vtt(markers: &[serde_json::Value]) -> String {
    let mut output = "WEBVTT\n\n".to_owned();
    for marker in markers {
        let start = marker["time"].as_f64().unwrap_or_default();
        output.push_str(&format!(
            "{}\n{} --> {}\n{}: {}\n\n",
            marker["id"].as_str().unwrap_or_default(),
            vtt_time(start),
            vtt_time(start + 0.001),
            marker["type"].as_str().unwrap_or_default(),
            one_line(marker["label"].as_str().unwrap_or_default())
        ));
    }
    output
}

fn vtt_time(seconds: f64) -> String {
    let milliseconds = (seconds * 1000.0).round() as u64;
    format!(
        "{:02}:{:02}:{:02}.{:03}",
        milliseconds / 3_600_000,
        milliseconds / 60_000 % 60,
        milliseconds / 1000 % 60,
        milliseconds % 1000
    )
}

fn marker_ffmetadata(markers: &[serde_json::Value]) -> String {
    let mut output = ";FFMETADATA1\n".to_owned();
    for marker in markers {
        let start = (marker["time"].as_f64().unwrap_or_default() * 1000.0).round() as u64;
        output.push_str(&format!(
            "[CHAPTER]\nTIMEBASE=1/1000\nSTART={start}\nEND={}\ntitle={}\n",
            start + 1,
            one_line(marker["label"].as_str().unwrap_or_default())
                .replace('=', "\\=")
                .replace(';', "\\;")
        ));
    }
    output
}

fn write_atomic_output(output: &Path, bytes: &[u8], overwrite: bool) -> Result<()> {
    let parent = output.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|source| VedError::Io {
        path: parent.to_path_buf(),
        source,
    })?;
    let extension = output
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("tmp");
    let temporary = parent.join(format!(
        ".markers.{}.{}.tmp.{extension}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    fs::write(&temporary, bytes).map_err(|source| VedError::Io {
        path: temporary.clone(),
        source,
    })?;
    let result = if output.exists() {
        if !overwrite {
            Err(coded(
                "output_exists",
                format!("output already exists: {}", output.display()),
            ))
        } else {
            crate::project::replace_existing(&temporary, output)
        }
    } else {
        fs::rename(&temporary, output).map_err(|source| VedError::Io {
            path: output.to_path_buf(),
            source,
        })
    };
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn build_inventory(project: &Project) -> Result<serde_json::Value> {
    let mut inventory = serde_json::Map::new();
    for item in &project.timeline {
        insert_inventory(&mut inventory, item.id(), item)?;
    }
    for item in &project.text_overlays {
        insert_inventory(&mut inventory, &item.id, item)?;
    }
    for item in &project.image_overlays {
        insert_inventory(&mut inventory, &item.id, item)?;
    }
    for item in &project.audio_clips {
        insert_inventory(&mut inventory, &item.id, item)?;
    }
    for item in &project.audio_ducking {
        insert_inventory(&mut inventory, &item.id, item)?;
    }
    Ok(serde_json::Value::Object(inventory))
}

fn insert_inventory<T: Serialize>(
    inventory: &mut serde_json::Map<String, serde_json::Value>,
    id: &str,
    value: &T,
) -> Result<()> {
    let bytes = serde_json::to_vec(value).map_err(|error| message(error.to_string()))?;
    inventory.insert(
        id.to_owned(),
        serde_json::Value::String(format!("sha256:{}", sha256_hex(&bytes))),
    );
    Ok(())
}

fn load_previous_review(
    path: Option<&Path>,
    current_inventory: &serde_json::Value,
    current_project_hash: &str,
) -> Result<serde_json::Value> {
    let Some(path) = path else {
        return Ok(serde_json::json!({
            "since_review_id": null,
            "project_changed": null,
            "added": 0,
            "removed": 0,
            "changed": 0
        }));
    };
    let bytes = fs::read(path).map_err(|source| VedError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|source| VedError::Json {
            path: path.to_path_buf(),
            source,
        })?;
    let review_id = value["review_id"].as_str().ok_or_else(|| {
        coded(
            "previous_review_invalid",
            "previous review has no review_id",
        )
    })?;
    let previous_hash = value["project_hash"].as_str().ok_or_else(|| {
        coded(
            "previous_review_invalid",
            "previous review has no project_hash",
        )
    })?;
    let previous_inventory = value["inventory"].as_object().ok_or_else(|| {
        coded(
            "previous_review_invalid",
            "previous review has no stable-ID inventory",
        )
    })?;
    let current_inventory = current_inventory.as_object().unwrap();
    let added = current_inventory
        .keys()
        .filter(|id| !previous_inventory.contains_key(*id))
        .count();
    let removed = previous_inventory
        .keys()
        .filter(|id| !current_inventory.contains_key(*id))
        .count();
    let changed = current_inventory
        .iter()
        .filter(|(id, hash)| previous_inventory.get(*id).is_some_and(|old| old != *hash))
        .count();
    Ok(serde_json::json!({
        "since_review_id": review_id,
        "previous_project_hash": previous_hash,
        "project_changed": previous_hash != current_project_hash,
        "added": added,
        "removed": removed,
        "changed": changed
    }))
}

fn write_json(path: &Path, value: &serde_json::Value) -> Result<()> {
    let mut bytes = serde_json::to_vec_pretty(value).map_err(|error| message(error.to_string()))?;
    bytes.push(b'\n');
    fs::write(path, bytes).map_err(|source| VedError::Io {
        path: path.to_path_buf(),
        source,
    })
}

fn build_review_markdown(
    review_id: &str,
    project_hash: &str,
    duration: f64,
    checks: &CheckReport,
    loudness: &crate::audio_analysis::LoudnessReport,
    notes: &ReviewNotes,
) -> String {
    let mut report = format!(
        "# ved review {review_id}\n\nProject: `{project_hash}`\nDuration: {duration:.3}s\nLoudness: {} LUFS / {} dBTP\n\n## Summary\n\n{}\n",
        loudness
            .integrated_lufs
            .map_or_else(|| "-inf".into(), |value| format!("{value:.2}")),
        loudness
            .true_peak_dbtp
            .map_or_else(|| "-inf".into(), |value| format!("{value:.2}")),
        one_line(&notes.summary)
    );
    if !notes.changes.is_empty() {
        report.push_str("\n## Changed intervals\n\n");
        for change in &notes.changes {
            report.push_str(&format!(
                "- {:.3}s: {}\n",
                change.time,
                one_line(&change.text)
            ));
        }
    }
    if !checks.issues.is_empty() {
        report.push_str("\n## Checks\n\n");
        for issue in &checks.issues {
            report.push_str(&format!(
                "- {} `{}` {}: {}\n",
                issue.severity,
                issue.code,
                issue.target,
                one_line(&issue.message)
            ));
        }
    }
    if !notes.decisions.is_empty() {
        report.push_str("\n## Decisions\n\n");
        for decision in &notes.decisions {
            report.push_str(&format!(
                "- {:.3}s: {} ({})\n",
                decision.time,
                one_line(&decision.question),
                decision
                    .options
                    .iter()
                    .map(|option| one_line(option))
                    .collect::<Vec<_>>()
                    .join(" / ")
            ));
        }
    }
    report.push_str("\nArtifacts: `preview.mp4`, `storyboard.jpg`, `markers.json`, `review.json`.\nTransfer the actual files when reviewing in a separate chat.\n");
    truncate_utf8(report, 2048)
}

fn one_line(value: &str) -> String {
    value.replace(['\r', '\n'], " ")
}

fn truncate_utf8(mut value: String, maximum_bytes: usize) -> String {
    if value.len() <= maximum_bytes {
        return value;
    }
    let suffix = "\n… report truncated; see review.json for full evidence.\n";
    let mut end = maximum_bytes.saturating_sub(suffix.len());
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value.truncate(end);
    value.push_str(suffix);
    value
}

fn verified_review_bundle(directory: &Path, review_id: &str, project_hash: &str) -> bool {
    for name in [
        "preview.mp4",
        "storyboard.jpg",
        "markers.json",
        "review.json",
        "review.md",
    ] {
        if !directory.join(name).is_file() {
            return false;
        }
    }
    let Ok(bytes) = fs::read(directory.join("review.json")) else {
        return false;
    };
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return false;
    };
    value["review_id"] == review_id && value["project_hash"] == project_hash
}

fn publish_review_directory(temporary: &Path, destination: &Path, overwrite: bool) -> Result<()> {
    if !destination.exists() {
        return fs::rename(temporary, destination).map_err(|source| VedError::Io {
            path: destination.to_path_buf(),
            source,
        });
    }
    if !overwrite {
        return Err(coded(
            "output_exists",
            format!(
                "review destination already exists: {}",
                destination.display()
            ),
        ));
    }
    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    let name = destination
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("review");
    let backup = parent.join(format!(".{name}.{}.backup", std::process::id()));
    if backup.exists() {
        return Err(coded(
            "output_busy",
            format!("review backup path already exists: {}", backup.display()),
        ));
    }
    fs::rename(destination, &backup).map_err(|source| VedError::Io {
        path: destination.to_path_buf(),
        source,
    })?;
    if let Err(source) = fs::rename(temporary, destination) {
        let _ = fs::rename(&backup, destination);
        return Err(VedError::Io {
            path: destination.to_path_buf(),
            source,
        });
    }
    remove_path(&backup)?;
    Ok(())
}

fn remove_path(path: &Path) -> Result<()> {
    let result = if path.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    };
    result.map_err(|source| VedError::Io {
        path: path.to_path_buf(),
        source,
    })
}

fn review_summary(
    changed: bool,
    directory: &Path,
    review_id: String,
    project_hash: String,
    checks: IssueCounts,
    decisions: usize,
) -> Result<ReviewBuildSummary> {
    let directory = fs::canonicalize(directory).map_err(|source| VedError::Io {
        path: directory.to_path_buf(),
        source,
    })?;
    Ok(ReviewBuildSummary {
        changed,
        review_id,
        project_hash,
        video: directory.join("preview.mp4").to_string_lossy().into_owned(),
        report: directory.join("review.md").to_string_lossy().into_owned(),
        storyboard: directory
            .join("storyboard.jpg")
            .to_string_lossy()
            .into_owned(),
        checks,
        decisions,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{Canvas, Project};

    #[test]
    fn project_hash_is_deterministic_sha256() {
        let project = Project::new(Canvas {
            width: 1080,
            height: 1920,
            fps: 30,
        });
        let first = project_hash(&project).unwrap();
        assert_eq!(first, project_hash(&project).unwrap());
        assert_eq!(first.len(), 71);
        assert!(first.starts_with("sha256:"));
    }
}
