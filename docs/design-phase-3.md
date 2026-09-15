# Phase 3 agent workflow design

This document fixes the Phase 3 contracts for token-efficient, agent-friendly audio
workflows. Phase 3 preserves the existing compilation boundary:

```text
CLI mutation -> validated Project -> ResolvedTimeline -> RenderPlan -> FFmpeg arguments
```

The project remains editing intent. Batch commands mutate that intent atomically; they
do not persist FFmpeg syntax or an execution log.

## Goals and acceptance targets

Phase 3 optimizes repeated machine-driven edits without making interactive commands
harder for humans.

- Applying a prepared narration/BGM/SFX schedule to an existing project takes one
  command and creates one history snapshot.
- BGM can be lowered automatically while narration is active without per-cue volume
  edits or waveform analysis.
- Reapplying the same schedule is idempotent: it creates no duplicate media or audio
  clips and reports `changed = false`.
- Slowing a video and moving selected timed items with it takes one command. Agents do
  not calculate every replacement timestamp themselves.
- Extending a title, result, or reaction frame does not require slowing the surrounding
  motion. A hold is a first-class timeline item that can be added or resized in one
  command.
- A pronunciation or source-file correction can be applied and rendered in at most two
  project commands: `audio-sync` (or `media-refresh`) and `render --overwrite`.
- Successful machine-readable mutations emit one JSON value of at most 1 KiB unless
  verbose detail is explicitly requested.
- Multi-item changes are all-or-nothing. A failed probe, TTS request, reference, or
  validation never leaves a partially edited project.

Automatic content understanding, transcription, scene detection, arbitrary FFmpeg
filters, and a general-purpose edit-operation DSL are not Phase 3 goals. A narrow audio
schedule is easier to validate, document, and generate correctly.

## Common agent output contract

Every command accepts global `--json` and `--quiet` flags. They are mutually exclusive
and may appear before or after the subcommand.

- The default remains concise human-readable text.
- `--quiet` writes nothing on success. Errors still go to stderr and use a non-zero exit
  status.
- `--json` writes exactly one compact JSON object to stdout on success. FFmpeg progress
  and diagnostics never enter stdout.
- With `--json`, a handled error writes exactly one compact JSON object to stderr and
  exits non-zero.
- `--verbose` may be combined with human or JSON output when per-item details or the
  complete FFmpeg argument vector are genuinely needed.

The stable success envelope is:

```json
{
  "ok": true,
  "command": "audio-sync",
  "changed": true,
  "project": "edit.json",
  "duration": 23.077,
  "created": { "media": 8, "audio": 15, "ducking": 1 },
  "updated": { "media": 0, "audio": 0, "ducking": 0 },
  "removed": { "media": 0, "audio": 0, "ducking": 0 },
  "warnings": []
}
```

Counts, rather than full item arrays, are the default. `--verbose` adds item IDs. Fields
that are not relevant to a command are omitted.

The stable error envelope is:

```json
{
  "ok": false,
  "code": "media_duration_conflict",
  "message": "a4 ends past the refreshed media duration",
  "details": { "media_id": "m5", "audio_id": "a4" }
}
```

Error `code` values are lowercase snake case and are part of the compatibility contract.
Human wording is not. A JSON command must not require a follow-up `describe` merely to
learn its resulting timeline duration or whether it changed the project.

`render --dry-run --json` returns a compact plan summary by default: duration, canvas,
input/output counts, and a deterministic plan hash. The complete render plan and FFmpeg
argument vector require `--verbose`.

## Arbitrary canvas creation

`new` accepts either a preset or an explicit canvas:

```powershell
ved new edit.json --preset vertical --fps 30
ved new edit.json --width 1080 --height 2400 --fps 30
```

`--width` and `--height` must be supplied together and conflict with an explicitly
supplied `--preset`. All three values must be positive integers. This removes the need
to patch project JSON merely to preserve a source video's dimensions.

## Project schema version 3

Phase 3 increments the project schema from version 2 to 3. A version 2 project migrates
in memory with all new values at their defaults and is written as version 3 on the next
successful edit. Migration never probes or changes media files.

