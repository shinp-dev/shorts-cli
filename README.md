# shorts-cli (`ved`)

`ved` is a small Rust CLI for deterministic, AI-friendly video editing. Phase 1 can
import videos, build a gapless timeline, preview arbitrary ranges, capture frames, and
export MP4 through FFmpeg.

Requirements: FFmpeg, FFprobe, and FFplay on `PATH`, plus a Rust 1.92 or newer toolchain.

```powershell
cargo build --release
ved doctor
ved new demo.json --preset vertical
ved import demo.json game.mp4
ved import demo.json reaction.mp4
ved add demo.json game.mp4 --in 3.2 --out 8.4
ved insert demo.json reaction.mp4 --at 2.5 --in 0 --out 1.2 --volume 0.8
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

The reviewed Phase 1 contracts and deferred design decisions are in
[`docs/design-phase-1.md`](docs/design-phase-1.md).
