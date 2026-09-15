use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn ved(directory: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ved"))
        .current_dir(directory)
        .args(args)
        .output()
        .unwrap()
}

fn success_json(directory: &Path, args: &[&str]) -> (Output, serde_json::Value) {
    let output = ved(directory, args);
    assert!(
        output.status.success(),
        "command failed: {args:?}\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout.clone()).unwrap();
    assert_eq!(stdout.lines().count(), 1, "expected compact JSON");
    let json = serde_json::from_str(stdout.trim()).unwrap();
    (output, json)
}

#[test]
fn executor_bootstrap_and_work_order_are_bounded_and_read_only() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    success_json(
        root,
        &[
            "--json",
            "new",
            "edit.json",
            "--width",
            "320",
            "--height",
            "480",
        ],
    );

    let (capabilities_output, capabilities) = success_json(root, &["--json", "capabilities"]);
    assert!(capabilities_output.stdout.len() < 4 * 1024);
    assert_eq!(capabilities["network_default"], "deny");
    assert_eq!(capabilities["artifact_versions"]["work_order"], 1);
    assert!(
        capabilities["features"]
            .as_array()
            .unwrap()
            .iter()
            .any(|feature| feature == "compact_inspect")
    );

    let (_, schema) = success_json(root, &["capabilities", "--schema", "work_order", "--json"]);
    assert_eq!(schema["schema"], "work_order");
    assert_eq!(schema["json_schema"]["additionalProperties"], false);

    let before = fs::read(root.join("edit.json")).unwrap();
    let (inspection_output, inspection) = success_json(root, &["inspect", "edit.json", "--json"]);
    assert!(inspection_output.stdout.len() < 16 * 1024);
    let project_hash = inspection["project_hash"].as_str().unwrap();
    assert!(project_hash.starts_with("sha256:"));
    assert_eq!(project_hash.len(), 71);
    assert_eq!(inspection["counts"]["timeline"], 0);

    let brief = serde_json::json!({
        "version": 1,
        "project_hash": project_hash,
        "objective": "音声完成版を自然なテンポで仕上げる",
        "authorized": ["read_only", "derived_artifact", "project_mutation"],
        "changes": [
            { "target": "timeline", "instruction": "結果表示の間を確認する" }
        ],
        "constraints": ["台詞とクリップ順は変更しない"],
        "review_focus": ["最後の間"]
    });
    fs::write(
        root.join("brief.json"),
        serde_json::to_vec_pretty(&brief).unwrap(),
    )
    .unwrap();
    let (_, checked) = success_json(root, &["--json", "brief-check", "edit.json", "brief.json"]);
    assert_eq!(checked["valid"], true);
    assert_eq!(checked["changes"], 1);
    assert_eq!(fs::read(root.join("edit.json")).unwrap(), before);

    let (_, report) = success_json(
        root,
        &["--json", "check", "edit.json", "--profile", "shorts"],
    );
    assert_eq!(report["counts"]["fail"], 1);
    assert_eq!(report["issues"][0]["code"], "timeline_empty");
    assert_eq!(fs::read(root.join("edit.json")).unwrap(), before);

    let (_, analysis_only) = success_json(
        root,
        &["--json", "inspect", "edit.json", "--section", "analysis"],
    );
    assert!(analysis_only.get("analysis").is_some());
    assert!(analysis_only.get("timeline").is_none());
}

#[test]
fn stale_or_executable_work_orders_fail_with_stable_codes() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    success_json(root, &["--json", "new", "edit.json"]);
    let before = fs::read(root.join("edit.json")).unwrap();

    let stale = serde_json::json!({
        "version": 1,
        "project_hash": format!("sha256:{}", "0".repeat(64)),
        "objective": "仕上げる",
        "authorized": ["read_only"],
        "changes": []
    });
    fs::write(root.join("brief.json"), serde_json::to_vec(&stale).unwrap()).unwrap();
    let output = ved(root, &["--json", "brief-check", "edit.json", "brief.json"]);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let error: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(error["code"], "brief_stale");

    let (_, inspection) = success_json(root, &["--json", "inspect", "edit.json"]);
    let executable = serde_json::json!({
        "version": 1,
        "project_hash": inspection["project_hash"],
        "objective": "仕上げる",
        "authorized": ["project_mutation"],
        "changes": [
            { "target": "timeline", "instruction": "ffmpeg -i input.mp4 output.mp4" }
        ]
    });
    fs::write(
        root.join("brief.json"),
        serde_json::to_vec(&executable).unwrap(),
    )
    .unwrap();
    let output = ved(root, &["--json", "brief-check", "edit.json", "brief.json"]);
    assert!(!output.status.success());
    let error: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(error["code"], "brief_payload_rejected");
    assert_eq!(fs::read(root.join("edit.json")).unwrap(), before);
}