The `timeline` array becomes an untagged union of existing video clips and hold items.
Existing video clip JSON stays byte-for-byte structurally compatible; it does not gain a
required discriminator merely for the new variant. A hold is distinguished by
`freeze_at` and `duration`:

```json
{
  "id": "h1",
  "media_id": "m1",
  "freeze_at": 10.166667,
  "duration": 2.5
}
```

Hold IDs use the independent `hN` sequence. `freeze_at` is a source-media second and
must select a decodable video frame. `duration` is a finite positive completed-timeline
duration. A hold has no attached source audio; the base audio for its interval is
silence, while independent narration, BGM, and SFX continue normally.

The canonical Rust model is conceptually `Vec<TimelineItem>`, where `TimelineItem` is
either the existing `Clip` shape or the new `HoldClip` shape. The two shapes have
disjoint required fields and both reject unknown fields. All resolved-timeline,
range-export, history, and ID-allocation behavior applies to both variants.

Independent audio gains the following optional intent:

```json
{
  "id": "a7",
  "key": "result-sting",
  "track": "sfx",
  "media_id": "m11",
  "start": 15.64,
  "end": null,
  "source_in": 0.0,
  "source_out": null,
  "speed": 1.0,
  "loop": false,
  "volume": 0.16,
  "mute": false,
  "fade_in": 0.0,
  "fade_out": 0.0
}
```

Defaults (`key = null`, `track = null`, `end = null`, `speed = 1`, and `loop = false`)
are omitted from canonical JSON.

`key` and `track` use `[A-Za-z0-9][A-Za-z0-9._-]{0,63}`. The pair `(track, key)` is
unique among audio clips when `key` is present. Keys provide stable, agent-chosen
identity for synchronization; generated `aN` IDs remain the project's internal stable
identity.

The selected source duration is `source_out - source_in`. Its natural completed duration
is that duration divided by `speed`.

- `speed` is finite and within `0.05..=20.0`.
- If `end` is omitted and `loop` is false, the clip ends after its natural completed
  duration.
- If `end` is present and `loop` is false, it must be no later than the natural end and
  trims the completed clip.
- If `loop` is true, `end` is required and must be greater than `start`. The selected
  source range repeats until `end`.
- `fade_in` and `fade_out` are completed-timeline seconds. Each must not exceed the
  resolved completed duration.

The compiler applies the existing factorized `atempo` chain to independent audio. For
looped audio it trims the source range, changes its tempo, loops the resulting stream,
then trims it to `[start, end)`. Fades are applied after tempo and looping.

The project root also gains an empty-by-default `audio_ducking` array:

```json
{
  "id": "d1",
  "key": "voice-over-bgm",
  "target_track": "bgm",
  "trigger_tracks": ["narration"],
  "reduction_db": 12.0,
  "attack": 0.12,
  "release": 0.35
}
```

Ducking IDs use the independent `dN` sequence. `key` follows the audio-key syntax and
is unique among ducking rules. `target_track` and every trigger track use the audio-track
syntax. Trigger tracks must be non-empty and unique, and the target cannot trigger its
own rule. Only one rule may target a given track; multiple trigger tracks belong in that
rule.

`reduction_db` is finite and within `(0, 60]`. `attack` and `release` are finite
completed-timeline seconds within `0..=10`. Version 2 migration supplies an empty array.
Rules may temporarily refer to tracks with no clips; validation allows this and render
reports one compact warning because audio schedules are commonly assembled in stages.

## Single-item audio commands

The existing `audio-add` accepts these additions:

```powershell
ved audio-add edit.json voice.wav --at 3.3 --key intro --track narration --speed 0.95
ved audio-add edit.json bgm.wav --at 0 --track bgm --loop --to timeline
```

`--to` accepts a completed-timeline second or the literal `timeline`. `timeline` resolves
to the current video timeline duration when the edit is saved.

Independent audio speed has a dedicated command so it cannot be confused with video
clip speed:

