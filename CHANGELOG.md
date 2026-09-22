# Changelog

Version headings record source versions, not proof of publication. Confirmed
publications are recorded separately by annotated published/vX.Y.Z Git tags.

## [Unreleased]

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
