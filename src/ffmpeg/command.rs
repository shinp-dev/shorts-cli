use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use clap::ValueEnum;

use crate::compiler::{RenderPlan, RenderPosition};
use crate::error::{Result, VedError, message};
use crate::project::{CoordinateValue, DimensionValue};

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
    build_args_with_text_dir(plan, output, kind, Path::new("C:/ved-temporary-text"))
}

fn build_args_with_text_dir(
    plan: &RenderPlan,
    output: &Path,
    kind: OutputKind,
    text_dir: &Path,
) -> Vec<OsString> {
    let mut args = vec![
        OsString::from("-hide_banner"),
        OsString::from("-loglevel"),
        OsString::from("warning"),
    ];
    for segment in &plan.video_segments {
        args.extend([
            OsString::from("-ss"),
            format_number(segment.source_in).into(),
            OsString::from("-t"),
            format_number(segment.source_out - segment.source_in).into(),
            OsString::from("-i"),
            segment.path.as_os_str().to_owned(),
        ]);
    }
    for image in &plan.image_overlays {
        args.extend([
            OsString::from("-f"),
            OsString::from("image2"),
            OsString::from("-pattern_type"),
            OsString::from("none"),
            OsString::from("-loop"),
            OsString::from("1"),
            OsString::from("-framerate"),
            plan.canvas.fps.to_string().into(),
            OsString::from("-t"),
            format_number(image.end - image.start).into(),
            OsString::from("-i"),
            image.path.as_os_str().to_owned(),
        ]);
    }
    for audio in &plan.audio_clips {
        args.extend([
            OsString::from("-ss"),
            format_number(audio.source_in).into(),
            OsString::from("-t"),
            format_number(audio.source_out - audio.source_in).into(),
            OsString::from("-i"),
            audio.path.as_os_str().to_owned(),
        ]);
    }

    args.push("-filter_complex".into());
    let mut graph = filter_graph(plan, text_dir);
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
        OutputKind::Frame => args.extend(strings(&["-frames:v", "1", "-update", "1"])),
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
    let prepared = PreparedCommand::new(plan, output, kind)?;
    let output = Command::new("ffmpeg")
        .args(&prepared.args)
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
                .map(|code| code.to_string())
                .unwrap_or_else(|| "terminated".into()),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    Ok(())
}