```powershell
ved audio-speed edit.json --audio a3 --rate 0.95
```

Changing audio speed preserves its `start` and explicit `end`. If an explicit non-looped
end would exceed the new natural end, the command fails atomically rather than silently
padding it.

## Media refresh

Imported probe data is still stable during ordinary commands and rendering. Explicit
refresh is available for files replaced in place:

```powershell
ved media-refresh edit.json m5
ved media-refresh edit.json --all
```

Refresh probes every requested path first, constructs the complete candidate project,
and validates all references before writing anything. A kind change is rejected. A
duration or dimension change is accepted only if the resulting project remains valid.
On failure, the error identifies every conflicting clip or overlay in verbose JSON.

Refreshing unchanged media records no history snapshot and reports `changed = false`.
`audio-sync` automatically performs this same refresh for every file named in its
manifest, so the common agent workflow does not need a separate command.

## Atomic audio synchronization

`audio-sync` imports, refreshes, creates, and updates independent audio in one validated
transaction:

```powershell
ved audio-sync edit.json mix.json
```

Manifest paths are resolved relative to the manifest, not the current working
directory. The manifest is UTF-8 JSON and rejects unknown fields.

```json
{
  "version": 1,
  "defaults": {
    "volume": 1.0,
    "speed": 1.0,
    "fade_in": 0.0,
    "fade_out": 0.0
  },
  "clips": [
    {
      "key": "bgm",
      "track": "bgm",
      "file": "audio/bgm.wav",
      "at": 0,
      "loop": true,
      "to": "timeline",
      "volume": 0.22,
      "fade_out": 0.8
    },
    {
      "key": "hook",
      "track": "narration",
      "file": "audio/hook.wav",
      "at": { "clip": "c1", "source": 0.03 }
    },
    {
      "key": "victory",
      "track": "sfx",
      "file": "audio/victory.wav",
      "at": { "clip": "c1", "source": 10.17 },
      "volume": 0.16
    }
  ],
  "ducking": [
    {
      "key": "voice-over-bgm",
      "target_track": "bgm",
      "trigger_tracks": ["narration"],
      "reduction_db": 12,
      "attack": 0.12,
      "release": 0.35
    }
  ]
}
```

Each entry requires `key`, `track`, `at`, and exactly one of `file` or `tts`. File entries
may also specify `source_in`, `source_out`, `speed`, `volume`, `mute`, `fade_in`,
`fade_out`, `loop`, and `to`.

The synchronization rules are:

1. Resolve and probe every referenced file, deduplicating media by canonical path.
2. Resolve all time expressions against the pre-edit video timeline.
3. Match existing audio by `(track, key)`.
4. Update matches, create missing entries, and leave unlisted entries unchanged.
5. Upsert ducking rules by `key` when the optional manifest array is present.
6. Validate the complete project and save it as one history snapshot.

Running the same manifest twice is a no-op. Changing the file, timing, or mix properties
updates the existing `aN`; it does not allocate another audio ID.

Stale items are removed only with an explicit repeated track selector:

```powershell
ved audio-sync edit.json mix.json --replace-track narration --replace-track sfx
```

For each selected track, existing keyed items absent from the manifest are removed.
Unkeyed audio and other tracks are never removed. Imported media that becomes unreferenced
is retained; media pruning remains a separate future operation.

`--replace-ducking` similarly removes project ducking rules whose keys are absent from
the manifest. Without that flag, unlisted rules remain unchanged.

If any step fails, the project and history are unchanged. Successfully generated TTS
cache files may remain because they are immutable cache entries, not project state.

### Time expressions

`at` and numeric `to` accept either a finite non-negative completed-timeline second or a
source-linked expression:

```json
{ "clip": "c1", "source": 10.17, "offset": 0.05 }
```

`offset` defaults to zero and is in completed-timeline seconds. The resolved time is:

```text
resolved_start(clip) + (source - clip.source_in) / clip.speed + offset
```

