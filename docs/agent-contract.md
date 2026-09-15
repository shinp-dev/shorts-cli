# shorts-cli Agent Contract

This is the canonical boundary contract for AI agents and contributors changing `shorts-cli`.
It is intentionally not a complete command reference. Its purpose is to prevent plausible but
wrong assumptions from GUI/NLE editors from leaking into this deterministic CLI.

Current project schema: **v3**.

If this document, tests, and implementation disagree, **stop and reconcile the disagreement**.
Do not invent behavior. Executable tests and implementation determine current behavior; this file
is the contributor-facing statement of the intended boundaries.

## 1. Core mental model

The render path is one-way:

```text
Project intent
  -> load + in-memory schema migration + validation
  -> TTS materialization on a project clone when needed
  -> ResolvedTimeline
  -> RenderPlan
  -> FFmpeg argument/filter construction
  -> ffmpeg / ffplay
```

`Project` stores editing intent, not a cached render graph. Video timeline positions are derived.
`RenderPlan` contains typed semantic values, not FFmpeg filter syntax. The FFmpeg adapter is the
only layer that creates filter expressions and process arguments.

**Do not add render behavior to one CLI command only.** `render`, `play`, `frame`, and
`export-range` all go through the common compiler/RenderPlan path. TTS is materialized into normal
audio media/`AudioClip`s before that path; it does not get a separate renderer.

## 2. Time coordinates

There are two different time spaces. Never infer one from an option name alone.

| Operation / property | Coordinate |
| --- | --- |
| `add --in/--out` | source-media time |
| `insert --in/--out` | source-media time |
| `trim --in/--out` | source-media time |
| `insert --at` | completed/final timeline time |
| `remove --from/--to` | completed/final timeline time, half-open `[from,to)` |
| `text` start/end | completed/final timeline time |
| `image` start/end | completed/final timeline time |
| `AudioClip.start` / `audio-add --at` | completed/final timeline time |
| `AudioClip.source_in/source_out` | source-media time of that audio media |
| `voice-add --at` / `voice-set --at` | completed/final timeline time, stored in linked `AudioClip.start` |
| `play --from/--to` | completed/final timeline time |
| `frame <time>` | completed/final timeline time |
| `export-range --from/--to` | completed/final timeline time |

For a video clip:

```text
final_duration = (source_out - source_in) / speed
```

Mapping a final offset inside a clip back to source media multiplies by `speed`.

### Example: sped clip

A clip uses source `10.0..16.0` at `2.0x`. Source duration is `6.0s`; final duration is
`3.0s`. `trim --in 11 --out 15` uses source seconds. `insert --at 2.0` uses final timeline
seconds; if it lands inside this clip, the source split point is derived through the clip speed.

Project times remain floating-point seconds. Do not persist frame indices or pre-quantize edit
coordinates.

## 3. Sources of truth

| Concern | Source of truth |
| --- | --- |
| Video clip ordering | `Project.timeline` array order |
| Video final start/end | derived by resolving ordered clips |
| Video final duration | source range divided by `Clip.speed` |
| Text timing | `TextOverlay.start/end` |
| Image timing | `ImageOverlay.start/end` |
| Independent/TTS playback placement | linked `AudioClip.start` |
| Independent/TTS playback volume, mute, fades | `AudioClip` |
| TTS text/provider/voice/synthesis speed/pitch | `VoiceClip` |
| TTS endpoint/engine identity/cache key | `VoiceClip` |
| TTS generated WAV reference and probe metadata | linked `AudioClip.media_id` -> `Media` |
| TTS duration | probed duration of the generated WAV `Media`, never text estimation |
| Render-relative clipped times | derived in `RenderPlan`, never written back to Project |

A `VideoClip` has no persisted completed-timeline `start`, `end`, or duration. Adding those fields
would create a second source of truth and violate the model.

## 4. What does NOT automatically move

Video editing ripples **video clips only** because their final positions are derived from ordered
`Project.timeline` content.

Text overlays, image overlays, independent audio, and TTS playback positions are absolute final
seconds. `insert`, `remove`, `trim`, and `speed` do **not** rewrite those stored times.

### Example: insert without overlay ripple

Text `t1` is at `2.0..4.0`. Insert a one-second video clip at final time `0.0`. The video sequence
becomes one second longer, but `t1` stays `2.0..4.0`; it does **not** become `3.0..5.0`.

Do not implement GUI-editor-style linked/ripple movement unless an explicit future specification
changes this contract.

