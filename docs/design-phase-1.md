# Phase 1 design review

This document fixes the contracts required to implement Phase 1. Features assigned to
later phases are deliberately absent from the data model and command surface.

## Scope

Phase 1 supports `new`, `doctor`, `import`, `add`, `trim`, `insert`, `remove`,
`describe`, `render`, `play`, `frame`, and `export-range`. It edits video clips together,
keeps each video's own audio, supplies silence for video without audio, and renders a
uniform canvas through FFmpeg.

Speed changes, overlays, separate audio tracks, history, TTS, transitions, transforms,
and raw FFmpeg filters remain design topics for later phases. Their speculative fields
are not written to version 1 project files.

The original end-to-end example includes `speed`, `text-add`, `image-add`, and
`voice-add`, so it describes the eventual v1 rather than Phase 1 acceptance. Phase 1 is
complete when the same flow works through clip edits, range preview, frame capture, and
MP4 export without those commands.

## Canonical project JSON

`project.json` is editing intent, rather than a cached render graph. It has four facts:

- schema version and canvas;
- imported media, with stable `mN` IDs and immutable probe metadata;
- ordered clips, with stable `cN` IDs and source ranges;
- clip audio policy (`mute` and `volume`), set by `--mute`, `--video-only`, and
  `--volume`, then omitted when unchanged.

There is no stored timeline start or duration. Timeline starts are derived by summing
source ranges in array order. That makes gapless ripple editing the only representation
of a given sequence and keeps Git diffs small. Media paths are project-relative with `/`
separators when the media is under the project directory; external media uses an
absolute path. Unknown JSON fields are rejected so a typo cannot silently change intent.

All writes use a same-directory temporary file, flush it to disk, and replace the
destination. On Windows the replacement uses `MoveFileExW` with replace and write-through
flags so an existing project is never truncated in place.

## Timeline semantics

All edit positions use seconds as finite non-negative decimal numbers.

- `insert --at` uses completed-timeline time. At a clip interior it keeps the left side's
  ID, assigns the inserted clip the next ID, and assigns the right side another ID.
- `remove --from --to` uses the half-open completed-timeline range `[from, to)`. It trims,
  drops, or splits intersecting clips, then array order provides ripple behavior.
- `trim --clip --in/--out` uses source-media time because it modifies a named clip's
  source range. Later clips ripple automatically.
- `add` and `insert` accept an imported media ID or its stored path. Phase 1 rejects image
  and audio media as timeline clips, although `import` records them for future phases.

IDs are identities, not positions. Moving through edits never renumbers an existing clip.
New numeric IDs are allocated above the greatest ID currently present.

## Compilation boundary

The data flow is one way:

```text
CLI mutation -> validated Project -> ResolvedTimeline -> RenderPlan -> FFmpeg arguments
```

`ResolvedTimeline` assigns completed start/end times. The compiler intersects it with an
optional preview/export range and produces source slices in a `RenderPlan`. This plan has
no FFmpeg filter syntax. The FFmpeg adapter alone creates process arguments and filter
labels.

`render`, `play`, `frame`, and `export-range` share this compiler. Range operations first
intersect clips, then seek each source input directly. Checking a late five-second range
does not process the timeline before that range. Process execution uses
`std::process::Command`; no shell parses filenames or generated filters.

## Known Phase 1 limits

The canvas policy is contain-and-letterbox with square pixels and a fixed project FPS.
Rendering uses CPU `libx264` plus AAC for predictable availability. Existing output files
are rejected. Hardware encoder selection, configurable fit/crop policy, migrations,
machine-readable `info`, and media relocation are decisions to make before Phase 2.