`source` must lie within the named clip's source range. This lets an agent copy observed
source-video event times into a manifest without manually dividing every value after a
speed change. Reapplying the manifest after changing video speed recalculates those
positions and updates the same keyed audio.

## Still-frame holds

A hold extends a visually stable moment without slowing motion before or after it. The
canonical agent form uses a clip and source-media time:

```powershell
ved hold edit.json --clip c1 --source 10.17 --duration 2.5 --retime-overlays --retime-track sfx
```

Convenience forms place a hold on a clip boundary:

```powershell
ved hold edit.json --after c1 --duration 2.5
ved hold edit.json --before c2 --duration 1.0
```

Exactly one of `--clip ... --source ...`, `--after`, or `--before` is required.

- An interior source position splits the video clip. The left side retains the original
  `cN`, the hold receives the next `hN`, and the right side receives the next `cN`.
- A source position equal to the clip start inserts the hold before the unchanged clip.
- A source position equal to the clip end inserts the hold after the unchanged clip.
- `--before` freezes the first frame of the named clip.
- `--after` freezes the last complete frame of the named clip. The stored `freeze_at` is
  the chosen frame timestamp, using source FPS when known and project FPS otherwise, so
  later renders do not repeat the boundary calculation.
- Holds cannot target another hold. `speed` and `trim` reject `hN` IDs.

For an interior split at source time `T`, the left clip ends at `T` and the right clip
starts at `T`; the hold displays the frame selected at `T`. The compiler materializes
one frame and clones it for the hold duration. It does not seek or decode the source for
every output frame.

Hold duration can be revised or the hold can be removed by stable ID:

```powershell
ved hold-set edit.json --hold h1 --duration 3.0 --retime-overlays --retime-track sfx
ved hold-remove edit.json --hold h1 --retime-overlays --retime-track sfx
```

Adding a hold of duration `H` at completed time `P` uses this mapping for explicitly
selected timed items:

```text
F(t) = t       when t <= P
F(t) = t + H   when t > P
```

The exact insertion cue stays at `P`, allowing narration to begin on the held frame.
Later cues move after the hold. An overlay that spans `P` has only its end shifted and
therefore remains visible during the hold. As with video speed, `--retime-overlays` maps
text/image endpoints and repeated `--retime-track` selectors map audio starts and
explicit ends; natural audio duration and speed do not change.

`hold-set` applies the same mapping with `H` equal to `new_duration - old_duration`.
`hold-remove` uses `H = -old_duration`. Commands fail atomically if a negative change
would map any selected time below zero or collapse a text/image interval. Unselected
timed items retain Phase 2 absolute-time behavior.

The existing range-based `remove` command may shorten or delete a hold when its removal
range intersects one. A hold cannot be split into two holds: removing a strict interior
range shortens the same `hN` by the removed duration. `hold-remove` is preferred for
agent workflows because it avoids resolving the hold's completed-timeline range.

## Video speed with selected retiming

Phase 2 behavior remains the default: changing video speed does not move independent
timed items. Phase 3 adds explicit selectors:

```powershell
ved speed edit.json --clip c1 --rate 0.65 --retime-overlays --retime-track narration --retime-track sfx
```

For the edited clip, let its old completed interval be `[S, E]`, its new end be `E2`,
and `r = (E2 - S) / (E - S)`. The piecewise mapping is:

```text
F(t) = t                         when t < S
F(t) = S + (t - S) * r          when S <= t <= E
F(t) = t + (E2 - E)             when t > E
```

- `--retime-overlays` maps both endpoints of every text and image overlay with `F`.
- Each `--retime-track TRACK` maps `start` and an explicit `end` for audio in that track.
- Audio source ranges, audio speed, and natural duration are not changed. This preserves
  natural narration delivery while moving its cue.
- Untargeted items, including BGM unless its track is selected, remain unchanged.
- The video speed change and every retimed item form one history snapshot.

The command returns overlap warnings but does not reject intentional overlaps. Source-
linked audio manifests remain the preferred workflow when the original event times are
known; explicit retiming is for projects already arranged in completed-timeline time.

## TTS entries and cache

