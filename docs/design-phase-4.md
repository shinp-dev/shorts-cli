# Phase 4 assisted production design

This document fixes the Phase 4 contract for an assisted production workflow in which
a human and a capable director model decide story and taste, while a lower-cost executor
model performs deterministic analysis, editing, rendering, and quality checks.

The intended loop is:

```text
human + director -> bounded edit brief -> executor -> preview + review bundle
       ^                                                   |
       +---------------- human review/share ---------------+
```

`ved` remains model-neutral. It does not call, select, or orchestrate a director or
executor model. In the expected deployment, GPT-5.6 Sol may fill the director role and
GPT-5.6 Luna may fill the executor role, but no project or artifact schema contains a
model name. Human transfer between chats remains explicit.

Phase 4 preserves the compilation boundary established in earlier phases:

```text
CLI mutation -> validated Project -> ResolvedTimeline -> RenderPlan -> FFmpeg arguments
```

Read-only analysis produces versioned artifacts and immutable caches outside the
project. It never writes analysis payloads or provider responses into editing intent.

## Token-efficiency objective

Phase 4 treats expensive director context as a constrained resource. The reference
benchmark starts from a voice-complete project and performs three finishing rounds that
adjust timing, mix, effects, or wording. Compared with completing the same benchmark in
the director chat alone, the assisted workflow targets:

- at least 70% fewer director-model text tokens;
- at least 80% of tool calls, analysis, render waiting, retries, and QA handled in the
  executor task;
- no raw logs, full project JSON, full transcripts, waveform arrays, provider payloads,
  or unchanged item lists sent to the director;
- per review, one preview, one default report no larger than 2 KiB, and at most ten
  unresolved decision points;
- no repeated README, schema, or help ingestion after executor bootstrap.

These are workload acceptance targets, not a guarantee about an account usage-window
percentage. Director and executor tokens are recorded separately so savings are not
claimed by merely moving verbose context to the executor. If a comparable director-only
finish consumed 10% of a usage window, Phase 4 is designed to move most execution away
from that model; the measured result still depends on project length and review count.

## Committed scope

Phase 4 includes all of the following:

1. EBU R128 loudness analysis, cached two-pass normalization, and final-mix checks.
2. Speech transcription and transcript-anchored narration alignment.
3. Waveform images and portable marker export.
4. Duration-preserving preset transitions and motion effects.
5. Local and provider-based music or sound-effect discovery with explicit acquisition.
6. Remote TTS adapters integrated with the existing `audio-sync` cache workflow.
7. A review-bundle command that returns a preview, storyboard, checks, and concise
   handoff notes for human/director review.
8. Machine-readable discovery and compact project inspection so an executor does not
   need to ingest the README or raw project JSON for routine work.

Phase 4 is released as `ved` 0.4.x and increments the project schema from version 3 to
version 4 only for visual-effect and normalization provenance fields. Transcripts,
waveforms, search results, alignment candidates, and review bundles have independent
artifact schema versions.

## Explicit non-goals

Phase 4 does not include:

- autonomous story, script, or clip-order decisions;
- unrestricted FFmpeg filter expressions or arbitrary effect graphs;
- semantic music selection without human/director review;
- automatic acceptance of a new license, purchase, or paid API charge;
- automatic upload, publishing, chat attachment, or generation of public share links;
- background network access without an explicit network permission flag;
- face recognition, speaker identity inference, or emotion/personality profiling;
- a general operation DSL or embedded multi-model coordinator;
- destructive source-media rewriting.

True overlap dissolves, arbitrary keyframes, text animation, automatic scene rewriting,
and direct timeline exchange with a particular NLE remain deferred. Phase 4 transitions
are intentionally duration-preserving so adding polish cannot silently move narration,
markers, or approved story beats.

## Executor bootstrap contract

Two commands are the normal first calls for a fresh executor task:

```powershell
ved capabilities --json
ved inspect edit.json --json
```

`capabilities` is read-only and returns at most 4 KiB by default. Its stable fields are:

```json
{
  "ok": true,
  "command": "capabilities",
  "tool_version": "0.4.0",
  "project_versions": { "read": [3, 4], "write": 4 },
  "artifact_versions": {
    "work_order": 1,
    "transcript": 1,
    "alignment": 1,
    "markers": 1,
    "visual_manifest": 1,
    "review_notes": 1,
    "review_bundle": 1
  },
  "features": [
    "audio_sync",
    "loudness",
    "transcription",
    "alignment",
    "waveform",
    "markers",
    "visual_sync",
    "asset_search",
    "remote_tts",
    "review_bundle"
  ],
  "network_default": "deny"
}
```

`--schema NAME` returns one requested JSON Schema. Full schemas are never included in
the default result. Unknown schema names fail with stable code `schema_not_found`.

`inspect` returns the compact IDs and coordinates needed for editing: project hash,
duration, timeline items with completed and source ranges, overlay IDs, audio IDs with
`(track, key)`, transition/effect keys, and cached-analysis status. It omits media probe
payloads and long text by default. Repeated `--section timeline|audio|visual|analysis`
selectors restrict output. `--verbose` adds complete item data.

The default compact inspection must remain below 16 KiB for a project with 100 timeline
items and 100 audio items. Long text is returned as a length and SHA-256 plus a maximum
80-character preview.

### Director-to-executor work order

A director may hand the executor an optional compact work-order artifact:

```json
{
  "version": 1,
  "project_hash": "sha256:...",
  "objective": "音声完成版を自然なテンポで仕上げる",
  "authorized": ["read_only", "derived_artifact", "project_mutation"],
  "changes": [
    { "target": "h1", "instruction": "結果表示を約2秒確保" },
    { "target": "narration", "instruction": "音量をspeechプロファイルへ統一" }
  ],
  "constraints": ["台詞とクリップ順は変更しない"],
  "review_focus": ["12秒以降の間", "最後の効果音"]
}
```

This is not an executable DSL. The executor translates it into documented `ved`
commands and remains responsible for validation and checks. Before work begins,
`ved brief-check edit.json brief.json --json` validates the schema, size, project hash,
safety classes, and stable target IDs without mutating the project. A brief for an older
revision fails with `brief_stale`.

Objective is limited to 500 Unicode scalar values; each array to 20 entries; and every
instruction, constraint, or review-focus string to 200. Secrets, credentials, shell
commands, and arbitrary FFmpeg expressions are rejected. The brief carries approved
decisions and boundaries, not source media, analysis output, or conversation history.

All batch mutation commands accept `--dry-run`. Dry-run performs path resolution,
external-cache discovery, candidate construction, and validation but writes neither
project nor history. It returns the candidate project hash, counts, warnings, required
network operations, and required generated artifacts.

## Safety classes and executor autonomy

Every capability reported by `capabilities` has one safety class:

- `read_only`: inspect, check, provider discovery, transcript/cache reads;
- `derived_artifact`: transcription, waveform, marker export, preview, review bundle;
- `project_mutation`: normalization attachment, alignment acceptance, visual/audio sync;
- `network`: remote transcription, asset search/fetch, remote TTS;
- `external_publish`: uploads and publishing, which Phase 4 never implements.

An executor may perform the following without a new creative decision when the human or
director has already placed the project and goal in scope:

- run read-only checks and analysis;
- generate or reuse immutable caches;
- reapply an unchanged manifest;
- normalize to an already specified target;
- render previews and build review bundles;
- retry a failed local render with identical intent;
- surface low-confidence or ambiguous results as decision points.

The executor must escalate rather than infer permission before it:

- changes story text, clip inclusion, clip order, or approved narration wording;
- selects among materially different music or visual styles;
- accepts a license, purchases an asset, or begins a paid provider call not already
  authorized by the task;
- changes a confidence threshold to force an ambiguous alignment;
- replaces a source file, deletes media, or publishes a result.

No command interprets model confidence or prose. Safety is determined from the command,
its explicit flags, and validated artifact status.

## Project and artifact hashing

`project_hash` is lowercase SHA-256 over canonical version-4 project JSON. It changes
only when editing intent changes. Analysis and review artifacts store both
`project_hash` and the SHA-256 of every source file they depend on.

Cache keys cover:

- exact source content hash;
- selected source range and audio speed where relevant;
- operation settings;
- adapter and engine identity/version;
- artifact schema version.

File modification time is never the sole cache key. A stale artifact is never silently
used; commands either regenerate it or fail with `stale_artifact` and a suggested
read-only command. Canonical hashes allow a human and director to verify that a preview
matches the project version being discussed.

