use std::ffi::OsString;
use std::path::Path;
use std::process::{Command, Stdio};

use clap::ValueEnum;

use crate::compiler::RenderPlan;
use crate::error::{Result, VedError, message};

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum Quality {
    Preview,
    Normal,
    High,
}

#[derive(Debug, Clone, Copy)]
pub enum OutputKind {
    Mp4(Quality),
    MatroskaPreview,
    Frame,
}

pub fn build_args(plan: &RenderPlan, output: &Path, kind: OutputKind) -> Vec<OsString> {
    let mut args = vec![
        OsString::from("-hide_banner"),
        OsString::from("-loglevel"),
        OsString::from("warning"),
    ];
    for segment in &plan.segments {
        args.push("-ss".into());
        args.push(format_number(segment.source_in).into());
        args.push("-t".into());
        args.push(format_number(segment.source_out - segment.source_in).into());
        args.push("-i".into());
        args.push(segment.path.as_os_str().to_owned());
    }
    args.push("-filter_complex".into());
    let mut graph = filter_graph(plan);
    if matches!(kind, OutputKind::Frame) {
        graph.push_str(";[aout]anullsink");
    }
    args.push(graph.into());
    args.extend([OsString::from("-map"), OsString::from("[vout]")]);
    if !matches!(kind, OutputKind::Frame) {
        args.extend([OsString::from("-map"), OsString::from("[aout]")]);
    }

    match kind {
        OutputKind::Mp4(quality) => {
            let (preset, crf) = match quality {
                Quality::Preview => ("ultrafast", "28"),
                Quality::Normal => ("medium", "23"),
                Quality::High => ("slow", "18"),
            };
            args.extend(strings(&[
                "-c:v",
                "libx264",
                "-preset",
                preset,
                "-crf",
                crf,
                "-c:a",
                "aac",
                "-b:a",
                "192k",
                "-movflags",
                "+faststart",
            ]));
            args.extend([OsString::from("-t"), format_number(plan.duration).into()]);
        }
        OutputKind::MatroskaPreview => {
            args.extend(strings(&[
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
                "-crf",
                "30",
                "-c:a",
                "aac",
                "-f",
                "matroska",
            ]));
            args.extend([OsString::from("-t"), format_number(plan.duration).into()]);
        }
        OutputKind::Frame => {
            args.extend(strings(&["-frames:v", "1", "-update", "1"]));
        }
    }
    args.push(output.as_os_str().to_owned());
    args
}

pub fn run(plan: &RenderPlan, output: &Path, kind: OutputKind) -> Result<()> {
    if output.as_os_str() != "-" && output.exists() {
        return Err(message(format!(
            "output already exists: {}",
            output.display()
        )));
    }
    ensure_output_is_not_input(plan, output)?;
    let status = Command::new("ffmpeg")
        .args(build_args(plan, output, kind))
        .status()
        .map_err(|source| VedError::Process {
            program: "ffmpeg".into(),
            source,
        })?;
    if !status.success() {
        return Err(VedError::ProcessFailed {
            program: "ffmpeg".into(),
            code: status
                .code()
                .map(|code| code.to_string())
                .unwrap_or_else(|| "terminated".into()),
            stderr: "see ffmpeg diagnostics above".into(),
        });
    }
    Ok(())
}

pub fn play(plan: &RenderPlan) -> Result<()> {
    let mut ffmpeg = Command::new("ffmpeg")
        .args(build_args(
            plan,
            Path::new("pipe:1"),
            OutputKind::MatroskaPreview,
        ))
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|source| VedError::Process {
            program: "ffmpeg".into(),
            source,
        })?;
    let stdout = ffmpeg
        .stdout
        .take()
        .ok_or_else(|| message("could not open ffmpeg output pipe"))?;
    let status = Command::new("ffplay")
        .args([
            "-hide_banner",
            "-loglevel",
            "warning",
            "-autoexit",
            "-i",
            "pipe:0",
        ])
        .stdin(stdout)
        .status()
        .map_err(|source| VedError::Process {
            program: "ffplay".into(),
            source,
        })?;
    if !status.success() {
        let _ = ffmpeg.kill();
        return Err(VedError::ProcessFailed {
            program: "ffplay".into(),
            code: status
                .code()
                .map(|code| code.to_string())
                .unwrap_or_else(|| "terminated".into()),
            stderr: "see ffplay diagnostics above".into(),
        });
    }
    let ffmpeg_status = ffmpeg.wait().map_err(|source| VedError::Process {
        program: "ffmpeg".into(),
        source,
    })?;
    if !ffmpeg_status.success() {
        return Err(message("ffmpeg preview process failed"));
    }
    Ok(())
}

fn filter_graph(plan: &RenderPlan) -> String {
    let width = plan.canvas.width;
    let height = plan.canvas.height;
    let fps = plan.canvas.fps;
    let mut graph = String::new();
    for (index, segment) in plan.segments.iter().enumerate() {
        graph.push_str(&format!(
            "[{index}:v:0]setpts=PTS-STARTPTS,scale={width}:{height}:force_original_aspect_ratio=decrease,pad={width}:{height}:(ow-iw)/2:(oh-ih)/2:color=black,fps={fps},setsar=1,format=yuv420p[v{index}];"
        ));
        let duration = format_number(segment.source_out - segment.source_in);
        if segment.has_audio {
            graph.push_str(&format!(
                "[{index}:a:0]asetpts=PTS-STARTPTS,aresample=48000,aformat=sample_fmts=fltp:channel_layouts=stereo,volume={},apad=whole_dur={duration},atrim=duration={duration}[a{index}];",
                format_number(segment.volume)
            ));
        } else {
            graph.push_str(&format!(
                "anullsrc=r=48000:cl=stereo:d={duration}[a{index}];"
            ));
        }
    }
    for index in 0..plan.segments.len() {
        graph.push_str(&format!("[v{index}][a{index}]"));
    }
    graph.push_str(&format!(
        "concat=n={}:v=1:a=1[vout][aout]",
        plan.segments.len()
    ));
    graph
}

fn ensure_output_is_not_input(plan: &RenderPlan, output: &Path) -> Result<()> {
    let output = absolute_lexical(output)?;
    for segment in &plan.segments {
        if absolute_lexical(&segment.path)? == output {
            return Err(message(
                "output path must not be one of the input media files",
            ));
        }
    }
    Ok(())
}

fn absolute_lexical(path: &Path) -> Result<std::path::PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(path))
            .map_err(|source| VedError::Io {
                path: path.to_path_buf(),
                source,
            })
    }
}

fn format_number(value: f64) -> String {
    let value = format!("{value:.6}");
    value.trim_end_matches('0').trim_end_matches('.').to_owned()
}

fn strings(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}
