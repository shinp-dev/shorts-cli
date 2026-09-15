use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::compiler::TimeRange;
use crate::error::{Result, VedError, coded, message};
use crate::ffmpeg::{self, OutputKind};
use crate::hashing::sha256_hex;
use crate::phase4;
use crate::project::Project;
use crate::timeline;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoudnessReport {
    pub project_hash: String,
    pub source: String,
    pub integrated_lufs: Option<f64>,
    pub loudness_range_lu: Option<f64>,
    pub true_peak_dbtp: Option<f64>,
    pub threshold_lufs: Option<f64>,
    pub measured_duration: f64,
    pub cache_key: String,
    pub cached: bool,
}

pub struct LoudnessOptions<'a> {
    pub audio: Option<&'a str>,
    pub track: Option<&'a str>,
    pub mix: bool,
    pub from: Option<f64>,
    pub to: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WaveformSummary {
    pub project_hash: String,
    pub track: String,
    pub output: String,
    pub width: u32,
    pub height: u32,
    pub duration: f64,
    pub peaks_output: Option<String>,
}

pub fn loudness(
    project_path: &Path,
    project: &Project,
    options: LoudnessOptions<'_>,
) -> Result<LoudnessReport> {
    let targets = usize::from(options.audio.is_some())
        + usize::from(options.track.is_some())
        + usize::from(options.mix);
    if targets != 1 {
        return Err(coded(
            "loudness_target_invalid",
            "exactly one of --audio, --track, or --mix is required",
        ));
    }
    if options.audio.is_some() && (options.from.is_some() || options.to.is_some()) {
        return Err(coded(
            "loudness_range_invalid",
            "--from and --to are available only for --track or --mix",
        ));
    }
    let total = timeline::duration(project);
    let range = resolve_range(options.from, options.to, total)?;
    let source = if let Some(audio) = options.audio {
        format!("audio:{audio}")
    } else if let Some(track) = options.track {
        format!("track:{track}")
    } else {
        "mix".into()
    };
    let dependency_hashes = dependency_hashes(project_path, project, options.audio)?;
    let cache_identity = serde_json::json!({
        "version": 1,
        "project_hash": phase4::project_hash(project)?,
        "source": source,
        "range": range.as_ref().map(|value| [value.start, value.end]),
        "dependencies": dependency_hashes,
        "engine": "ffmpeg-loudnorm",
        "tool_version": env!("CARGO_PKG_VERSION")
    });
    let key_bytes =
        serde_json::to_vec(&cache_identity).map_err(|error| message(error.to_string()))?;
    let cache_key = format!("sha256:{}", sha256_hex(&key_bytes));
    let cache_path = cache_root(project_path)
        .join("loudness")
        .join(format!("{}.json", &cache_key[7..]));
    if cache_path.is_file() {
        let bytes = fs::read(&cache_path).map_err(|source| VedError::Io {
            path: cache_path.clone(),
            source,
        })?;
        let mut report: LoudnessReport =
            serde_json::from_slice(&bytes).map_err(|source| VedError::Json {
                path: cache_path.clone(),
                source,
            })?;
        if report.cache_key == cache_key {
            report.cached = true;
            return Ok(report);
        }
    }

    let (measurement, measured_duration) = if let Some(audio_id) = options.audio {
        analyze_audio(project_path, project, audio_id)?
    } else {
        analyze_compiled(project_path, project, options.track, range)?
    };
    let report = LoudnessReport {
        project_hash: phase4::project_hash(project)?,
        source,
        integrated_lufs: parse_metric(&measurement.input_i),
        loudness_range_lu: parse_metric(&measurement.input_lra),
        true_peak_dbtp: parse_metric(&measurement.input_tp),
        threshold_lufs: parse_metric(&measurement.input_thresh),
        measured_duration,
        cache_key,
        cached: false,
    };
    write_cache_json(&cache_path, &report)?;
    Ok(report)
}