## 5. Derived vs persisted state

Persist intent; derive render state.

Do not persist:

- resolved video timeline starts/ends;
- range-relative overlay/audio times;
- FFmpeg labels, expressions, filter graphs, or argument vectors;
- frame indices derived from seconds;
- estimated TTS duration.

Schema v1/v2 files are migrated to v3 **in memory on load**. A read-only operation does not rewrite
the file. The migrated v3 representation is written only when an edit later saves the project.
Migration does not re-probe imported media.

## 6. TTS ownership, generated media, and cache

`VoiceClip` owns synthesis identity:

- `provider`
- `voice`
- `text`
- synthesis `speed`
- `pitch`
- `endpoint`
- `engine_identity`
- `cache_key`
- `audio_clip_id`

It deliberately does **not** own timeline position, playback volume, mute, fades, or source range.
Those belong to the linked `AudioClip`.

The linked `AudioClip` owns:

- `start`
- `source_in/source_out`
- playback `volume`
- `mute`
- `fade_in/fade_out`
- `media_id`

That `Media` points to the generated WAV under `.ved/cache/tts/` and stores its probe metadata.
TTS WAVs are real project-referenced media, but the cache file itself is not history state.

`voice-remove` removes the `VoiceClip`, its linked `AudioClip`, and an unneeded project `Media`
record. It **does not delete the cached WAV**.

Undo/redo restores project JSON only. It does not copy/delete/restore imported files or cache files,
and the undo/redo operation itself does not synthesize. If a referenced TTS cache file is later
missing/invalid, a render-path compilation may re-materialize it.

### Cache identity

Current cache key identity includes:

- cache-key schema version;
- provider;
- engine identity/version returned by that provider;
- voice/style ID;
- text;
- TTS synthesis speed;
- pitch.

No additional provider-specific synthesis-affecting field is currently exposed. If one is added,
it must join the canonical cache identity at the provider boundary.

Current cache identity excludes:

- timeline position;
- playback volume/mute/fades;
- video clips;
- text/image overlays;
- canvas;
- output/range/quality settings;
- endpoint **as a direct field** (engine identity is included).

### Example: synthesis vs playback edits

Changing `text`, `voice`, TTS `speed`, or `pitch` changes synthesis identity and can require a new
WAV. Changing `--at` or playback `--volume` updates only the linked `AudioClip` and must not trigger
synthesis.

An explicit synthesis operation obtains current engine identity before deciding its cache key, so a
cache hit is not a promise that no provider request occurs. Rendering an existing project with a
valid WAV at its stored cache key can reuse that file without contacting the engine.

## 7. Render side effects and architecture

`compiler::compile` clones the loaded project and runs TTS materialization before building the
`RenderPlan`. Materialization updates the clone, not project JSON.

Therefore `render`, `play`, `frame`, `export-range`, and their dry-run compilation can validate TTS
cache files and, when a required cache entry is missing/invalid, may contact the configured TTS
provider and write `.ved/cache/tts/`.

Do **not** assume `--dry-run` is filesystem/network side-effect-free when the project contains TTS.
Do **not** assume `frame` can always ignore TTS availability merely because its output has no audio.

The FFmpeg adapter alone may produce FFmpeg filter syntax. `RenderPlan` must remain typed semantic
data.

## 8. Coordinates and sizing

Percent coordinates (`0%..100%`) denote the **center point of the overlay** on the canvas axis.
Named edge coordinates are different: `left`/`right`/`top`/`bottom` align the corresponding overlay
edge with a 5% safe canvas margin. They are not simple percentage aliases.

`--position bottom-center` normalizes to `x = center`, `y = bottom`. Named center and `50%` happen
to resolve to the same geometric center, but named edges and percentages have different semantics.

Image width/height may be pixels or percentages. Percentage width is relative to canvas width;
percentage height is relative to canvas height. If exactly one dimension is omitted, FFmpeg keeps
aspect ratio. If both are omitted, imported image dimensions are used.

## 9. Timeline bounds and frame quantization

Overlay/audio/TTS placement is not required to fit inside the current video duration. Validity only
requires valid non-negative intervals/starts and valid referenced media.

### Example: out-of-range overlay

A five-second video may contain an image at `8.0..10.0`. The project remains valid. Normal rendering
of the five-second video intersects items with the render range, so that image is simply not shown.