#[test]
fn review_bundle_is_atomic_bounded_and_reusable() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let generated = Command::new("ffmpeg")
        .current_dir(root)
        .args([
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
            "1",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-shortest",
            "source.mp4",
        ])
        .output()
        .unwrap();
    assert!(generated.status.success());
    success_json(
        root,
        &[
            "--json",
            "new",
            "edit.json",
            "--width",
            "320",
            "--height",
            "480",
        ],
    );
    success_json(root, &["--json", "import", "edit.json", "source.mp4"]);
    success_json(root, &["--json", "add", "edit.json", "m1"]);
    success_json(
        root,
        &[
            "--json",
            "audio-add",
            "edit.json",
            "m1",
            "--at",
            "0",
            "--track",
            "narration",
            "--key",
            "voice",
        ],
    );
    let notes = serde_json::json!({
        "version": 1,
        "summary": "音声完成版の初回仕上げ。",
        "changes": [{ "time": 0.4, "text": "全体のテンポを確認" }],
        "decisions": [{
            "time": 0.8,
            "question": "最後の間を残すか",
            "options": ["現状維持", "少し短縮"]
        }]
    });
    fs::write(
        root.join("notes.json"),
        serde_json::to_vec_pretty(&notes).unwrap(),
    )
    .unwrap();
    let before = fs::read(root.join("edit.json")).unwrap();

    let (_, audio_loudness) =
        success_json(root, &["--json", "loudness", "edit.json", "--audio", "a1"]);
    assert_eq!(audio_loudness["cached"], false);
    assert!(audio_loudness["integrated_lufs"].is_number());
    assert!(audio_loudness["true_peak_dbtp"].is_number());
    let (_, cached_loudness) =
        success_json(root, &["--json", "loudness", "edit.json", "--audio", "a1"]);
    assert_eq!(cached_loudness["cached"], true);
    assert_eq!(cached_loudness["cache_key"], audio_loudness["cache_key"]);
    let (_, track_loudness) = success_json(
        root,
        &["--json", "loudness", "edit.json", "--track", "narration"],
    );
    assert_eq!(track_loudness["source"], "track:narration");
    let (_, mix_loudness) = success_json(
        root,
        &[
            "--json",
            "loudness",
            "edit.json",
            "--mix",
            "--from",
            "0.1",
            "--to",
            "0.9",
        ],
    );
    assert_eq!(mix_loudness["measured_duration"], 0.8);
    let (_, waveform) = success_json(
        root,
        &[
            "--json",
            "waveform",
            "edit.json",
            "--track",
            "narration",
            "--out",
            "narration.png",
            "--width",
            "640",
            "--peaks-out",
            "narration-peaks.json",
        ],
    );
    assert_eq!(waveform["width"], 640);
    assert_eq!(waveform["height"], 320);
    assert!(root.join("narration.png").is_file());
    let peaks: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("narration-peaks.json")).unwrap()).unwrap();
    assert!(peaks["peaks"].as_array().unwrap().len() <= 400);
    assert!(peaks["peaks"].as_array().unwrap().len() > 10);
    let (_, markers) = success_json(
        root,
        &[
            "--json",
            "markers",
            "edit.json",
            "--include",
            "timeline",
            "--include",
            "audio",
            "--out",
            "markers-export.json",
        ],
    );
    assert_eq!(markers["format"], "json");
    assert_eq!(markers["markers"], 2);
    let exported_markers: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("markers-export.json")).unwrap()).unwrap();
    assert!(
        exported_markers["markers"][0]["id"]
            .as_str()
            .unwrap()
            .starts_with("mk-")
    );
    success_json(
        root,
        &[
            "--json",
            "markers",
            "edit.json",
            "--format",
            "vtt",
            "--out",
            "markers.vtt",
        ],
    );
    assert!(
        fs::read_to_string(root.join("markers.vtt"))
            .unwrap()
            .starts_with("WEBVTT\n")
    );
    assert_eq!(fs::read(root.join("edit.json")).unwrap(), before);

    let (first_output, first) = success_json(
        root,
        &[
            "--json",
            "review-build",
            "edit.json",
            "--out",
            "reviews",
            "--notes",
            "notes.json",
        ],
    );
    assert!(first_output.stdout.len() < 1024);
    assert_eq!(first["changed"], true);
    assert_eq!(first["checks"]["fail"], 0);
    assert_eq!(first["decisions"], 1);
    for field in ["video", "report", "storyboard"] {
        assert!(Path::new(first[field].as_str().unwrap()).is_file());
    }
    let report = fs::read(first["report"].as_str().unwrap()).unwrap();
    assert!(report.len() <= 2048);
    let review_dir = Path::new(first["report"].as_str().unwrap())
        .parent()
        .unwrap();
    assert!(review_dir.join("markers.json").is_file());
    assert!(review_dir.join("review.json").is_file());
    assert_eq!(fs::read(root.join("edit.json")).unwrap(), before);

    let (_, second) = success_json(
        root,
        &[
            "review-build",
            "edit.json",
            "--out",
            "reviews",
            "--notes",
            "notes.json",
            "--json",
        ],
    );
    assert_eq!(second["changed"], false);
    assert_eq!(second["review_id"], first["review_id"]);
    assert_eq!(second["video"], first["video"]);
}