pub fn run_overwrite(
    plan: &RenderPlan,
    output: &Path,
    kind: OutputKind,
    overwrite: bool,
) -> Result<()> {
    if !overwrite || !output.exists() {
        return run(plan, output, kind);
    }
    ensure_output_is_not_input(plan, output)?;
    let parent = output.parent().unwrap_or_else(|| Path::new("."));
    let stem = output
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("output");
    let extension = output
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("tmp");
    let temp = parent.join(format!(
        ".{stem}.{}.{}.tmp.{extension}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let result = (|| {
        run(plan, &temp, kind)?;
        if !matches!(kind, OutputKind::Frame) {
            let _ = crate::ffmpeg::probe(&temp)?;
        }
        crate::project::replace_existing(&temp, output)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

pub fn play(plan: &RenderPlan) -> Result<()> {
    let prepared = PreparedCommand::new(plan, Path::new("pipe:1"), OutputKind::MatroskaPreview)?;
    let mut ffmpeg = Command::new("ffmpeg")
        .args(&prepared.args)
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
        return Err(process_failed("ffplay", status));
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

struct PreparedCommand {
    args: Vec<OsString>,
    _text_dir: tempfile::TempDir,
}

impl PreparedCommand {
    fn new(plan: &RenderPlan, output: &Path, kind: OutputKind) -> Result<Self> {
        let text_dir = tempfile::tempdir().map_err(|source| VedError::Io {
            path: std::env::temp_dir(),
            source,
        })?;
        for (index, overlay) in plan.text_overlays.iter().enumerate() {
            let path = text_path(text_dir.path(), index);
            let mut file = std::fs::File::create(&path).map_err(|source| VedError::Io {
                path: path.clone(),
                source,
            })?;
            file.write_all(overlay.text.as_bytes())
                .map_err(|source| VedError::Io {
                    path: path.clone(),
                    source,
                })?;
            file.sync_all()
                .map_err(|source| VedError::Io { path, source })?;
            if let Some(font) = &overlay.font {
                let source_path = Path::new(font);
                if source_path.is_file() {
                    let destination = font_path(text_dir.path(), index, source_path);
                    std::fs::copy(source_path, &destination).map_err(|source| VedError::Io {
                        path: destination,
                        source,
                    })?;
                }
            }
        }
        let args = build_args_with_text_dir(plan, output, kind, text_dir.path());
        Ok(Self {
            args,
            _text_dir: text_dir,
        })
    }
}

fn filter_graph(plan: &RenderPlan, text_dir: &Path) -> String {
    let mut graph = String::new();
    let width = plan.canvas.width;
    let height = plan.canvas.height;
    let fps = plan.canvas.fps;
    for (index, segment) in plan.video_segments.iter().enumerate() {
        let timeline_duration = segment
            .hold_duration
            .unwrap_or((segment.source_out - segment.source_in) / segment.speed);
        if segment.hold_duration.is_some() {
            graph.push_str(&format!("[{index}:v:0]setpts=PTS-STARTPTS,scale={width}:{height}:force_original_aspect_ratio=decrease,pad={width}:{height}:(ow-iw)/2:(oh-ih)/2:color=black,fps={fps},tpad=stop_mode=clone:stop_duration={},trim=duration={},setsar=1,format=yuv420p[v{index}];", format_number(timeline_duration), format_number(timeline_duration)));
        } else {
            graph.push_str(&format!("[{index}:v:0]setpts=(PTS-STARTPTS)/{},scale={width}:{height}:force_original_aspect_ratio=decrease,pad={width}:{height}:(ow-iw)/2:(oh-ih)/2:color=black,fps={fps},setsar=1,format=yuv420p[v{index}];", format_number(segment.speed)));
        }
        if segment.has_audio && segment.hold_duration.is_none() {
            graph.push_str(&format!("[{index}:a:0]asetpts=PTS-STARTPTS,aresample=48000,aformat=sample_fmts=fltp:channel_layouts=stereo,volume={},{}apad=whole_dur={},atrim=duration={}[a{index}];",
                format_number(segment.volume), atempo_chain(segment.speed), format_number(timeline_duration), format_number(timeline_duration)));
        } else {
            graph.push_str(&format!(
                "anullsrc=r=48000:cl=stereo:d={}[a{index}];",
                format_number(timeline_duration)
            ));
        }
    }
    for index in 0..plan.video_segments.len() {
        graph.push_str(&format!("[v{index}][a{index}]"));
    }
    graph.push_str(&format!(
        "concat=n={}:v=1:a=1[vbase][abase]",
        plan.video_segments.len()
    ));

    let mut current_video = "vbase".to_owned();
    for (index, text) in plan.text_overlays.iter().enumerate() {
        let next = format!("vtext{index}");
        let path = escape_filter_value(
            &text_path(text_dir, index)
                .to_string_lossy()
                .replace('\\', "/"),
        );
        let mut options = format!(
            "textfile='{path}':expansion=none:fontsize={}:fontcolor={}:alpha={}",
            text.font_size,
            text.color,
            format_number(text.opacity)
        );
        if let Some(font) = &text.font {
            let is_file = Path::new(font).is_file();
            let key = if is_file { "fontfile" } else { "font" };
            let value = if is_file {
                font_path(text_dir, index, Path::new(font))
                    .to_string_lossy()
                    .replace('\\', "/")
            } else {
                font.clone()
            };
            options.push_str(&format!(":{key}='{}'", escape_filter_value(&value)));
        }
        if let Some(outline) = &text.outline {
            options.push_str(&format!(
                ":borderw={}:bordercolor={}",
                outline.width, outline.color
            ));
        }
        if let Some(background) = &text.background {
            options.push_str(&format!(":box=1:boxcolor={background}"));
        }
        options.push_str(&format!(
            ":x={}:y={}:enable='between(t,{},{})'",
            text_x(&text.position),
            text_y(&text.position),
            format_number(text.start),
            format_number(text.end)
        ));
        graph.push_str(&format!(";[{current_video}]drawtext={options}[{next}]"));
        current_video = next;
    }

    let image_input_start = plan.video_segments.len();
    for (index, image) in plan.image_overlays.iter().enumerate() {
        let input = image_input_start + index;
        let image_label = format!("img{index}");
        let next = format!("vimg{index}");
        let (scaled_width, scaled_height) =
            image_dimensions(image.width, image.height, width, height);
        graph.push_str(&format!(";[{input}:v:0]setpts=PTS-STARTPTS+{}/TB,format=rgba,scale={scaled_width}:{scaled_height},colorchannelmixer=aa={}[{image_label}]",
            format_number(image.start), format_number(image.opacity)));
        graph.push_str(&format!(";[{current_video}][{image_label}]overlay=x={}:y={}:enable='between(t,{},{})':format=auto[{next}]",
            image_x(&image.position), image_y(&image.position), format_number(image.start), format_number(image.end)));
        current_video = next;
    }
    graph.push_str(&format!(";[{current_video}]null[vout]"));

    if plan.audio_clips.is_empty() {
        graph.push_str(";[abase]anull[aout]");
    } else {
        let audio_input_start = image_input_start + plan.image_overlays.len();
        for (index, audio) in plan.audio_clips.iter().enumerate() {
            let input = audio_input_start + index;
            let volume = audio_volume_expression(audio);
            let delay_ms = (audio.timeline_start * 1000.0).round() as u64;
            let render_duration = audio.timeline_end - audio.timeline_start;
            let looping = if audio.r#loop {
                "aloop=loop=-1:size=2147483647,".to_owned()
            } else {
                String::new()
            };
            let ducking = audio_ducking_expression(audio)
                .map(|expression| format!(",volume='{expression}':eval=frame"))
                .unwrap_or_default();
            graph.push_str(&format!(";[{input}:a:0]asetpts=PTS-STARTPTS,aresample=48000,aformat=sample_fmts=fltp:channel_layouts=stereo,{}{looping}atrim=start={}:duration={},asetpts=PTS-STARTPTS,volume='{volume}':eval=frame,adelay={delay_ms}:all=1,apad=whole_dur={},atrim=duration={}{}[aext{index}]",
                atempo_chain(audio.speed), format_number(audio.input_trim_offset), format_number(render_duration), format_number(plan.duration), format_number(plan.duration), ducking));
        }
        graph.push_str(";[abase]");
        for index in 0..plan.audio_clips.len() {
            graph.push_str(&format!("[aext{index}]"));
        }
        graph.push_str(&format!(
            "amix=inputs={}:duration=first:dropout_transition=0:normalize=0[aout]",
            plan.audio_clips.len() + 1
        ));
    }
    graph
}

fn atempo_chain(rate: f64) -> String {
    let mut remaining = rate;
    let mut factors = Vec::new();
    while remaining > 2.0 {
        factors.push(2.0);
        remaining /= 2.0;
    }
    while remaining < 0.5 {
        factors.push(0.5);
        remaining /= 0.5;
    }
    if (remaining - 1.0).abs() > 0.000_001 || factors.is_empty() {
        factors.push(remaining);
    }
    factors
        .into_iter()
        .map(|factor| format!("atempo={},", format_number(factor)))
        .collect()
}

fn audio_volume_expression(audio: &crate::compiler::RenderAudioClip) -> String {
    if audio.mute {
        return "0".into();
    }
    let mut expression = format_number(audio.volume);
    if audio.fade_in > 0.0 {
        expression.push_str(&format!(
            "*max(0,min(1,(t+{})/{}))",
            format_number(audio.clip_offset),
            format_number(audio.fade_in)
        ));
    }
    if audio.fade_out > 0.0 {
        expression.push_str(&format!(
            "*max(0,min(1,({}-(t+{}))/{}))",
            format_number(audio.clip_duration),
            format_number(audio.clip_offset),
            format_number(audio.fade_out)
        ));
    }
    format!("if(isnan(t),0,{expression})")
}

fn audio_ducking_expression(audio: &crate::compiler::RenderAudioClip) -> Option<String> {
    let mut expressions = Vec::new();
    for rule in &audio.ducking {
        let gain = 10_f64.powf(-rule.reduction_db / 20.0);
        for interval in &rule.intervals {
            let start = interval.start;
            let end = interval.end;
            let attack_start = (start - rule.attack).max(0.0);
            let release_end = end + rule.release;
            let mut expression = format_number(1.0);
            if rule.release > 0.0 {
                expression = format!(
                    "if(between(t,{},{})\\,{}+(1-{})*(t-{})/{}\\,{expression})",
                    format_number(end),
                    format_number(release_end),
                    format_number(gain),
                    format_number(gain),
                    format_number(end),
                    format_number(rule.release)
                );
            }
            expression = format!(
                "if(between(t,{},{})\\,{}\\,{expression})",
                format_number(start),
                format_number(end),
                format_number(gain)
            );
            if rule.attack > 0.0 && start > 0.0 {
                expression = format!(
                    "if(between(t,{},{})\\,1-(1-{})*(t-{})/{}\\,{expression})",
                    format_number(attack_start),
                    format_number(start),
                    format_number(gain),
                    format_number(attack_start),
                    format_number(start - attack_start)
                );
            }
            expressions.push(expression);
        }
    }
    let mut values = expressions.into_iter();
    let first = values.next()?;
    Some(values.fold(first, |left, right| format!("min({left}\\,{right})")))
}

fn text_x(position: &RenderPosition) -> String {
    coordinate_expression(position.x, "w", "text_w")
}
fn text_y(position: &RenderPosition) -> String {
    coordinate_expression(position.y, "h", "text_h")
}
fn image_x(position: &RenderPosition) -> String {
    coordinate_expression(position.x, "main_w", "overlay_w")
}
fn image_y(position: &RenderPosition) -> String {
    coordinate_expression(position.y, "main_h", "overlay_h")
}

fn coordinate_expression(value: CoordinateValue, canvas: &str, overlay: &str) -> String {
    match value {
        CoordinateValue::Start => format!("{canvas}*0.05"),
        CoordinateValue::Center => format!("({canvas}-{overlay})/2"),
        CoordinateValue::End => format!("{canvas}-{overlay}-{canvas}*0.05"),
        CoordinateValue::Percent(value) => format!("{canvas}*{}-{overlay}/2", format_number(value)),
    }
}

fn image_dimensions(
    width: Option<DimensionValue>,
    height: Option<DimensionValue>,
    canvas_width: u32,
    canvas_height: u32,
) -> (String, String) {
    let width = width.map(|value| dimension_pixels(value, canvas_width));
    let height = height.map(|value| dimension_pixels(value, canvas_height));
    match (width, height) {
        (Some(width), Some(height)) => (width, height),
        (Some(width), None) => (width, "-1".into()),
        (None, Some(height)) => ("-1".into(), height),
        (None, None) => ("iw".into(), "ih".into()),
    }
}

fn dimension_pixels(value: DimensionValue, canvas: u32) -> String {
    let value = match value {
        DimensionValue::Pixels(value) => value,
        DimensionValue::Percent(value) => value * f64::from(canvas),
    };
    format_number(value.max(1.0).round())
}

fn escape_filter_value(value: &str) -> String {
    let mut output = String::new();
    for character in value.chars() {
        if character == '\'' {
            output.push_str("'\\''");
            continue;
        }
        if matches!(character, '\\' | ':' | ';' | ',' | '[' | ']') {
            output.push('\\');
        }
        output.push(character);
    }
    output
}

fn text_path(directory: &Path, index: usize) -> PathBuf {
    directory.join(format!("text-{index:04}.txt"))
}

fn font_path(directory: &Path, index: usize, source: &Path) -> PathBuf {
    let extension = source
        .extension()
        .and_then(|value| value.to_str())
        .filter(|value| matches!(value.to_ascii_lowercase().as_str(), "ttf" | "ttc" | "otf"))
        .unwrap_or("ttf");
    directory.join(format!("font-{index:04}.{extension}"))
}

fn ensure_output_is_not_input(plan: &RenderPlan, output: &Path) -> Result<()> {
    let output = absolute_lexical(output)?;
    let paths = plan
        .video_segments
        .iter()
        .map(|item| &item.path)
        .chain(plan.image_overlays.iter().map(|item| &item.path))
        .chain(plan.audio_clips.iter().map(|item| &item.path));
    for path in paths {
        if absolute_lexical(path)? == output {
            return Err(message(
                "output path must not be one of the input media files",
            ));
        }
    }
    Ok(())
}

fn absolute_lexical(path: &Path) -> Result<PathBuf> {
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

fn process_failed(program: &str, status: std::process::ExitStatus) -> VedError {
    VedError::ProcessFailed {
        program: program.into(),
        code: status
            .code()
            .map(|code| code.to_string())
            .unwrap_or_else(|| "terminated".into()),
        stderr: format!("see {program} diagnostics above"),
    }
}

fn format_number(value: f64) -> String {
    let value = format!("{value:.6}");
    value.trim_end_matches('0').trim_end_matches('.').to_owned()
}
fn strings(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn atempo_is_split_into_supported_factors() {
        assert_eq!(atempo_chain(4.0), "atempo=2,atempo=2,");
        assert_eq!(atempo_chain(0.25), "atempo=0.5,atempo=0.5,");
        assert_eq!(
            atempo_chain(20.0),
            "atempo=2,atempo=2,atempo=2,atempo=2,atempo=1.25,"
        );
        assert_eq!(
            atempo_chain(0.05),
            "atempo=0.5,atempo=0.5,atempo=0.5,atempo=0.5,atempo=0.8,"
        );
    }
    #[test]
    fn filter_values_escape_every_graph_delimiter() {
        assert_eq!(
            escape_filter_value("'\":;,\\[] 日本語 50%"),
            "'\\''\"\\:\\;\\,\\\\\\[\\] 日本語 50%"
        );
    }
}