## Project schema version 4

Version 3 projects migrate in memory by adding empty `transitions` and `motions` arrays
and `normalization = null` to independent audio. Existing renders remain unchanged.
Writing occurs only after a successful mutation.

Independent audio gains optional normalization provenance:

```json
{
  "source_media_id": "m4",
  "target_lufs": -16.0,
  "max_true_peak_db": -1.5,
  "measured_lufs": -22.41,
  "measured_true_peak_db": -4.18,
  "applied_gain_db": 6.41,
  "cache_key": "sha256:..."
}
```

When present, the audio clip's `media_id` refers to the immutable normalized PCM cache
file and `source_media_id` refers to the original imported media. Re-normalization always
starts from `source_media_id`; it never normalizes an already normalized derivative.
The derived media must have the same selected duration within one audio sample. Missing
or mismatched cache media fails render with `normalized_cache_missing`.

The project root gains:

```json
{
  "transitions": [
    {
      "id": "x1",
      "key": "result-dip",
      "from_item": "c2",
      "to_item": "h1",
      "kind": "dip_black",
      "duration": 0.24,
      "audio": "unchanged"
    }
  ],
  "motions": [
    {
      "id": "e1",
      "key": "result-push",
      "target": { "type": "timeline", "id": "h1" },
      "start": 12.4,
      "end": 14.1,
      "preset": "zoom_in",
      "amount": 0.08,
      "easing": "ease_in_out"
    }
  ]
}
```

Transition IDs use `xN`; motion IDs use `eN`. Keys use the Phase 3 agent-key syntax and
are unique within their collection.

Transitions must connect two currently adjacent timeline items in their displayed order.
Supported Phase 4 kinds are `dip_black`, `dip_white`, `flash_white`, and `audio_fade`.
Visual transitions apply half of their duration before and half after the boundary.
They do not overlap or shorten source clips and never change completed-timeline duration.
`audio` is `unchanged` or `fade`; it cannot alter independent narration/BGM tracks.
Duration must be within `0.04..=2.0` seconds and each half must fit its adjacent item.

Supported motion presets are `zoom_in`, `zoom_out`, `pan_left`, `pan_right`, `pan_up`,
`pan_down`, and `ken_burns`. Targets may be timeline clips, holds, or image overlays.
Motion intervals use completed-timeline time. `amount` is finite within `0.0..=0.25`;
easing is `linear`, `ease_in`, `ease_out`, or `ease_in_out`. The compiler clamps crop
coordinates to valid pixels and uses deterministic expressions; arbitrary expressions
and keyframe arrays are rejected.

Existing speed/hold retiming gains `--retime-effects`. It maps motion endpoints using
the same documented timeline mapping as overlays. Transitions remain attached to item
IDs and are removed only when their adjacency ceases to exist; an operation that would
break adjacency fails unless `--remove-invalid-transitions` is explicit.

## Loudness analysis and normalization

Phase 4 uses FFmpeg's EBU R128/loudnorm measurements and reports integrated LUFS,
loudness range, true peak dBTP, and measured duration.

```powershell
ved loudness edit.json --audio a3 --json
ved loudness edit.json --track narration --json
ved loudness edit.json --mix --from 0 --to 23.4 --json
```

Exactly one of `--audio`, `--track`, or `--mix` is required. Audio analysis measures the
selected source after speed but before clip volume, ducking, and delay. Track and mix
analysis compile the audible result including volume, fades, ducking, normalization,
and timeline intersections. Results are cached and read-only.

Normalization is explicit:

```powershell
ved normalize edit.json --audio a3 --target-lufs -16 --true-peak -1.5
ved normalize edit.json --track narration --target-lufs -16 --true-peak -1.5
```

`--track` normalizes every non-looping clip independently in one transaction. Looping
BGM is normalized from one selected source cycle. Default target profiles are available
only through explicit names:

- `speech`: `-16 LUFS`, `-1.5 dBTP`;
- `music-bed`: `-20 LUFS`, `-1.5 dBTP`;
- `shorts-mix`: final-mix check target `-14 LUFS +/- 1`, maximum `-1 dBTP`.