TTS is available through `audio-sync`; it is not a separate render path. A generated WAV
is cached, imported as ordinary media, and attached as an ordinary audio clip.

```json
{
  "key": "hook",
  "track": "narration",
  "at": 0.05,
  "tts": {
    "provider": "voicevox",
    "voice": "3",
    "text": "弱いAIを作るほうが、難しい説。",
    "rate": 1.0,
    "pitch": 0.0,
    "intonation": 1.0
  }
}
```

Phase 3 providers are local VOICEVOX, local AivisSpeech, and Windows SAPI when available.
`provider`, `voice`, and `text` are required. Generic `rate`, `pitch`, and `intonation`
default to neutral values; unsupported non-default options produce an error instead of
being ignored. Provider-specific options are deliberately absent from manifest version
1.

```powershell
ved voices --json
ved voices --provider voicevox --json
```

`voices` returns available provider/voice keys, display names, and languages when the
provider exposes them. Provider availability is also summarized by `doctor`.

Generated audio is normalized to PCM WAV at 48 kHz stereo and stored under
`.ved/cache/tts/<sha256>.wav`. The hash covers canonical manifest TTS fields, including
the selected provider and voice. The same request reuses the file without a provider
call. The project stores only the resulting media path and probe; provider details remain
reproducible in the manifest. Delete the matching immutable cache entry when an engine
upgrade must deliberately regenerate identical request settings.

Only loopback TTS endpoints are accepted in Phase 3. Remote endpoints and credential
management remain out of scope.

## Track-based ducking

Ducking is schedule-based and deterministic. It does not inspect signal amplitude. A
non-muted, non-zero-volume clip on any `trigger_track` contributes its resolved audible
interval. The compiler takes the union of those intervals and applies an envelope to
every independent audio clip on `target_track`.

For a trigger interval `[S, E]`, reduction `D`, attack `A`, and release `R`, the target
gain moves linearly from `1` to `10^(-D/20)` during `[S-A, S]`, remains reduced through
`[S, E]`, and returns linearly to `1` during `[E, E+R]`. Negative attack start is clamped
to zero. Overlapping trigger intervals and ramps use the lowest gain, never multiplied
reductions.

This intentionally keeps BGM reduced through silence inside one narration file. Agents
that need the BGM to rise during a long pause should split the narration into separate
scheduled clips. The rule affects only independent audio with the exact target track;
video clip audio is unchanged.

Human-oriented commands mirror manifest behavior:

```powershell
ved duck-add edit.json --key voice-over-bgm --target-track bgm --trigger-track narration --reduction-db 12 --attack 0.12 --release 0.35
ved duck-set edit.json --ducking d1 --reduction-db 9
ved duck-remove edit.json --ducking d1
```

`--trigger-track` is repeatable. `duck-set` requires at least one changed field and
replaces the complete trigger-track list when any trigger selector is supplied. These
commands use normal project history and the compact output contract.

Range preview and export compile ducking from the full project intervals and then
intersect the resulting envelope with the requested range. Starting a preview during an
attack, reduction, or release therefore produces the same gain it would have at that
point in a full render.

## Safe iterative rendering

Render-like commands accept explicit atomic replacement:

```powershell
ved render edit.json preview.mp4 --overwrite
```

Without `--overwrite`, existing-output rejection remains unchanged. With it, rendering
targets a same-directory temporary file. The old output is replaced only after FFmpeg
success, output probing, flush, and close. A failed render leaves the prior output
untouched. An output path that resolves to any input media remains forbidden.

`export-range` follows the same contract. `frame` may use `--overwrite` but does not need
post-render media probing.

## Transaction, history, and path rules

- `audio-sync`, multi-media refresh, and speed-with-retiming each create at most one undo
  snapshot.
- A semantic no-op creates no snapshot and does not rewrite project JSON.
- Generated IDs are allocated only after all external work and validation succeeds, so
  a failed transaction consumes no IDs.
- Manifest and media paths support Unicode and spaces and never pass through a shell.
- Canonical project serialization remains deterministic and rejects unknown fields.
- Batch operations report warnings in their result envelope rather than printing extra
  advisory lines.

