use std::fs;
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

fn json_stdout(output: &Output) -> serde_json::Value {
    let stdout = String::from_utf8(output.stdout.clone()).unwrap();
    assert_eq!(
        stdout.lines().count(),
        1,
        "expected one JSON value: {stdout:?}"
    );
    serde_json::from_str(stdout.trim()).unwrap()
}

fn file_count(path: &Path) -> usize {
    if !path.exists() {
        return 0;
    }
    fs::read_dir(path)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .map(|path| if path.is_dir() { file_count(&path) } else { 1 })
        .sum()
}

#[test]
fn phase_three_agent_audio_workflow_is_idempotent_and_renders() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let video = "素材 (3) 50%.mp4";
    let voice = "声 (つられない).wav";

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
            "sine=frequency=330:sample_rate=48000",
            "-t",
            "2",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-shortest",
            video,
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
            "sine=frequency=880:sample_rate=48000",
            "-t",
            "0.4",
            voice,
        ],
    );

    let created = json_stdout(&ved(
        root,
        &[
            "--json",
            "new",
            "edit.json",
            "--width",
            "320",
            "--height",
            "400",
        ],
    ));
    assert_eq!(created["ok"], true);
    ved(root, &["--quiet", "import", "edit.json", video]);
    ved(root, &["--quiet", "add", "edit.json", "m1", "--mute"]);

    let held = json_stdout(&ved(
        root,
        &[
            "hold",
            "edit.json",
            "--clip",
            "c1",
            "--source",
            "1",
            "--duration",
            "1",
            "--json",
        ],
    ));
    assert_eq!(held["duration"], 3.0);

    let manifest = serde_json::json!({
        "version": 1,
        "defaults": { "fade_in": 0.01, "fade_out": 0.01 },
        "clips": [
            {
                "key": "music-bed",
                "track": "bgm",
                "file": voice,
                "at": 0,
                "to": "timeline",
                "loop": true,
                "volume": 0.15
            },
            {
                "key": "hook",
                "track": "narration",
                "file": voice,
                "at": { "clip": "c1", "source": 0.25 }
            },
            {
                "key": "result",
                "track": "sfx",
                "file": voice,
                "at": { "clip": "c2", "source": 1.5 },
                "speed": 0.8
            }
        ],
        "ducking": [{
            "key": "voice-over-music",
            "target_track": "bgm",
            "trigger_tracks": ["narration"],
            "reduction_db": 12,
            "attack": 0.1,
            "release": 0.2
        }]
    });
    fs::write(
        root.join("mix 日本語.json"),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();

    let first = json_stdout(&ved(
        root,
        &["audio-sync", "edit.json", "mix 日本語.json", "--json"],
    ));
    assert_eq!(first["changed"], true);
    assert_eq!(first["audio_created"], 3);
    assert_eq!(first["ducking_created"], 1);
    let project_after_first = fs::read(root.join("edit.json")).unwrap();
    let history_after_first = file_count(&root.join(".ved").join("history"));

    let second = json_stdout(&ved(
        root,
        &["--json", "audio-sync", "edit.json", "mix 日本語.json"],
    ));
    assert_eq!(second["changed"], false);
    assert_eq!(
        fs::read(root.join("edit.json")).unwrap(),
        project_after_first
    );
    assert_eq!(
        file_count(&root.join(".ved").join("history")),
        history_after_first,
        "an idempotent sync must not create history"
    );

    let project: serde_json::Value = serde_json::from_slice(&project_after_first).unwrap();
    assert_eq!(project["version"], 3);
    assert_eq!(project["timeline"][1]["id"], "h1");
    assert_eq!(project["timeline"][2]["id"], "c2");
    assert_eq!(project["audio_clips"][0]["end"], 3.0);
    assert_eq!(project["audio_clips"][1]["start"], 0.25);
    assert_eq!(project["audio_clips"][2]["start"], 2.5);

    let before_invalid = fs::read(root.join("edit.json")).unwrap();
    let before_invalid_history = file_count(&root.join(".ved").join("history"));
    let invalid_clips = (0..20)
        .map(|index| {
            serde_json::json!({
                "key": if index == 19 { "bad key with spaces" } else { "valid-key" },
                "track": format!("track-{index}"),
                "file": voice,
                "at": 0
            })
        })
        .collect::<Vec<_>>();
    fs::write(
        root.join("invalid.json"),
        serde_json::to_vec(&serde_json::json!({
            "version": 1,
            "clips": invalid_clips
        }))
        .unwrap(),
    )
    .unwrap();
    let invalid = Command::new(env!("CARGO_BIN_EXE_ved"))
        .current_dir(root)
        .args(["--json", "audio-sync", "edit.json", "invalid.json"])
        .output()
        .unwrap();
    assert!(!invalid.status.success());
    assert!(invalid.stdout.is_empty());
    let error: serde_json::Value =
        serde_json::from_slice(invalid.stderr.strip_suffix(b"\n").unwrap()).unwrap();
    assert_eq!(error["ok"], false);
    assert_eq!(fs::read(root.join("edit.json")).unwrap(), before_invalid);
    assert_eq!(
        file_count(&root.join(".ved").join("history")),
        before_invalid_history
    );

    let failed_output = root.join("preserved.invalid");
    fs::write(&failed_output, b"old output stays intact").unwrap();
    let failed_render = Command::new(env!("CARGO_BIN_EXE_ved"))
        .current_dir(root)
        .args([
            "render",
            "edit.json",
            "preserved.invalid",
            "--overwrite",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(!failed_render.status.success());
    assert!(failed_render.stdout.is_empty());
    let render_error_text = String::from_utf8(failed_render.stderr).unwrap();
    assert_eq!(render_error_text.lines().count(), 1);
    let render_error: serde_json::Value = serde_json::from_str(render_error_text.trim()).unwrap();
    assert_eq!(render_error["ok"], false);
    assert_eq!(
        fs::read(&failed_output).unwrap(),
        b"old output stays intact"
    );

    fs::write(root.join("preview.mp4"), b"previous output").unwrap();
    let rendered = json_stdout(&ved(
        root,
        &[
            "render",
            "edit.json",
            "preview.mp4",
            "--quality",
            "preview",
            "--overwrite",
            "--json",
        ],
    ));
    assert_eq!(rendered["duration"], 3.0);
    let probe = run(
        "ffprobe",
        root,
        &[
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=nokey=1:noprint_wrappers=1",
            "preview.mp4",
        ],
    );
    let duration: f64 = String::from_utf8(probe.stdout)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert!(
        (duration - 3.0).abs() < 0.05,
        "rendered duration was {duration}"
    );
}
