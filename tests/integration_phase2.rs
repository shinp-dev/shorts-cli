use std::path::Path;
use std::process::{Command, Output};

fn run(program: &str, directory: &Path, args: &[&str]) -> Output {
    let output = Command::new(program)
        .current_dir(directory)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "command failed: {program} {args:?}\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn ved(directory: &Path, args: &[&str]) -> Output {
    run(env!("CARGO_BIN_EXE_ved"), directory, args)
}

#[test]
fn phase_two_pipeline_uses_one_final_timeline() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let video_a = "video A [50%],;'.mp4";
    let video_b = "video B.mp4";
    let overlay = "透過 overlay [50%].png";
    let bgm = "BGM 音.wav";

    run(
        "ffmpeg",
        root,
        &[
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=320x180:rate=30",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000",
            "-t",
            "3",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-shortest",
            video_a,
        ],
    );
    run(
        "ffmpeg",
        root,
        &[
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=c=blue:size=240x320:rate=24",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=660:sample_rate=48000",
            "-t",
            "1",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-shortest",
            video_b,
        ],
    );
    run(
        "ffmpeg",
        root,
        &[
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=c=red@0.5:size=64x64:rate=1,format=rgba",
            "-frames:v",
            "1",
            "-threads",
            "1",
            overlay,
        ],
    );
    run(
        "ffmpeg",
        root,
        &[
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=220:sample_rate=48000",
            "-t",
            "3",
            "-c:a",
            "pcm_s16le",
            bgm,
        ],
    );

    ved(root, &["new", "demo.json", "--preset", "square"]);
    for media in [video_a, video_b, overlay, bgm] {
        ved(root, &["import", "demo.json", media]);
    }
    ved(
        root,
        &["add", "demo.json", video_a, "--in", "0", "--out", "3"],
    );
    ved(
        root,
        &["speed", "demo.json", "--clip", "c1", "--rate", "1.5"],
    );
    ved(
        root,
        &[
            "insert",
            "demo.json",
            video_b,
            "--at",
            "1",
            "--in",
            "0",
            "--out",
            "0.75",
        ],
    );

    let text = "'\" : ; , \\ [brackets] 50% 日本語 空白\n二行目";
    let mut text_args = vec![
        "text-add",
        "demo.json",
        "--text",
        text,
        "--from",
        "0.5",
        "--to",
        "2.2",
        "--position",
        "bottom-center",
        "--font-size",
        "52",
        "--outline-color",
        "black",
    ];
    let windows_font = Path::new(r"C:\Windows\Fonts\meiryo.ttc");
    let special_font = root.join("font ' ; , [50%] 日本語 空白.ttc");
    if windows_font.is_file() {
        std::fs::copy(windows_font, &special_font).unwrap();
        text_args.extend(["--font", special_font.to_str().unwrap()]);
    }
    ved(root, &text_args);
    ved(root, &["text-set", "demo.json", "t1", "--opacity", "0.9"]);
    ved(
        root,
        &[
            "image-add",
            "demo.json",
            overlay,
            "--from",
            "0.9",
            "--to",
            "2.1",
            "--x",
            "85%",
            "--y",
            "10%",
            "--width",
            "15%",
            "--opacity",
            "1",
        ],
    );
    ved(root, &["image-set", "demo.json", "i1", "--opacity", "0.8"]);
    ved(
        root,
        &[
            "audio-add",
            "demo.json",
            bgm,
            "--at",
            "0",
            "--in",
            "0",
            "--out",
            "2.75",
            "--volume",
            "0.2",
            "--fade-in",
            "0.4",
            "--fade-out",
            "0.4",
        ],
    );
    ved(
        root,
        &["volume", "demo.json", "--audio", "a1", "--value", "0.25"],
    );
    ved(
        root,
        &["volume", "demo.json", "--clip", "c1", "--value", "0.7"],
    );
    ved(root, &["mute", "demo.json", "--audio", "a1"]);
    ved(root, &["mute", "demo.json", "--audio", "a1", "--off"]);
    ved(root, &["mute", "demo.json", "--clip", "c2"]);
    ved(root, &["undo", "demo.json"]);
    let undone: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("demo.json")).unwrap()).unwrap();
    assert!(undone["timeline"][1].get("mute").is_none());
    ved(root, &["redo", "demo.json"]);
    ved(
        root,
        &["fade-in", "demo.json", "--audio", "a1", "--duration", "0.5"],
    );
    ved(
        root,
        &[
            "fade-out",
            "demo.json",
            "--audio",
            "a1",
            "--duration",
            "0.5",
        ],
    );
    ved(
        root,
        &[
            "text-add",
            "demo.json",
            "--text",
            "temporary",
            "--from",
            "2",
            "--to",
            "2.5",
        ],
    );
    ved(root, &["text-remove", "demo.json", "t2"]);
    ved(
        root,
        &[
            "image-add",
            "demo.json",
            overlay,
            "--from",
            "2",
            "--to",
            "2.5",
        ],
    );
    ved(root, &["image-remove", "demo.json", "i2"]);
    ved(
        root,
        &[
            "audio-add",
            "demo.json",
            bgm,
            "--at",
            "2",
            "--in",
            "0",
            "--out",
            "0.5",
        ],
    );
    ved(root, &["audio-remove", "demo.json", "a2"]);

    let project: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("demo.json")).unwrap()).unwrap();
    assert_eq!(project["version"], 3);
    assert_eq!(project["text_overlays"][0]["start"], 0.5);
    assert_eq!(project["image_overlays"][0]["start"], 0.9);
    assert_eq!(project["audio_clips"][0]["start"], 0.0);
    assert_eq!(project["timeline"][0]["volume"], 0.7);
    assert_eq!(project["text_overlays"][0]["opacity"], 0.9);
    assert_eq!(project["image_overlays"][0]["opacity"], 0.8);
    assert_eq!(project["audio_clips"][0]["volume"], 0.25);

    let dry_run = ved(
        root,
        &[
            "play",
            "demo.json",
            "--from",
            "0.8",
            "--to",
            "1.8",
            "--dry-run",
        ],
    );
    let dry_run: serde_json::Value = serde_json::from_slice(&dry_run.stdout).unwrap();
    let plan = &dry_run["plan"];
    assert_eq!(plan["duration"], 1.0);
    assert!((plan["video_segments"][0]["source_in"].as_f64().unwrap() - 1.2).abs() < 1e-9);
    assert_eq!(plan["video_segments"][1]["clip_id"], "c2");
    assert_eq!(plan["video_segments"][2]["source_in"], 1.5);
    assert!((plan["text_overlays"][0]["start"].as_f64().unwrap() - 0.0).abs() < 1e-9);
    assert!((plan["image_overlays"][0]["start"].as_f64().unwrap() - 0.1).abs() < 1e-9);
    assert!((plan["audio_clips"][0]["source_in"].as_f64().unwrap() - 0.8).abs() < 1e-9);

    ved(root, &["frame", "demo.json", "1.2", "frame.png"]);
    ved(
        root,
        &[
            "export-range",
            "demo.json",
            "--from",
            "0.8",
            "--to",
            "1.8",
            "--out",
            "range.mp4",
        ],
    );
    ved(
        root,
        &["render", "demo.json", "final.mp4", "--quality", "preview"],
    );

    let frame = run(
        "ffmpeg",
        root,
        &[
            "-hide_banner",
            "-loglevel",
            "error",
            "-i",
            "frame.png",
            "-vf",
            "format=rgb24",
            "-frames:v",
            "1",
            "-f",
            "rawvideo",
            "pipe:1",
        ],
    );
    let white_pixels = frame
        .stdout
        .chunks_exact(3)
        .filter(|pixel| pixel[0] > 180 && pixel[1] > 180 && pixel[2] > 180)
        .count();
    assert!(white_pixels > 100, "Japanese text was not visibly rendered");
    let center = run(
        "ffmpeg",
        root,
        &[
            "-hide_banner",
            "-loglevel",
            "error",
            "-i",
            "frame.png",
            "-vf",
            "crop=1:1:918:108,format=rgb24",
            "-frames:v",
            "1",
            "-f",
            "rawvideo",
            "pipe:1",
        ],
    );
    assert!(
        center.stdout[0] > 40 && center.stdout[2] > 40,
        "transparent PNG did not blend"
    );

    let range_probe = run(
        "ffprobe",
        root,
        &[
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=nw=1:nk=1",
            "range.mp4",
        ],
    );
    let range_duration: f64 = String::from_utf8_lossy(&range_probe.stdout)
        .trim()
        .parse()
        .unwrap();
    assert!((range_duration - 1.0).abs() < 0.05);
    let final_probe = run(
        "ffprobe",
        root,
        &[
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-show_entries",
            "stream=codec_type,width,height,r_frame_rate",
            "-of",
            "json",
            "final.mp4",
        ],
    );
    let final_probe: serde_json::Value = serde_json::from_slice(&final_probe.stdout).unwrap();
    assert!(
        (final_probe["format"]["duration"]
            .as_str()
            .unwrap()
            .parse::<f64>()
            .unwrap()
            - 2.75)
            .abs()
            < 0.05
    );
    assert_eq!(final_probe["streams"][0]["width"], 1080);
    assert_eq!(final_probe["streams"][0]["height"], 1080);
    assert!(
        final_probe["streams"]
            .as_array()
            .unwrap()
            .iter()
            .any(|stream| stream["codec_type"] == "audio")
    );
}