`--profile` conflicts with numeric targets. There is no implicit default normalization.
The command performs two-pass analysis, writes normalized PCM WAV to
`.ved/cache/normalize/<sha256>.wav`, verifies duration and peak, then atomically updates
all selected clips while preserving `aN`, track, key, placement, fades, and speed.

`audio-sync` manifest version 2 accepts the same `normalize` object per entry and in
track defaults. Repeating a normalization or manifest is a no-op when source content and
settings are unchanged.

## Transcription and narration alignment

Transcription is an analysis operation, not a project mutation:

```powershell
ved transcribe edit.json m1 --provider local-whisper --out transcript.json
ved transcribe edit.json m1 --provider remote --out transcript.json --allow-network
```

The minimum required adapter is a configured local Whisper-compatible executable.
Remote adapters use the generic provider contract below. Audio is extracted as 16 kHz
mono PCM without changing source media. Transcript artifact version 1 contains:

```json
{
  "version": 1,
  "source": { "media_id": "m1", "sha256": "...", "duration": 19.4 },
  "provider": { "id": "local-whisper", "engine": "..." },
  "language": "ja",
  "segments": [
    {
      "id": "s1",
      "start": 1.12,
      "end": 2.84,
      "text": "...",
      "confidence": 0.93,
      "words": []
    }
  ]
}
```

Word timestamps are optional; segment timestamps are required. Raw provider payloads are
not retained unless `--keep-provider-payload` is explicitly requested. Default JSON
stdout returns only artifact path, source hash, language, segment count, and warnings.

Alignment matches explicit textual anchors to transcript positions:

```powershell
ved align edit.json cues.json --transcript transcript.json --out alignment.json
```

Cue manifest version 1 contains stable cue keys and an `anchor_text`; it may also specify
occurrence, source-media offset, and a preferred clip. It does not infer an anchor from
narration prose. Alignment output records one chosen candidate, alternatives, confidence,
and status `accepted`, `candidate`, `ambiguous`, or `unmatched`.

Automatic status is `accepted` only when confidence is at least `0.85`, the lead over the
second candidate is at least `0.10`, and the source position maps to exactly one project
clip. Otherwise no project mutation is permitted. A human/director choice is recorded
atomically with:

```powershell
ved align-accept alignment.json --cue hook --candidate 2
```

`audio-sync --alignment alignment.json` adds the time form
`{ "alignment": "hook", "offset": 0.05 }`. Only accepted, source-hash-current cues may
resolve. Rejection codes distinguish `alignment_ambiguous`, `alignment_unmatched`, and
`alignment_stale`.

Phase 4 alignment is lexical transcript anchoring. Semantic decisions such as choosing
which visual joke matches narration remain with the human/director.

## Waveforms and markers

Waveforms and markers are portable derived artifacts:

```powershell
ved waveform edit.json --track narration --out narration.png
ved markers edit.json --include timeline --include transcript:transcript.json --out markers.json
ved markers edit.json --include all --format vtt --out markers.vtt
```

Waveform output is a PNG with a deterministic width, channel mode, time ruler, and
optional markers. Default width is 1600 pixels; `--width` is within `320..=8192`.
`--peaks-out` optionally writes compact min/max peak data. The command never emits peak
arrays to normal stdout.

Marker sources are timeline boundaries, holds, audio keys, transition boundaries,
transcript segments, accepted alignment cues, and check issues. Supported formats are
versioned JSON, CSV, WebVTT, and FFmetadata. Marker IDs are stable hashes of source type,
source ID/key, and source time. Export does not add markers to the project or history.

JSON markers use completed-timeline seconds and may also contain source coordinates.
Long transcript text is truncated in review-oriented exports unless `--verbose` is used.

## Preset transitions and motions

Single-item commands mirror schema version 4:

```powershell
ved transition-add edit.json --key result-dip --from c2 --to h1 --kind dip-black --duration 0.24
ved transition-set edit.json --transition x1 --duration 0.3
ved transition-remove edit.json --transition x1
ved motion-add edit.json --key result-push --hold h1 --from 12.4 --to 14.1 --preset zoom-in --amount 0.08
ved motion-set edit.json --motion e1 --amount 0.05
ved motion-remove edit.json --motion e1
```

The canonical executor path is atomic synchronization:

```powershell
ved visual-sync edit.json visuals.json --replace-transitions --replace-motions
```

