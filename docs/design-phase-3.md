# Phase 3 TTS design review

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