## Delivery order

1. Agent I/O contract, arbitrary canvas, `media-refresh`, `audio-speed`, and atomic
   `--overwrite`.
2. Schema v3 audio identity/timing fields, `audio-sync`, source-linked time expressions,
   looping, hold items, track-based ducking, and selected retiming.
3. `voices`, local TTS providers, and the immutable TTS cache.

Stages 1 and 2 provide the main token reduction for workflows using prepared audio.
Stage 3 removes the remaining external speech-generation and probing round trips.

## Deferred follow-ups

The Phase 3 surface is sufficient for a first complete rough-cut-to-voice workflow.
The following remain useful but are intentionally deferred until real projects show that
volume/fade controls and range preview are insufficient:

- LUFS analysis and cached loudness normalization;
- automatic scene/caption detection and speech-to-text alignment;
- waveform or marker export for external editors;
- transitions, motion effects, and music or stock-audio discovery;
- remote TTS providers and secret management.

These features improve mix quality or automation depth, but none removes a structural
blocker left in the Phase 3 workflow. They should not delay compact output, idempotent
audio synchronization, source-linked timing, holds, or safe iterative rendering.

These candidates were subsequently assigned to the fixed Phase 4 assisted-production
scope in [`design-phase-4.md`](design-phase-4.md). This section remains the historical
Phase 3 deferral boundary rather than the current roadmap.

## Required acceptance coverage

- Version 2 projects migrate to version 3 without changing rendered output.
- Repeating an identical `audio-sync` returns `changed = false`, preserves IDs, and adds
  no history entry.
- One invalid entry makes a 20-entry synchronization leave project and history bytes
  unchanged.
- Source-linked times resolve correctly for trimmed, inserted, and sped-up clips.
- Adding an interior hold splits video deterministically, emits silence for base audio,
  preserves the visible frame for the requested duration, and keeps the insertion cue
  fixed while selected later cues shift.
- Boundary holds select the documented first or last complete frame. Repeated renders
  use the persisted `freeze_at` without recalculating it.
- Growing, shrinking, range-shortening, and removing a hold preserve stable IDs and
  apply the documented positive or negative retime mapping atomically.
- The retime mapping handles points before, inside, at the end of, and after an edited
  clip; text/image intervals map both endpoints while natural audio length is preserved.
- Audio tempo factors remain within FFmpeg's supported range for rates `0.05..=20.0`.
- Loop-to-timeline audio ends exactly at the current video duration and fades in
  completed-timeline time.
- Ducking computes the union of audible trigger intervals, applies the documented
  attack/reduction/release envelope to only the target track, and produces identical
  gain in full and range renders.
- Muted and zero-volume trigger clips do not duck. Overlapping triggers use the lowest
  gain without multiplying reductions.
- Refreshing a replaced file updates probe data; an incompatible duration or kind change
  fails atomically with stable conflict details.
- JSON success and handled-error modes each emit exactly one parseable value on the
  documented stream, with no FFmpeg progress mixed into stdout.
- `render --overwrite` preserves the old output after a forced FFmpeg failure and
  atomically replaces it after success.
- Japanese text and paths containing spaces, parentheses, quotes, and percent signs work
  in manifests and cached TTS media.
## TTS ownership and provider review

Phase 3 preserves the existing pipeline:

```text
Project -> Validated Project -> ResolvedTimeline -> RenderPlan -> FFmpeg arguments
```

TTS does not introduce a render path. Synthesis is a materialization step before the normal
compiler: TTS metadata identifies a deterministic cached WAV; that WAV is registered as ordinary
audio media and referenced by an ordinary `AudioClip`. `play`, `export-range`, and `render` then
compile and mix it exactly like any other independent audio. `frame` continues to compile through
the same timeline logic but does not need audible output.

## Schema and ownership

Phase 3 increments the project schema to version 3. Versions 1 and 2 are migrated in memory by
adding an empty `voice_clips` array; existing media/timeline/overlay/audio data is unchanged.