`frame` accepts a final timeline second, not a frame index. The model/compiler do not round that
second before storage. The FFmpeg adapter normalizes video to project FPS, and decoding/seeking can
select the nearest representable frame boundary.

### Example: frame timing

At 30 fps, a request such as `4.250s` need not correspond to an exact encoded frame timestamp.
A difference of a few tens of milliseconds in observed output is not automatically a bug. Do not
rewrite project times to `N / fps` merely to make them look frame-aligned.

## 10. History, persistence, and media ownership

Every normal project edit records a whole-project JSON snapshot, then saves through the existing
atomic project-save path. Undo/redo history is capped at 100 snapshots per stack.

Project save writes a same-directory temporary file, flushes it, and atomically replaces the target
(the Windows path uses replace + write-through). Do not bypass `project::save` with ad-hoc truncating
writes.

`import` references source media in place. Files under the project directory are stored as relative
paths; external files are stored as absolute paths. **Import does not copy, rewrite, transcode, or
freeze the source file.** Probe metadata is persisted and is not automatically refreshed on every
load.

Treat imported source files as externally owned and immutable for deterministic operation. The CLI
must never overwrite an imported input file.

## 11. Process, HTTP, diagnostics, and output boundaries

External process execution uses `std::process::Command` with discrete arguments for `ffmpeg`,
`ffprobe`, and `ffplay`. TTS HTTP uses the Rust HTTP client directly.

Never introduce:

- `cmd /C`;
- PowerShell execution;
- `sh -c`;
- `shell=true`-style execution;
- `curl`/HTTP subprocesses;
- user-controlled text interpolated into a shell command.

`ved doctor` is diagnostic/reporting. Current implementation prints availability rows and returns
success even when a row is unavailable. Operationally, FFmpeg/FFprobe/FFplay are required by the
commands that use them; VOICEVOX/AivisSpeech are optional dependencies for non-TTS workflows.
Unavailable local TTS engines must not make unrelated editing fail.

Real output writes (`render`, `export-range`, `frame`) reject an already-existing output and reject
using an input media path as output. Do not assume implicit overwrite. Dry-run only builds/displays
the plan/arguments and does not perform the existing-output guard.

## 12. IDs are identities, not indexes

Prefixes are collection-scoped identities:

- `mN`: media;
- `cN`: video clip;
- `tN`: text overlay;
- `iN`: image overlay;
- `aN`: independent/playback audio clip, including TTS-linked audio;
- `vN`: TTS voice clip.

Commands target IDs, not array positions. Reordering/ripple resolution must not reinterpret an ID as
an index.

Surviving objects keep their IDs, but edits that split a video clip may create a new ID for the
right-hand fragment. Allocation is `max(current numeric suffix) + 1`; it is not a permanent global
tombstone sequence, so deleting the current highest ID can allow that number to be reused later.
Do not infer historical identity solely from a numeric suffix.

## 13. Unsupported assumptions

Do not assume any of the following unless a future implementation explicitly adds it:

- GUI/NLE-style linked ripple for overlays/audio/TTS;
- persisted video timeline coordinates;
- imported-media copying or automatic re-probing;
- TTS provider/endpoint switching through `voice-set`;
- raw user FFmpeg filters in project intent;
- frame-index editing semantics;
- implicit output overwrite;
- cache deletion as part of `voice-remove` or undo;
- command-specific rendering pipelines.

## 14. Agent invariants

**MUST**

- preserve source-media vs completed-timeline time semantics;
- derive video final positions from ordered clips, source ranges, and speed;
- keep overlay/audio/TTS playback times absolute unless explicitly edited;
- keep TTS synthesis identity in `VoiceClip` and playback state in linked `AudioClip`;
- take TTS duration from probed generated audio;
- reuse the common compiler/`RenderPlan` path;
- keep FFmpeg syntax inside the FFmpeg adapter;
- use existing history and atomic-save paths for project edits;
- treat IDs as identities, not indexes;
- preserve the no-shell process/HTTP boundary.

**MUST NOT**

- persist derived video start/end or render-relative times;
- ripple overlays/audio/TTS implicitly after video edits;
- rebuild TTS for position/volume/mute/fade-only changes;
- estimate TTS duration from text;
- delete imported media or TTS cache as part of undo/redo;
- delete TTS cache merely because `voice-remove` was called;
- quantize stored project seconds to frames;
- add a render/filter rule to only `play`, `frame`, `export-range`, or `render`;
- bypass atomic project save;
- construct shell command strings from user input.
