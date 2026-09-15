# shorts-cli (`ved`)

`ved` is a small Rust CLI for deterministic, AI-friendly video editing. It can import
media, build a gapless video timeline, apply text/images/independent audio, preview
arbitrary ranges, capture frames, and export MP4 through FFmpeg. Phase 3 adds a compact
agent mode, atomic audio schedules, local TTS, still-frame holds, selected retiming,
track-based ducking, and safe iterative overwrite.

Requirements: FFmpeg, FFprobe, and FFplay on `PATH`, plus a Rust 1.92 or newer toolchain.

```powershell
cargo build --release
ved doctor
ved new demo.json --preset vertical
ved import demo.json game.mp4
ved import demo.json reaction.mp4
ved add demo.json game.mp4 --in 3.2 --out 8.4
ved insert demo.json reaction.mp4 --at 2.5 --in 0 --out 1.2 --volume 0.8
ved speed demo.json --clip c1 --rate 1.5
ved hold demo.json --clip c1 --source 5.2 --duration 2 --retime-overlays --retime-track sfx
ved text-add demo.json --text "でもちゃんと先を読む。" --from 1 --to 4 --position bottom-center
ved image-add demo.json logo.png --from 2 --to 6 --x 85% --y 8% --width 12%
ved audio-add demo.json bgm.wav --at 0 --key bed --track bgm --loop --to timeline --volume 0.3
ved audio-sync demo.json mix.json --replace-track narration --replace-track sfx
ved duck-add demo.json --key voice-over-bgm --target-track bgm --trigger-track narration
ved mute demo.json --clip c2
ved describe demo.json --json
ved frame demo.json 3.0 check.png
ved play demo.json --from 0 --to 6
ved export-range demo.json --from 2 --to 6 --out review.mp4
ved render demo.json final.mp4 --overwrite
```

`insert`, `remove`, `play`, `frame`, and `export-range` use completed-timeline time.
`trim --clip c1 --in 4 --out 8` uses source-media time. Existing outputs are rejected
unless `--overwrite` is explicit; replacement is atomic. Use
`ved render demo.json final.mp4 --dry-run --json` for a compact render summary and
plan hash, or add `--verbose` for the complete plan and FFmpeg argument vector.

Text and image properties can be changed with `text-set` and `image-set`; their matching
`*-remove` commands delete them. `volume` and `mute` target either `--clip c1` or
`--audio a1`. `fade-in` and `fade-out` target independent audio IDs. Every project edit
is recorded automatically; use `ved undo demo.json` and `ved redo demo.json` to move
through up to 100 whole-project snapshots.

For agent workflows, add global `--json` to receive exactly one compact JSON object or
`--quiet` to suppress successful output. `audio-sync` resolves manifest-relative files,
upserts audio by stable `(track, key)`, imports or refreshes media, and saves the entire
schedule as one transaction. Reapplying an unchanged manifest is a no-op. Manifest TTS
entries support local VOICEVOX, AivisSpeech, and Windows SAPI; discover installed voices
with `ved voices --json`. See the Phase 3 design for the manifest schema and source-linked
time expressions.

The first Phase 4 executor foundation is available for bounded Luna-style finishing
tasks:

```powershell
ved capabilities --json
ved inspect edit.json --json
ved brief-check edit.json brief.json --json
ved check edit.json --profile shorts --json
ved loudness edit.json --track narration --json
ved waveform edit.json --track narration --out narration.png --peaks-out peaks.json --json
ved markers edit.json --include timeline --include audio --out markers.json --json
ved review-build edit.json --out reviews --notes review-notes.json --json
```

`brief-check` verifies the project hash, stable targets, safety classes, text bounds, and
rejects secret or executable payloads without modifying the project. `review-build` runs
checks and atomically returns a preview MP4, 12-frame storyboard, marker JSON, review JSON,
and a default Markdown report capped at 2 KiB. Repeating the same build reuses the verified
bundle.

`loudness` measures one independent audio clip, one compiled track, or the final compiled
mix through FFmpeg's EBU R128 `loudnorm` analysis and reuses a content-addressed cache.
`waveform` writes a deterministic PNG and can place compact min/max peaks in a separate
JSON artifact. `markers` exports stable marker IDs as JSON, CSV, WebVTT, or FFmetadata.

AI agents and contributors should read the canonical boundary contract in
[`docs/agent-contract.md`](docs/agent-contract.md) before changing behavior.

The reviewed Phase 1 contracts and deferred design decisions are in
[`docs/design-phase-1.md`](docs/design-phase-1.md).
Phase 2 schema and time-coordinate decisions are in
[`docs/design-phase-2.md`](docs/design-phase-2.md).
Phase 3 agent workflow, batch audio, retiming, and TTS contracts are in
[`docs/design-phase-3.md`](docs/design-phase-3.md).
Phase 4 assisted production, analysis, visual polish, external providers, and review
handoff contracts are in [`docs/design-phase-4.md`](docs/design-phase-4.md). Its reference
workflow keeps story and taste decisions with a human/director while delegating bounded
execution to a lower-cost executor, targeting at least 70% fewer director text tokens on
the fixed finishing benchmark.