A `VoiceClip` owns only synthesis identity and the link to its normal audio clip:

```json
{
  "id": "v1",
  "provider": "voicevox",
  "voice": "3",
  "text": "でもちゃんと先を読む。",
  "speed": 1.0,
  "pitch": 0.0,
  "endpoint": "http://127.0.0.1:50021",
  "engine_identity": "voicevox:0.24.1",
  "cache_key": "...",
  "audio_clip_id": "a2"
}
```

`VoiceClip` intentionally does not persist timeline start or playback volume. Those values have a
single source of truth in the referenced `AudioClip`. The referenced `AudioClip` owns `start`,
`volume`, source range, mute and fades, and points to generated WAV media. Therefore project JSON
still exposes all requested TTS information without storing timing twice.

The generated WAV is represented by an ordinary `Media { kind: audio }`, whose probed duration is
the source of truth. TTS duration is never estimated from text.

## Provider abstraction

A small `TtsProvider` trait exposes only the operations Phase 3 needs: provider identity, voice
listing, synthesis, default endpoint, and parameter validation. `VoicevoxProvider` and
`AivisSpeechProvider` implement it independently. Shared HTTP plumbing is kept below the provider
boundary, while response interpretation and query mutation remain provider-owned so API drift in
one provider cannot silently change the other.

Both providers use local defaults only:

- VOICEVOX: `http://127.0.0.1:50021`
- AivisSpeech: `http://127.0.0.1:10101`

A caller may override the endpoint explicitly. No cloud fallback, telemetry, shell-based HTTP,
or updater is added.

VOICEVOX and AivisSpeech both use `GET /speakers`, `GET /version`, `POST /audio_query`, and
`POST /synthesis`, but they are not treated as interchangeable. Each provider validates and edits
its own `AudioQuery`. Phase 3 exposes `speed` and `pitch`; playback `volume` remains an AudioClip
property and is deliberately not injected into provider synthesis.

## Cache

The cache root is `<project-dir>/.ved/cache/tts/`. A SHA-256 cache key is computed from a canonical
serialization of:

- provider
- provider/engine identity from the engine version endpoint
- voice/style id
- text
- speed
- pitch
- provider-specific synthesis-affecting parameters introduced by this implementation

It deliberately excludes timeline position, playback volume, video clips, overlays, BGM, canvas,
and output settings.

A cache hit is accepted only after WAV validation. A miss or invalid cache is synthesized to a
unique temporary file in the cache directory, flushed, validated using FFprobe, and atomically
renamed to `<key>.wav`. A failed synthesis or failed validation never leaves the target cache path
as a valid entry. Removing a voice clip removes only project references, never the cache file.

## Commands and history

`voice-add`, `voice-set`, and `voice-remove` use the existing whole-project history mechanism.
Their project changes are therefore undoable/redoable and a new edit after undo clears the redo
branch exactly as in Phase 2. Cache files are not part of history, so undo/redo reuses an existing
cache instead of copying or regenerating it.

`voice-set --at` and `voice-set --volume` update only the linked AudioClip and do not synthesize.
Changes to text, voice, speed or pitch derive a new cache key and synthesize only on cache miss.

## Diagnostics and failures

`doctor` keeps FFmpeg diagnostics independent from TTS availability and adds provider-level checks
for the configured local default endpoints. An unavailable TTS engine is reported but does not make
`doctor` or unrelated editing commands fail.

Unsupported provider names are rejected by CLI parsing. Provider/cache failures are classified as
engine not running / connection failure, invalid voice, invalid parameter, synthesis failure,
invalid returned audio, cache write failure, and cache validation failure. Normal errors remain
concise; verbose provider diagnostics are not dumped by default.

## Security boundary

All HTTP is sent through a Rust HTTP client with URL/query encoding. User text is never interpolated
into a shell command. FFmpeg/FFprobe continue to use `std::process::Command` with discrete arguments.
No `cmd /C`, PowerShell, `sh -c`, curl, or shell execution is introduced.
