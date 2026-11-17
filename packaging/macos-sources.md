# macOS dependency notices and source access

The matching [CuePool GitHub release](https://github.com/kovvbojAV/cuePool/releases)
provides `cuepool-macos-sources.zip` beside the DMG. Source downloads are free and
do not require the binary package. Preserve this archive alongside archived DMGs.
Inside CuePool.app, `Contents/Resources/ThirdParty` contains this guide,
`SOURCE.json`, dependency notices and the exact installed Homebrew build recipes.

`SOURCE.json` identifies the CuePool commit and every non-system dylib in the app,
including its original Mach-O build UUID and installed Homebrew version. The
collector follows the built executable's actual dependency closure; it does not
substitute today's Homebrew formula for an older installed version. Rewritten
load paths and code signatures change library bytes, so the original library
hashes describe the Homebrew inputs, not the signed files inside the DMG.

## FFmpeg and its supporting libraries

This package uses GPL v3-or-later FFmpeg, built with `--enable-gpl` and
`--enable-version3`. `FFMPEG-BUILD.txt` records the version and configuration
embedded in the actual FFmpeg library. The source ZIP includes its exact upstream
source archive, verified against the installed recipe's SHA256. These macOS
libraries can differ from the FFmpeg build in the Windows packages.

For each bundled Homebrew dependency, `homebrew/<name>/<version>/` preserves:

- Its installed license, copyright and author notices.
- `recipe.rb`: the exact build instructions, including patches and resources.
- `INSTALL_RECEIPT.json`: the installed build/compiler/options metadata.
- `formula.json`: metadata evaluated from that installed recipe.

`HOMEBREW-LICENSE.txt` preserves the license for the copied Homebrew recipes.

`SOURCE.json` lists each upstream source-download URL with its SHA256 or exact
Git commit. Download the archive and verify that hash; for a Git source, clone
the recorded repository and check out the recorded commit. The matching recipe
contains configure/build/install instructions and any additional resource or
patch URLs. These remain available through their upstream servers; retain local
copies if those servers become unavailable. A generic project homepage or a
moving branch is not a substitute for the recorded source.

Use the preserved Homebrew recipes and receipt options to rebuild dependencies,
then the CuePool source's build instructions. Homebrew's ordinary compiler and
build tools are not bundled here. The archive supports rebuilding and modifying
the libraries; it does not promise byte-for-byte identical binaries.

## CuePool and Rust dependencies

The source ZIP includes the tracked CuePool source at `cuepool_commit`, its
`Cargo.lock`, and `RUST-SOURCES.json`. Registry dependencies have direct `.crate`
source URLs and lockfile hashes; Git dependencies identify exact commits.
`cargo vendor --locked --versioned-dirs vendor` in that source checkout downloads
the Rust dependencies for offline builds; retain the Cargo configuration it
prints. The apriltag-sys crate contains its native AprilTag source. macOS uses
system pthreads and does not bundle the Windows ASIO SDK or pthreads library.

CuePool source files retain their MIT / Apache-2.0 licenses. Dependency notices
describe the additional terms for the distributed binaries. See FFmpeg's
[licensing guidance](https://ffmpeg.org/legal.html) and the included GPL v3 text;
GPL v3 section 6(d) permits corresponding source on another server when clear
directions accompany the binary. Source availability must be maintained.
