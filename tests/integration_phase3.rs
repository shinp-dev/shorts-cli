use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Command, Output};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

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

fn ved_fail(directory: &Path, args: &[&str]) -> Output {
    let output = Command::new(env!("CARGO_BIN_EXE_ved"))
        .current_dir(directory)
        .args(args)
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "command unexpectedly succeeded: {args:?}\nstdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    output
}

#[test]
fn phase_three_tts_uses_audio_clip_cache_history_and_render_plan() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let mock = MockEngine::start(false);

    let listed = ved(
        root,
        &[
            "voice",
            "list",
            "--provider",
            "voicevox",
            "--endpoint",
            &mock.endpoint,
            "--json",
        ],
    );
    let voices: serde_json::Value = serde_json::from_slice(&listed.stdout).unwrap();
    assert_eq!(voices[0]["provider"], "voicevox");
    assert_eq!(voices[0]["voice_id"], "1");
    assert_eq!(voices[0]["display_name"], "Mock Speaker");
    assert_eq!(voices[0]["style_name"], "Normal");

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
            "-t",
            "2",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "video.mp4",
        ],
    );
    ved(root, &["new", "demo.json", "--preset", "square"]);
    ved(root, &["import", "demo.json", "video.mp4"]);
    ved(
        root,
        &[
            "add",
            "demo.json",
            "video.mp4",
            "--in",
            "0",
            "--out",
            "2",
        ],
    );

    let special_text = "\" ' & | > < % \\ 日本語\n空白 でもちゃんと先を読む。";
    ved(
        root,
        &[
            "voice-add",
            "demo.json",
            "--provider",
            "voicevox",
            "--voice",
            "1",
            "--text",
            special_text,
            "--at",
            "0.25",
            "--speed",
            "1",
            "--pitch",
            "0",
            "--volume",
            "0.8",
            "--endpoint",
            &mock.endpoint,
        ],
    );
    assert_eq!(mock.synthesis_count(), 1);

    let project: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("demo.json")).unwrap()).unwrap();
    assert_eq!(project["version"], 3);
    assert_eq!(project["voice_clips"][0]["id"], "v1");
    assert_eq!(project["voice_clips"][0]["provider"], "voicevox");
    assert_eq!(project["voice_clips"][0]["audio_clip_id"], "a1");
    assert_eq!(project["audio_clips"][0]["start"], 0.25);
    assert_eq!(project["audio_clips"][0]["volume"], 0.8);
    assert!(project["voice_clips"][0]["cache_key"].as_str().unwrap().len() == 64);
    let media_path = project["media"]
        .as_array()
        .unwrap()
        .iter()
        .find(|media| media["id"] == project["audio_clips"][0]["media_id"])
        .unwrap()["path"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(media_path.starts_with(".ved/cache/tts/"));
    assert!(root.join(&media_path).is_file());

    let play = ved(
        root,
        &["play", "demo.json", "--from", "0", "--to", "1", "--dry-run"],
    );
    let play: serde_json::Value = serde_json::from_slice(&play.stdout).unwrap();
    assert_eq!(play["plan"]["audio_clips"].as_array().unwrap().len(), 1);
    assert_eq!(play["plan"]["audio_clips"][0]["id"], "a1");

    ved(
        root,
        &[
            "voice-set",
            "demo.json",
            "v1",
            "--at",
            "0.5",
            "--volume",
            "0.6",
        ],
    );
    assert_eq!(mock.synthesis_count(), 1, "at/volume must not synthesize");

    ved(
        root,
        &[
            "voice-add",
            "demo.json",
            "--provider",
            "voicevox",
            "--voice",
            "1",
            "--text",
            special_text,
            "--at",
            "1.0",
            "--speed",
            "1",
            "--pitch",
            "0",
            "--endpoint",
            &mock.endpoint,
        ],
    );
    assert_eq!(mock.synthesis_count(), 1, "identical synthesis must hit cache");
    ved(root, &["voice-remove", "demo.json", "v2"]);

    ved(
        root,
        &[
            "voice-set",
            "demo.json",
            "v1",
            "--text",
            "弱いAIを作ってみました。",
        ],
    );
    assert_eq!(mock.synthesis_count(), 2, "text change must miss cache");
    let after_text: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("demo.json")).unwrap()).unwrap();
    let changed_key = after_text["voice_clips"][0]["cache_key"]
        .as_str()
        .unwrap()
        .to_owned();

    ved(root, &["undo", "demo.json"]);
    assert_eq!(mock.synthesis_count(), 2, "undo must not synthesize");
    let undone: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("demo.json")).unwrap()).unwrap();
    assert_eq!(undone["voice_clips"][0]["text"], special_text);
    ved(root, &["redo", "demo.json"]);
    assert_eq!(mock.synthesis_count(), 2, "redo must not synthesize");
    let redone: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("demo.json")).unwrap()).unwrap();
    assert_eq!(redone["voice_clips"][0]["cache_key"], changed_key);

    ved(
        root,
        &[
            "export-range",
            "demo.json",
            "--from",
            "0",
            "--to",
            "1.5",
            "--out",
            "range.mp4",
            "--quality",
            "preview",
        ],
    );
    ved(
        root,
        &["render", "demo.json", "full.mp4", "--quality", "preview"],
    );
    for output in ["range.mp4", "full.mp4"] {
        let probe = run(
            "ffprobe",
            root,
            &[
                "-v",
                "error",
                "-select_streams",
                "a",
                "-show_entries",
                "stream=codec_type",
                "-of",
                "default=nw=1:nk=1",
                output,
            ],
        );
        assert!(String::from_utf8_lossy(&probe.stdout).contains("audio"));
    }

    let cache_before_remove = std::fs::read_dir(root.join(".ved/cache/tts"))
        .unwrap()
        .filter_map(|entry| entry.ok())
        .count();
    ved(root, &["voice-remove", "demo.json", "v1"]);
    let removed: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("demo.json")).unwrap()).unwrap();
    assert!(removed["voice_clips"].as_array().unwrap().is_empty());
    assert!(removed["audio_clips"].as_array().unwrap().is_empty());
    let cache_after_remove = std::fs::read_dir(root.join(".ved/cache/tts"))
        .unwrap()
        .filter_map(|entry| entry.ok())
        .count();
    assert_eq!(cache_before_remove, cache_after_remove, "remove must keep cache");
    ved(root, &["undo", "demo.json"]);
    assert_eq!(mock.synthesis_count(), 2);
    ved(root, &["redo", "demo.json"]);
    assert_eq!(mock.synthesis_count(), 2);
}