#[test]
fn phase_one_edit_and_render_flow_regresses_cleanly() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    run(
        "ffmpeg",
        root,
        &[
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=160x90:rate=30",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000",
            "-t",
            "1.5",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-shortest",
            "a.mp4",
        ],
    );
    run(
        "ffmpeg",
        root,
        &[
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=c=green:size=90x160:rate=24",
            "-t",
            "0.5",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "b.mp4",
        ],
    );
    ved(root, &["new", "phase1.json", "--preset", "square"]);
    ved(root, &["import", "phase1.json", "a.mp4"]);
    ved(root, &["import", "phase1.json", "b.mp4"]);
    ved(
        root,
        &["add", "phase1.json", "a.mp4", "--in", "0", "--out", "1.5"],
    );
    ved(
        root,
        &["trim", "phase1.json", "--clip", "c1", "--out", "1.2"],
    );
    ved(
        root,
        &[
            "insert",
            "phase1.json",
            "b.mp4",
            "--at",
            "0.4",
            "--in",
            "0",
            "--out",
            "0.4",
        ],
    );
    ved(
        root,
        &["remove", "phase1.json", "--from", "0.1", "--to", "0.2"],
    );
    let description = ved(root, &["describe", "phase1.json"]);
    assert!(String::from_utf8_lossy(&description.stdout).contains("Duration: 1.500s"));
    ved(
        root,
        &[
            "render",
            "phase1.json",
            "phase1.mp4",
            "--quality",
            "preview",
        ],
    );
    let probe = run(
        "ffprobe",
        root,
        &[
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=nw=1:nk=1",
            "phase1.mp4",
        ],
    );
    let duration: f64 = String::from_utf8_lossy(&probe.stdout)
        .trim()
        .parse()
        .unwrap();
    assert!((duration - 1.5).abs() < 0.05);
}