Visual manifest version 1 uses stable keys and accepts source-linked time expressions
for motion endpoints. It rejects arbitrary filters and unknown preset names. Repeating
an identical manifest is a no-op and creates no history. Every operation supports
`--dry-run`; output includes duration-preservation confirmation.

Effects are deliberately subtle by default, but defaults are manifest fields rather
than model-dependent interpretation. The CLI never accepts `--make-it-dynamic` or
similar subjective controls.

## Asset discovery and acquisition

Asset discovery is separated from acquisition and selection:

```powershell
ved asset-search --kind sfx --query "short victory chime" --limit 8 --out search.json --allow-network
ved asset-fetch search.json --candidate s3 --out assets/victory.wav --accept-license CC0 --allow-network
```

The always-available provider indexes explicitly configured local folders. Remote
catalogs use provider adapters; Phase 4 defines the adapter protocol but does not hardcode
a volatile third-party catalog. `providers --json` reports installed adapters,
capabilities, network requirement, authentication status, and whether results may incur
cost.

Search result artifact version 1 contains stable candidate IDs, provider ID, title,
kind, duration, tags, preview reference, source URL, license identifier, attribution
text, cost class, and provider metadata hash. Normal stdout returns at most ten compact
results. Search never downloads full media.

`asset-fetch` requires an explicit candidate and output. A license identifier from the
search artifact must match `--accept-license`; a paid result additionally requires
`--accept-cost` with the exact quoted amount and currency. Credentials and license
acceptance are never inferred from prior chat. The downloaded file is hash-verified,
probed, and stored with a sidecar provenance JSON before it may be imported.

Luna/executor may search and present candidates autonomously. Selection, new license
acceptance, and paid acquisition remain human/director decisions.

## Remote TTS provider contract

Remote TTS extends Phase 3 `audio-sync`; it is not a separate editing path. Provider
configuration contains a stable adapter ID, endpoint, supported options, timeout, and
credential environment-variable name. Secret values are read only from the environment
or OS credential store and are never accepted in manifests, project JSON, logs, cache
keys, or review bundles.

```json
{
  "key": "hook",
  "track": "narration",
  "at": 0.05,
  "tts": {
    "provider": "remote-tts",
    "model": "provider-model-id",
    "voice": "voice-id",
    "text": "弱いAIを作るほうが、難しい説。",
    "rate": 1.0,
    "pitch": 0.0,
    "intonation": 1.0
  }
}
```

A cache hit never requires network permission. A cache miss requires `--allow-network`;
without it, the transaction fails before contacting the provider. Requests use bounded
timeouts and at most three retry attempts for retryable transport/server failures.
Provider validation, TTS generation, PCM normalization, probing, and project mutation
remain one atomic `audio-sync` transaction. Immutable successful cache entries may remain
after a later project-validation failure.

Cache keys cover canonical TTS settings, adapter ID/version, provider model, voice, and
provider-reported engine/snapshot identity when available. Provider response IDs may be
stored in a non-secret provenance sidecar but not the project.

`voices --provider ID --allow-network --json` returns compact voices. Unsupported generic
rate, pitch, or intonation settings fail rather than being silently ignored.

## Quality checks

`check` is the executor's normal gate before review:

```powershell
ved check edit.json --profile shorts --json
```

It is read-only and returns `status = pass|warn|fail`, counts, and compact issues. Each
issue has a stable code, severity, related IDs, optional time range, evidence, and one
suggested command when a deterministic remedy exists. Phase 4 checks at least:

- missing or content-changed media;
- stale/missing normalization and analysis cache entries;
- final integrated loudness and true peak against the selected profile;
- clipped samples and unintended full-silence output;
- overlapping narration clips;
- narration ending past the video;
- ambiguous, unmatched, or stale alignment;
- invalid transition adjacency or insufficient duration;
- motion outside its target interval or unsafe crop bounds;
- unfetched remote asset references and missing attribution sidecars.

`check` never applies fixes. A suggested command may be executed by an executor only
under the autonomy boundary above. Creative warnings do not have auto-fix commands.

## Review bundle and human/director handoff

The primary completion command for an executor task is:

```powershell
ved review-build edit.json --out reviews --notes review-notes.json --profile shorts
ved review-build edit.json --out reviews --notes review-notes.json --since previous/review.json --overwrite
```