#[test]
fn tts_connection_and_invalid_audio_fail_cleanly() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let connection = ved_fail(
        root,
        &[
            "voice",
            "list",
            "--provider",
            "voicevox",
            "--endpoint",
            "http://127.0.0.1:1",
        ],
    );
    let error = String::from_utf8_lossy(&connection.stderr);
    assert!(error.contains("engine not running") || error.contains("connection refused"));

    let mock = MockEngine::start(true);
    ved(root, &["new", "bad.json"]);
    let invalid = ved_fail(
        root,
        &[
            "voice-add",
            "bad.json",
            "--provider",
            "voicevox",
            "--voice",
            "1",
            "--text",
            "壊れた音声",
            "--at",
            "0",
            "--endpoint",
            &mock.endpoint,
        ],
    );
    let error = String::from_utf8_lossy(&invalid.stderr);
    assert!(error.contains("cache validation failure") || error.contains("invalid returned audio"));
    assert!(
        !root.join(".ved/cache/tts").exists()
            || std::fs::read_dir(root.join(".ved/cache/tts"))
                .unwrap()
                .filter_map(|entry| entry.ok())
                .all(|entry| !entry.path().extension().is_some_and(|value| value == "wav")),
        "invalid WAV must not become a cache entry"
    );
}

struct MockEngine {
    endpoint: String,
    paths: Arc<Mutex<Vec<String>>>,
}

impl MockEngine {
    fn start(invalid_audio: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let paths = Arc::new(Mutex::new(Vec::new()));
        let captured = paths.clone();
        thread::spawn(move || {
            let started = std::time::Instant::now();
            while started.elapsed() < Duration::from_secs(30) {
                match listener.accept() {
                    Ok((mut stream, _)) => handle_request(&mut stream, &captured, invalid_audio),
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            endpoint: format!("http://{address}"),
            paths,
        }
    }

    fn synthesis_count(&self) -> usize {
        self.paths
            .lock()
            .unwrap()
            .iter()
            .filter(|path| path.as_str() == "/synthesis")
            .count()
    }
}

fn handle_request(stream: &mut TcpStream, paths: &Arc<Mutex<Vec<String>>>, invalid_audio: bool) {
    let request = read_http_request(stream);
    let first_line = request.lines().next().unwrap_or_default();
    let raw_path = first_line.split_whitespace().nth(1).unwrap_or("/");
    let path = raw_path.split('?').next().unwrap_or(raw_path).to_owned();
    paths.lock().unwrap().push(path.clone());
    let (status, content_type, body) = match path.as_str() {
        "/version" => ("200 OK", "application/json", b"\"mock-1.0\"".to_vec()),
        "/speakers" => (
            "200 OK",
            "application/json",
            br#"[{"name":"Mock Speaker","speaker_uuid":"mock-uuid","styles":[{"id":1,"name":"Normal"}]}]"#.to_vec(),
        ),
        "/audio_query" => (
            "200 OK",
            "application/json",
            br#"{"speedScale":1.0,"pitchScale":0.0,"volumeScale":1.0,"kana":null}"#.to_vec(),
        ),
        "/synthesis" if invalid_audio => ("200 OK", "audio/wav", b"not a wav".to_vec()),
        "/synthesis" => ("200 OK", "audio/wav", tiny_wav()),
        _ => ("404 Not Found", "text/plain", b"not found".to_vec()),
    };
    let header = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(header.as_bytes()).unwrap();
    stream.write_all(&body).unwrap();
}

fn read_http_request(stream: &mut TcpStream) -> String {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 4096];
    let mut header_end = None;
    let mut content_length = 0_usize;
    loop {
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => {
                bytes.extend_from_slice(&buffer[..count]);
                if header_end.is_none()
                    && let Some(position) = bytes.windows(4).position(|value| value == b"\r\n\r\n")
                {
                    header_end = Some(position + 4);
                    let headers = String::from_utf8_lossy(&bytes[..position + 4]);
                    content_length = headers
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().ok())
                                .flatten()
                        })
                        .unwrap_or(0);
                }
                if let Some(end) = header_end
                    && bytes.len() >= end + content_length
                {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

fn tiny_wav() -> Vec<u8> {
    let sample_rate = 16_000_u32;
    let samples = 8_000_u32;
    let data_size = samples * 2;
    let mut wav = Vec::new();
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_size).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16_u32.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    wav.extend_from_slice(&2_u16.to_le_bytes());
    wav.extend_from_slice(&16_u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_size.to_le_bytes());
    wav.resize((44 + data_size) as usize, 0);
    wav
}