fn resolve_range(from: Option<f64>, to: Option<f64>, total: f64) -> Result<Option<TimeRange>> {
    if from.is_none() && to.is_none() {
        return Ok(None);
    }
    let start = from.unwrap_or(0.0);
    let end = to.unwrap_or(total);
    if !start.is_finite()
        || !end.is_finite()
        || start < 0.0
        || end <= start
        || end > total + 0.000_001
    {
        return Err(coded(
            "loudness_range_invalid",
            format!("range must be within 0..={total:.3} with --to greater than --from"),
        ));
    }
    Ok(Some(TimeRange { start, end }))
}

fn analyze_audio(
    project_path: &Path,
    project: &Project,
    audio_id: &str,
) -> Result<(LoudnormOutput, f64)> {
    let audio = project
        .audio_clips
        .iter()
        .find(|item| item.id == audio_id)
        .ok_or_else(|| {
            coded(
                "audio_not_found",
                format!("audio clip {audio_id} does not exist"),
            )
        })?;
    let media = project.media_by_id(&audio.media_id)?;
    let path = resolved_media_path(project_path, &media.path);
    let source_out = audio.resolved_source_out(media)?;
    let mut filters = vec![
        format!("atrim=start={}:end={}", audio.source_in, source_out),
        "asetpts=PTS-STARTPTS".into(),
    ];
    filters.extend(atempo_filters(audio.speed));
    filters.push(measure_filter());
    let output = run_loudnorm(&path, &filters.join(","))?;
    Ok((output, (source_out - audio.source_in) / audio.speed))
}

fn analyze_compiled(
    project_path: &Path,
    project: &Project,
    track: Option<&str>,
    range: Option<TimeRange>,
) -> Result<(LoudnormOutput, f64)> {
    let selected = match track {
        Some(track) => selected_track_project(project, track)?,
        None => project.clone(),
    };
    let plan = crate::compiler::compile(project_path, &selected, range)?;
    let temp = tempfile::tempdir().map_err(|source| VedError::Io {
        path: std::env::temp_dir(),
        source,
    })?;
    let rendered = temp.path().join("loudness-source.mkv");
    ffmpeg::run_overwrite(&plan, &rendered, OutputKind::MatroskaPreview, false)?;
    let output = run_loudnorm(&rendered, &measure_filter())?;
    Ok((output, plan.duration))
}

pub fn waveform(
    project_path: &Path,
    project: &Project,
    track: &str,
    output: &Path,
    width: u32,
    peaks_output: Option<&Path>,
    overwrite: bool,
) -> Result<WaveformSummary> {
    if !(320..=8192).contains(&width) {
        return Err(coded(
            "waveform_width_invalid",
            "waveform width must be within 320..=8192",
        ));
    }
    ensure_output_available(output, overwrite)?;
    if let Some(peaks) = peaks_output {
        ensure_output_available(peaks, overwrite)?;
    }
    let selected = selected_track_project(project, track)?;
    let plan = crate::compiler::compile(project_path, &selected, None)?;
    let temp_dir = tempfile::tempdir().map_err(|source| VedError::Io {
        path: std::env::temp_dir(),
        source,
    })?;
    let rendered = temp_dir.path().join("waveform-source.mkv");
    ffmpeg::run_overwrite(&plan, &rendered, OutputKind::MatroskaPreview, false)?;

    let parent = output.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|source| VedError::Io {
        path: parent.to_path_buf(),
        source,
    })?;
    let temporary = unique_output_path(output, "png");
    let grid = (width / 10).max(1);
    let filter = format!(
        "aformat=channel_layouts=mono,showwavespic=s={width}x320:colors=#36c98f,drawgrid=width={grid}:height=80:color=white@0.18"
    );
    let result = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
        .arg(&rendered)
        .args(["-filter_complex", &filter, "-frames:v", "1"])
        .arg(&temporary)
        .output()
        .map_err(|source| VedError::Process {
            program: "ffmpeg".into(),
            source,
        })?;
    if !result.status.success() {
        let _ = fs::remove_file(&temporary);
        return Err(VedError::ProcessFailed {
            program: "ffmpeg".into(),
            code: result
                .status
                .code()
                .map_or_else(|| "terminated".into(), |code| code.to_string()),
            stderr: String::from_utf8_lossy(&result.stderr).trim().to_owned(),
        });
    }
    publish_file(&temporary, output, overwrite)?;

    if let Some(peaks_path) = peaks_output {
        let peaks = extract_peaks(&rendered, 400)?;
        let value = serde_json::json!({
            "version": 1,
            "project_hash": phase4::project_hash(project)?,
            "track": track,
            "duration": plan.duration,
            "sample_points": peaks.len(),
            "peaks": peaks
        });
        write_output_json(peaks_path, &value, overwrite)?;
    }

    Ok(WaveformSummary {
        project_hash: phase4::project_hash(project)?,
        track: track.to_owned(),
        output: absolute_output(output)?,
        width,
        height: 320,
        duration: plan.duration,
        peaks_output: peaks_output.map(absolute_output).transpose()?,
    })
}

