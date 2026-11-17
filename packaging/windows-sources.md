# Windows third-party source access

Official Windows packages use the exact archives and DLL hashes in
`windows-dependencies.json`. The matching CuePool release provides
`cuepool-windows-sources.zip` beside the MSI and portable ZIP, with source
snapshots, build scripts, patches and source-download indexes. Source downloads
are free and do not require the binary package. Keep this source artifact with
archived installers.

## FFmpeg

The shared GPL build is BtbN `n8.0.1-66-g27b8d1a017`, built on 2026-02-28:

- [Binary release](https://github.com/BtbN/FFmpeg-Builds/releases/tag/autobuild-2026-02-28-12-59)
- [Exact FFmpeg source](https://github.com/FFmpeg/FFmpeg/tree/27b8d1a017de62b51c0f3a039327b5cc2d7ea13f)
- [Build scripts and patches](https://github.com/BtbN/FFmpeg-Builds/tree/c482a075076c29edf6dd9c5a7c9d3113d0babed6)
- [External dependency recipes and source revisions](https://github.com/BtbN/FFmpeg-Builds/tree/c482a075076c29edf6dd9c5a7c9d3113d0babed6/scripts.d)

The source artifact includes the first two source archives above, an index of
the recipes' repository/revision pairs, and the rav1e Cargo lockfile and crate
source index. Each recipe contains its configure arguments and patches. The
included `download.sh` retrieves the dependency sources into `.cache/downloads`,
including explicitly requested submodules and shaderc's pinned `DEPS` inputs.
Use the included README's `win64 gpl-shared 8.0` build commands. To rebuild the
same FFmpeg revision, check out the full FFmpeg commit above after the clone in
`build.sh`; its original `release/8.0` branch continues to move.

The build recipes use general-purpose tools and base images that can change;
these sources support rebuilding and modification, not a promise of identical
binary bytes. In particular, rav1e's recipe updates its `cc` build helper.
Its checked-in Cargo.lock identifies the library source dependencies; the
recipe retains the helper update instruction. Sources under recursive git
submodules are identified by their parent commits, not by moving branch tips.

The DLLs use the Windows Universal CRT, supplied by supported Windows versions.
CuePool's Windows runtime still requires Windows 10 22H2 or newer. See
`FFmpeg-LICENSE.txt` for the bundled GPL v3 license.

## CuePool, Rust dependencies and native support

The source artifact records the exact CuePool commit and includes its tracked
source, `Cargo.lock`, and `RUST-SOURCES.json`. Each registry entry has a direct
`.crate` source-download URL and the Cargo.lock SHA256. Git entries identify the
exact repository commit. `cargo vendor --locked --versioned-dirs vendor` in the
CuePool source checkout retrieves registry and git sources for offline builds;
retain the Cargo configuration printed by that command. The apriltag-sys crate
already contains AprilTag's native sources.

The artifact also preserves the actual runner's vcpkg pthreads port, patches,
license and vcpkg commit. Its portfile records the pthreads4w source filename
and SHA512; the source index supplies the download URL. This static native
dependency sits outside Cargo.lock.

ASIO SDK 2.3.4 source is included unchanged in the source artifact and is also
available from [Steinberg](https://download.steinberg.net/sdk_downloads/ASIO-SDK_2.3.4_2025-10-15.zip).
Its exact archive hash is in `windows-dependencies.json`; see
`Steinberg-ASIO-LICENSE.txt` for the GPL v3 / Steinberg agreement terms. Device
drivers are supplied separately by their manufacturers.

CuePool's source license files remain MIT / Apache-2.0. The packaged dependency
notices describe the additional terms applicable to the distributed binaries.
Microsoft's app-local Visual C++ runtime comes from the build tools' Redist
directory under [Microsoft's redistribution terms](https://learn.microsoft.com/en-us/cpp/windows/redistributing-visual-cpp-files).
