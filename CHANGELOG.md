# Changelog

Version headings record source versions, not proof of publication. Confirmed
publications are recorded separately by annotated published/vX.Y.Z Git tags.

## [Unreleased]

## [0.14.0](https://github.com/kovvbojAV/cuePool/compare/v0.13.5...v0.14.0) - 2026-11-21

### Application features

- Play videos side by side on one output: each video cue has a layer (1–4) and a canvas region, and videos on different layers play together, frame-synced, each started and stopped as its own cue ([#63](https://github.com/kovvbojAV/cuePool/pull/63)).
- Loop a Group as one unit: its videos restart together when the longest ends, so clips of slightly different lengths stay in step ([#69](https://github.com/kovvbojAV/cuePool/pull/69)).
- Time Window trigger: play a cue between two times each day. It starts if CuePool comes up inside the window, fades out at the end, and a Stop holds until the next window ([#68](https://github.com/kovvbojAV/cuePool/pull/68)).
- Open a `.qproj` from Finder on macOS, and optionally reopen the last project on startup (Project Settings → This Computer). App settings are now saved as they change, not only on quit ([#70](https://github.com/kovvbojAV/cuePool/pull/70)).
- Decode NotchLC video on the GPU. It is faster, and its colours are now correct ([#64](https://github.com/kovvbojAV/cuePool/pull/64)).

### Application fixes

- macOS fullscreen outputs no longer crash CuePool when output windows are rebuilt. They stay on their own screens above other windows, hide the pointer, and re-enter fullscreen by themselves when the display stops pacing them ([#67](https://github.com/kovvbojAV/cuePool/pull/67)).
- Drop video frames when the GPU falls behind instead of queuing uploads until memory runs out, and draw opaque side-by-side layers without a composite pass ([#63](https://github.com/kovvbojAV/cuePool/pull/63)).

## [0.13.5](https://github.com/kovvbojAV/cuePool/compare/v0.13.4...v0.13.5) - 2026-09-24

### Application fixes

- *(cuepool)* keep cue list columns aligned regardless of cell content ([#52](https://github.com/kovvbojAV/cuePool/pull/52))

## [0.13.4](https://github.com/kovvbojAV/cuePool/compare/v0.13.3...v0.13.4) - 2026-09-24

### Application fixes

- keep control-only GO chains from starting the show clock ([#48](https://github.com/kovvbojAV/cuePool/pull/48))
- preserve independent fixture fades ([#47](https://github.com/kovvbojAV/cuePool/pull/47))

## [0.13.3](https://github.com/kovvbojAV/cuePool/compare/v0.13.2...v0.13.3) - 2026-09-24

### Application fixes

- Start a cue's WithLast cues when its timecode, hotkey, MIDI or wall-clock trigger fires it, as GO does ([#46](https://github.com/kovvbojAV/cuePool/pull/46)). A WithLast cue that also carries its own trigger can now fire twice; remove that trigger.

## [0.13.2] - 2026-09-22

### Packaging fixes

- publish Windows packages under kovvbojAV

## [0.13.1](https://github.com/kovvbojAV/cuePool/compare/v0.13.0...v0.13.1) - 2026-09-22

### Application fixes

- launch Windows releases without a console window

## [0.13.0](https://github.com/kovvbojAV/cuePool/compare/v0.12.3...v0.13.0) - 2026-09-22

### Application features

- Install CuePool on Windows with a versioned MSI or use the portable ZIP. Both include ASIO support and app-local runtime libraries; device drivers are installed separately.
- Check the source revision in Help → About and inspect changes offline from Help → Changes ([#31](https://github.com/kovvbojAV/cuePool/pull/31)). The `--version` command reports build identity without starting the GUI, audio or project loading ([#36](https://github.com/kovvbojAV/cuePool/pull/36)).
- Query master volume over OSC and receive change notifications; the included Nodel slider stays in sync ([#32](https://github.com/kovvbojAV/cuePool/pull/32)).

### Application fixes

- Keep a final Stop All cue from rearming the show clock ([#30](https://github.com/kovvbojAV/cuePool/pull/30)).
- Update the audio sample ring buffer to the version patched for RUSTSEC-2026-0293.

### Distribution

- Use [kovvbojAV/cuePool](https://github.com/kovvbojAV/cuePool) as the canonical source and release repository.
- Verify Windows runtime loading, installation, six-product legacy upgrades, failed-upgrade recovery and user-data preservation before publication.
- Pin the ASIO SDK and rebuild its cached native output when preparing Windows packages.
- Pin the FFmpeg SDK and include a checksummed Windows source archive with source snapshots, dependency indexes and native build recipes.
- Include macOS dependency notices in the app and publish matching source directions and exact Homebrew build recipes alongside the DMG.

## [0.12.3]

### Application

- Poll triggers without cloning the cue list; fire wall-clock cues once per target (#22).

### Build maintenance

- Prepare version 0.12.3. Its version tag does not establish a published release.