fn selected_track_project(project: &Project, track: &str) -> Result<Project> {
    if !project
        .audio_clips
        .iter()
        .any(|audio| audio.track.as_deref() == Some(track) && !audio.mute)
    {
        return Err(coded(
            "track_not_found",
            format!("track {track:?} has no audible audio clips"),
        ));
    }
    let mut selected = project.clone();
    for item in &mut selected.timeline {
        if let Some(clip) = item.clip_mut() {
            clip.mute = true;
        }
    }
    for audio in &mut selected.audio_clips {
        if audio.track.as_deref() != Some(track) {
            audio.mute = true;
        }
    }
    Ok(selected)
}

fn extract_peaks(input: &Path, points: usize) -> Result<Vec<[f32; 2]>> {
    let output = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-i"])
        .arg(input)
        .args(["-vn", "-ac", "1", "-ar", "8000", "-f", "f32le", "-"])
        .output()
        .map_err(|source| VedError::Process {
            program: "ffmpeg".into(),
            source,
        })?;
    if !output.status.success() {
        return Err(VedError::ProcessFailed {
            program: "ffmpeg".into(),
            code: output
                .status
                .code()
                .map_or_else(|| "terminated".into(), |code| code.to_string()),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    let samples = output
        .stdout
        .chunks_exact(4)
        .map(|bytes| f32::from_le_bytes(bytes.try_into().unwrap()))
        .collect::<Vec<_>>();
    if samples.is_empty() {
        return Ok(Vec::new());
    }
    let chunk_size = samples.len().div_ceil(points).max(1);
    Ok(samples
        .chunks(chunk_size)
        .map(|chunk| {
            let mut minimum = f32::INFINITY;
            let mut maximum = f32::NEG_INFINITY;
            for sample in chunk {
                minimum = minimum.min(*sample);
                maximum = maximum.max(*sample);
            }
            [minimum, maximum]
        })
        .collect())
}

fn ensure_output_available(path: &Path, overwrite: bool) -> Result<()> {
    if path.exists() && !overwrite {
        return Err(coded(
            "output_exists",
            format!("output already exists: {}", path.display()),
        ));
    }
    Ok(())
}

fn unique_output_path(output: &Path, extension: &str) -> PathBuf {
    let parent = output.parent().unwrap_or_else(|| Path::new("."));
    let stem = output
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("output");
    parent.join(format!(
        ".{stem}.{}.{}.tmp.{extension}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ))
}

fn publish_file(temporary: &Path, output: &Path, overwrite: bool) -> Result<()> {
    if output.exists() {
        if !overwrite {
            return Err(coded(
                "output_exists",
                format!("output already exists: {}", output.display()),
            ));
        }
        crate::project::replace_existing(temporary, output)
    } else {
        fs::rename(temporary, output).map_err(|source| VedError::Io {
            path: output.to_path_buf(),
            source,
        })
    }
}

fn write_output_json(path: &Path, value: &serde_json::Value, overwrite: bool) -> Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|source| VedError::Io {
        path: parent.to_path_buf(),
        source,
    })?;
    let temporary = unique_output_path(path, "json");
    let mut bytes = serde_json::to_vec_pretty(value).map_err(|error| message(error.to_string()))?;
    bytes.push(b'\n');
    fs::write(&temporary, bytes).map_err(|source| VedError::Io {
        path: temporary.clone(),
        source,
    })?;
    publish_file(&temporary, path, overwrite)
}

fn absolute_output(path: &Path) -> Result<String> {
    fs::canonicalize(path)
        .map(|path| path.to_string_lossy().into_owned())
        .map_err(|source| VedError::Io {
            path: path.to_path_buf(),
            source,
        })
}

fn measure_filter() -> String {
    "loudnorm=I=-23:TP=-2:LRA=7:print_format=json".into()
}

fn atempo_filters(mut speed: f64) -> Vec<String> {
    let mut filters = Vec::new();
    while speed > 2.0 {
        filters.push("atempo=2".into());
        speed /= 2.0;
    }
    while speed < 0.5 {
        filters.push("atempo=0.5".into());
        speed /= 0.5;
    }
    if (speed - 1.0).abs() > 0.000_001 {
        filters.push(format!("atempo={speed:.9}"));
    }
    filters
}

#[derive(Debug, Deserialize)]
struct LoudnormOutput {
    input_i: String,
    input_tp: String,
    input_lra: String,
    input_thresh: String,
}

fn run_loudnorm(input: &Path, filter: &str) -> Result<LoudnormOutput> {
    let output = Command::new("ffmpeg")
        .args(["-hide_banner", "-nostats", "-i"])
        .arg(input)
        .args(["-vn", "-af", filter, "-f", "null", "-"])
        .output()
        .map_err(|source| VedError::Process {
            program: "ffmpeg".into(),
            source,
        })?;
    if !output.status.success() {
        return Err(VedError::ProcessFailed {
            program: "ffmpeg".into(),
            code: output
                .status
                .code()
                .map_or_else(|| "terminated".into(), |code| code.to_string()),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let end = stderr
        .rfind('}')
        .ok_or_else(|| coded("loudness_parse_failed", "FFmpeg returned no loudnorm JSON"))?;
    let start = stderr[..=end].rfind('{').ok_or_else(|| {
        coded(
            "loudness_parse_failed",
            "FFmpeg returned incomplete loudnorm JSON",
        )
    })?;
    serde_json::from_str(&stderr[start..=end]).map_err(|error| {
        coded(
            "loudness_parse_failed",
            format!("invalid loudnorm JSON: {error}"),
        )
    })
}

fn parse_metric(value: &str) -> Option<f64> {
    value.parse().ok().filter(|value: &f64| value.is_finite())
}

fn dependency_hashes(
    project_path: &Path,
    project: &Project,
    audio: Option<&str>,
) -> Result<Vec<String>> {
    let media_ids = if let Some(audio_id) = audio {
        vec![
            project
                .audio_clips
                .iter()
                .find(|item| item.id == audio_id)
                .ok_or_else(|| {
                    coded(
                        "audio_not_found",
                        format!("audio clip {audio_id} does not exist"),
                    )
                })?
                .media_id
                .as_str(),
        ]
    } else {
        project
            .media
            .iter()
            .map(|media| media.id.as_str())
            .collect()
    };
    let mut hashes = Vec::with_capacity(media_ids.len());
    for id in media_ids {
        let media = project.media_by_id(id)?;
        let path = resolved_media_path(project_path, &media.path);
        let bytes = fs::read(&path).map_err(|source| VedError::Io {
            path: path.clone(),
            source,
        })?;
        hashes.push(format!("{}:sha256:{}", media.id, sha256_hex(&bytes)));
    }
    Ok(hashes)
}

fn cache_root(project_path: &Path) -> PathBuf {
    project_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(".ved")
        .join("cache")
}

fn write_cache_json(path: &Path, report: &LoudnessReport) -> Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|source| VedError::Io {
        path: parent.to_path_buf(),
        source,
    })?;
    let bytes = serde_json::to_vec_pretty(report).map_err(|error| message(error.to_string()))?;
    let temp = tempfile::NamedTempFile::new_in(parent).map_err(|source| VedError::Io {
        path: parent.to_path_buf(),
        source,
    })?;
    fs::write(temp.path(), bytes).map_err(|source| VedError::Io {
        path: temp.path().to_path_buf(),
        source,
    })?;
    temp.persist(path).map_err(|error| VedError::Io {
        path: path.to_path_buf(),
        source: error.error,
    })?;
    Ok(())
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