Review notes version 1 is deliberately small:

```json
{
  "version": 1,
  "summary": "結果表示を1.8秒延長し、ナレーションを再同期した。",
  "changes": [
    { "time": 10.2, "text": "h1を1.8秒追加" }
  ],
  "decisions": [
    {
      "time": 12.8,
      "question": "結果後の間を残すか",
      "options": ["現状維持", "0.5秒短縮"]
    }
  ]
}
```

Summary is limited to 500 Unicode scalar values, changes to 20 entries, decisions to 10,
and each text/question/option to 200. Unknown fields are rejected. Notes are untrusted
annotations; they never mutate the project or suppress check failures.

`review-build` performs `check`, renders a preview, creates a 12-frame storyboard, exports
compact markers, and writes `review.json` plus human-readable `review.md`. It builds into
a sibling temporary directory and atomically publishes the complete bundle. Failure
leaves an existing bundle untouched.

The bundle layout is:

```text
reviews/<project-stem>/<review-id>/
  preview.mp4
  storyboard.jpg
  markers.json
  review.json
  review.md
```

`review-id` is the first 12 hexadecimal characters of SHA-256 over project hash, render
range, preview profile, check profile, notes hash, and tool version. Repeating the same
command reuses a verified bundle and reports `changed = false`. `--since` validates the
previous review and adds stable-ID change counts; it does not ask a model to summarize.

Default preview is H.264/AAC, maximum 720 pixels on the long edge, with the project's
aspect ratio and complete duration. The storyboard uses project time labels. Review JSON
contains project/review hashes, relative artifact paths, duration, changes, checks,
markers, loudness summary, alignment status, and supplied notes.

Default `review.md` is at most 2 KiB and contains only revision identity, a change
summary, changed time intervals, warning/failing checks, and unresolved decisions. Full
evidence, transcripts, and marker data are linked as artifacts rather than inlined.
`--verbose` may create a separate `review-full.md`; it never expands the default report
sent back to the director.

Compact stdout is at most 1 KiB:

```json
{
  "ok": true,
  "command": "review-build",
  "changed": true,
  "review_id": "a93c71e502d4",
  "project_hash": "sha256:...",
  "video": "D:/project/reviews/demo/a93c71e502d4/preview.mp4",
  "report": "D:/project/reviews/demo/a93c71e502d4/review.md",
  "storyboard": "D:/project/reviews/demo/a93c71e502d4/storyboard.jpg",
  "checks": { "fail": 0, "warn": 2 },
  "decisions": 1
}
```

Paths are absolute so an executor can emit clickable local links. They are not public
URLs and are not assumed accessible from a separate chat. The human transfers the actual
video file and preferably `review.md` to the director chat. `ved` never uploads either.

## Provider and network rules

Provider adapters are subprocesses using newline-delimited JSON over stdin/stdout. They
are discovered from explicit configuration, never from the current directory. Requests
contain operation, schema version, non-secret settings, and input paths; responses reject
unknown fields. Adapter stderr is captured and never contaminates JSON stdout.

Every network-capable command requires `--allow-network` on a cache miss. Redirects to a
different origin are rejected unless the provider configuration allowlists that origin.
Downloads have configured byte/time limits, write to temporary files, and are probed
before atomic publication. Logs redact authorization headers, query credentials, and
provider response bodies by default.

Phase 4 configuration may store endpoints, default provider IDs, local model paths, and
preapproved cost ceilings. Secrets remain outside project-local files. Copying a project
therefore cannot copy credentials or silently authorize spending.

## Output, transaction, and compatibility rules

- Phase 3 `--json`, `--quiet`, `--verbose`, atomic overwrite, and error-stream contracts
  remain unchanged.
- Default mutation results remain at most 1 KiB; searches at most 8 KiB, inspections at
  most 16 KiB, and schemas/artifacts require an explicit output or `--verbose`.
- Read-only analysis may write immutable cache entries but never history.
- Every multi-item project mutation produces zero or one history snapshot.
- A semantic no-op does not rewrite project JSON or create history.
- Artifact paths resolve relative to their artifact file; project media paths retain
  existing project-relative behavior.
