use std::path::Path;
use std::process::Command;

use serde::Deserialize;

use crate::error::{Result, VedError, message};
use crate::project::{MediaKind, MediaProbe};

#[derive(Debug)]
pub struct ToolStatus {
    pub name: &'static str,
    pub available: bool,
    pub version: Option<String>,
    pub detail: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ProbeOutput {
    #[serde(default)]
    streams: Vec<ProbeStream>,
    format: Option<ProbeFormat>,
}

#[derive(Debug, Deserialize)]
struct ProbeStream {
    codec_type: Option<String>,
    codec_name: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    avg_frame_rate: Option<String>,
    duration: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ProbeFormat {
    duration: Option<String>,
}

pub fn probe(path: &Path) -> Result<(MediaKind, MediaProbe)> {
    let mut command = Command::new("ffprobe");
    command.args([
        "-v",
        "error",
        "-show_streams",
        "-show_format",
        "-of",
        "json",
    ]);
    if is_static_image(path) {
        command.args(["-f", "image2", "-pattern_type", "none"]);
    }
    let output = command
        .arg(path)
        .output()
        .map_err(|source| VedError::Process {
            program: "ffprobe".into(),
            source,
        })?;
    if !output.status.success() {
        return Err(VedError::ProcessFailed {
            program: "ffprobe".into(),
            code: exit_code(&output.status),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    let parsed: ProbeOutput = serde_json::from_slice(&output.stdout)
        .map_err(|error| message(format!("ffprobe returned invalid JSON: {error}")))?;
    let video = parsed
        .streams
        .iter()
        .find(|stream| stream.codec_type.as_deref() == Some("video"));
    let audio = parsed
        .streams
        .iter()
        .find(|stream| stream.codec_type.as_deref() == Some("audio"));
    let duration = parsed
        .format
        .as_ref()
        .and_then(|format| parse_number(format.duration.as_deref()))
        .or_else(|| video.and_then(|stream| parse_number(stream.duration.as_deref())))
        .or_else(|| audio.and_then(|stream| parse_number(stream.duration.as_deref())));
    let kind = match (is_static_image(path), video, audio, duration) {
        (true, Some(_), _, _) => MediaKind::Image,
        (_, Some(_), _, Some(_)) => MediaKind::Video,
        (_, Some(_), _, None) => MediaKind::Image,
        (_, None, Some(_), _) => MediaKind::Audio,
        _ => {
            return Err(message(format!(
                "{} has no supported media stream",
                path.display()
            )));
        }
    };
    Ok((
        kind,
        MediaProbe {
            duration: (kind != MediaKind::Image).then_some(duration).flatten(),
            width: video.and_then(|stream| stream.width),
            height: video.and_then(|stream| stream.height),
            fps: (kind != MediaKind::Image)
                .then(|| video.and_then(|stream| parse_rate(stream.avg_frame_rate.as_deref())))
                .flatten(),
            has_audio: audio.is_some(),
            video_codec: video.and_then(|stream| stream.codec_name.clone()),
            audio_codec: audio.and_then(|stream| stream.codec_name.clone()),
        },
    ))
}

fn is_static_image(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "png" | "jpg" | "jpeg"
            )
        })
}

pub fn doctor() -> Vec<ToolStatus> {
    let mut statuses: Vec<ToolStatus> = ["ffmpeg", "ffprobe", "ffplay"]
        .into_iter()
        .map(tool_status)
        .collect();

    let encoders = Command::new("ffmpeg")
        .args(["-hide_banner", "-encoders"])
        .output();
    let codec_detail = match encoders {
        Ok(output) if output.status.success() => {
            let text = String::from_utf8_lossy(&output.stdout);
            let h264 = if text.contains("libx264") {
                "libx264 ok"
            } else {
                "libx264 missing"
            };
            let aac = if text.lines().any(|line| line.contains(" aac ")) {
                "aac ok"
            } else {
                "aac missing"
            };
            format!("{h264}, {aac}")
        }
        _ => "codec check unavailable".to_owned(),
    };
    statuses.push(ToolStatus {
        name: "codecs",
        available: codec_detail == "libx264 ok, aac ok",
        version: None,
        detail: Some(codec_detail),
    });
    for status in crate::tts::doctor() {
        statuses.push(ToolStatus {
            name: status.name,
            available: status.available,
            version: None,
            detail: Some(format!("{}; {}", status.endpoint, status.detail)),
        });
    }
    statuses
}

fn tool_status(name: &'static str) -> ToolStatus {
    match Command::new(name).arg("-version").output() {
        Ok(output) if output.status.success() => ToolStatus {
            name,
            available: true,
            version: String::from_utf8_lossy(&output.stdout)
                .lines()
                .next()
                .map(str::to_owned),
            detail: None,
        },
        Ok(output) => ToolStatus {
            name,
            available: false,
            version: None,
            detail: Some(String::from_utf8_lossy(&output.stderr).trim().to_owned()),
        },
        Err(error) => ToolStatus {
            name,
            available: false,
            version: None,
            detail: Some(error.to_string()),
        },
    }
}

fn parse_number(value: Option<&str>) -> Option<f64> {
    value?
        .parse()
        .ok()
        .filter(|value: &f64| value.is_finite() && *value >= 0.0)
}

fn parse_rate(value: Option<&str>) -> Option<f64> {
    let (numerator, denominator) = value?.split_once('/')?;
    let numerator: f64 = numerator.parse().ok()?;
    let denominator: f64 = denominator.parse().ok()?;
    (denominator != 0.0).then_some(numerator / denominator)
}

fn exit_code(status: &std::process::ExitStatus) -> String {
    status
        .code()
        .map(|code| code.to_string())
        .unwrap_or_else(|| "terminated".into())
}
