# shorts-cli (`ved`)

`ved` is a small Rust CLI for deterministic, AI-friendly video editing. It can import
media, build a gapless video timeline, apply text/images/independent audio, preview
arbitrary ranges, capture frames, and export MP4 through FFmpeg.

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
ved text-add demo.json --text "でもちゃんと先を読む。" --from 1 --to 4 --position bottom-center
ved image-add demo.json logo.png --from 2 --to 6 --x 85% --y 8% --width 12%
ved audio-add demo.json bgm.wav --at 0 --volume 0.3 --fade-in 0.5 --fade-out 1
ved mute demo.json --clip c2
ved describe demo.json
ved frame demo.json 3.0 check.png
ved play demo.json --from 0 --to 6
ved export-range demo.json --from 2 --to 6 --out review.mp4
ved render demo.json final.mp4
```

`insert`, `remove`, `play`, `frame`, and `export-range` use completed-timeline time.
`trim --clip c1 --in 4 --out 8` uses source-media time. Outputs are never overwritten.
Use `ved render demo.json final.mp4 --dry-run` to inspect the resolved render plan and
exact FFmpeg argument vector without writing a file.

Text and image properties can be changed with `text-set` and `image-set`; their matching
`*-remove` commands delete them. `volume` and `mute` target either `--clip c1` or
`--audio a1`. `fade-in` and `fade-out` target independent audio IDs. Every project edit
is recorded automatically; use `ved undo demo.json` and `ved redo demo.json` to move
through up to 100 whole-project snapshots.

AI agents and contributors should read the canonical boundary contract in
[`docs/agent-contract.md`](docs/agent-contract.md) before changing behavior.

The reviewed Phase 1 contracts and deferred design decisions are in
[`docs/design-phase-1.md`](docs/design-phase-1.md).
Phase 2 schema and time-coordinate decisions are in
[`docs/design-phase-2.md`](docs/design-phase-2.md).
Phase 3 TTS ownership and cache decisions are in
[`docs/design-phase-3.md`](docs/design-phase-3.md).