- Unknown project, manifest, artifact, provider, and review-note fields are rejected.
- All generated outputs support Unicode, spaces, parentheses, quotes, and percent signs.
- Version 3 projects render identically after in-memory migration to version 4.
- Undo/redo restores project intent, not disposable analysis caches or review bundles.

## Delivery order

1. Executor foundation: `capabilities`, compact `inspect`, work-order validation,
   `--dry-run` for sync commands, project hashing, `check`, and atomic `review-build`.
2. Audio QA: loudness analysis, normalization provenance/cache, final-mix checks,
   waveform, and marker artifacts.
3. Speech assistance: transcription adapters, transcript cache, alignment candidates,
   explicit acceptance, and `audio-sync --alignment`.
4. Visual polish: schema v4 transitions/motions, individual commands, `visual-sync`,
   duration-preserving compiler support, and review rendering.
5. External providers: provider protocol, network permission boundary, remote TTS, local
   asset catalog, remote search, and explicit asset acquisition.

Each delivery stage must leave the project readable by later 0.4.x builds and preserve
all earlier acceptance tests. Network integrations are tested with deterministic mock
adapters; live credentials and third-party availability are never required by CI.

## Required acceptance coverage

- `capabilities` and compact `inspect` are stable, bounded, and sufficient to construct
  audio/visual manifests without reading raw project JSON.
- Work-order validation rejects stale hashes, excessive text, unknown stable IDs,
  unauthorized safety classes, secrets, and executable payloads without mutation.
- Version 3 migration adds empty Phase 4 fields and produces byte-equivalent audio/video
  output before any Phase 4 edit.
- Identical source/settings reuse loudness, normalization, transcript, waveform, and
  review caches without invoking FFmpeg or a provider.
- Normalization is two-pass, starts from original media, preserves duration/IDs/timing,
  and meets target tolerance without exceeding true-peak maximum.
- Track normalization is all-or-nothing for at least 20 clips and creates one history
  snapshot; one invalid clip leaves project/history bytes unchanged.
- Full/range loudness analysis includes volume, fades, ducking, and normalization.
- Transcript timestamps map through trims, insertions, speed changes, holds, and source-
  linked clips without manual timeline arithmetic.
- Accepted alignment resolves deterministically; ambiguous, unmatched, and stale cues
  cannot mutate a project until explicitly accepted.
- Waveform and marker outputs are deterministic and never place peak arrays in compact
  stdout.
- Transitions preserve project duration and reject non-adjacent targets or insufficient
  adjacent duration atomically.
- Motion presets remain within crop bounds, preserve timing, and retime only when
  explicitly selected.
- Repeating `visual-sync` preserves IDs, creates no history, and reports `changed=false`.
- A network cache miss without `--allow-network` makes no request and changes no project,
  history, or published artifact.
- Remote TTS retry/timeout failures preserve project/history; a verified cache hit works
  offline and does not read credentials.
- Asset search never downloads media; fetch rejects mismatched license/cost acceptance,
  hash mismatch, oversized content, redirect violations, and unsupported media.
- Provider secrets never appear in JSON output, errors, manifests, projects, cache keys,
  provenance, review bundles, or test snapshots.
- `check` emits stable issue codes and never mutates project state.
- `review-build` atomically produces a playable preview, storyboard, markers, JSON, and
  Markdown; a forced render/check failure preserves the previous complete bundle.
- Repeating a review build returns the same review ID and `changed=false`.
- Review stdout contains usable absolute local paths under 1 KiB, and the report clearly
  states that a separate chat requires transfer of the actual files.
- On the fixed three-review finishing benchmark, director-model text tokens are at least
  70% lower than the director-only baseline with the same or better required-check
  results. Fixtures, prompts, model IDs, reasoning settings, tool version, review count,
  and separate director/executor token accounting are retained with the benchmark.
- Japanese text and special-character paths work through transcription, normalization,
  provider adapters, search artifacts, effects, and review bundles.

## Completion definition

Phase 4 is complete when a human/director can provide a bounded brief, an executor can
discover the tool without reading repository documentation, perform authorized analysis
and edits, and return one verified preview plus a concise review report. The human must
be able to attach those artifacts to a separate director chat and continue discussion
without reconstructing project state from memory.

The tool is not expected to decide whether the story is good. It is expected to make
every approved technical edit repeatable, reviewable, and cheap enough to delegate.
