# Preparing and publishing CuePool

The canonical source and release repository is
[`kovvbojAV/cuePool`](https://github.com/kovvbojAV/cuePool).

CuePool has one product version in `[workspace.package]`, inherited by every
internal crate, one `CHANGELOG.md`, and product tags named `vX.Y.Z`. Release-plz
0.3.169 prepares changes entirely from Git; registry publication is disabled in
its configuration. We run its `update` and `release-pr` commands, never
`cargo publish` or `release-plz release`.

Always invoke it through `scripts/prepare-release.py`. That wrapper supplies an
ancestral version-tag checkout using the supported `--registry-manifest-path`
option. Despite that option name, the baseline is local Git source, never a
registry download. It fails if the baseline cannot be constructed or its version
does not match the tag. The tool's built-in `git_only = true` path runs
`cargo package` on old tags; CuePool's versionless private path dependencies and
fork-only wgpu features cannot be packaged as registry crates. The local-baseline
path avoids changing those historical manifests or compiling old application code.
The wrapper supplies original-manifest copies and the checked-in binary lockfile
in its temporary baseline only. It exposes legacy `publish=false` members in the
analysis manifests while retaining byte-for-byte historical `.orig` copies, so
the old private harness is compared at the tag rather than treated as a new crate.

Cargo manifests remain visible to release-plz's workspace analysis (including the
harness); the workflow's `publish = false` is the publication guard. Do not run raw
release-plz without the wrapper or introduce a registry/publishing fallback.

## Contributor conventions

Squash-merge PRs using a Conventional Commit title that describes the final change:

- `fix: ...` for application fixes: patch increment.
- `fix(deps): ...` for application dependency updates: patch increment.
- `feat: ...` for application features: minor increment, including during `0.x`.
- `feat!: ...` or another type with `!`, with a `BREAKING CHANGE:` explanation,
  for incompatible changes. Release-plz highlights these for review. In `0.x`,
  breaking changes advance the minor; do not describe that as a compatible patch.
- `ci: ...`, `docs: ...`, `build: ...` and `chore(deps): ...` for build tooling,
  GitHub Actions and documentation. These alone do not request a product release.

Review application dependency PRs so their squash title uses `fix(deps)`, not the
maintenance-only `chore(deps)` convention used by GitHub Actions updates. Do not
label application behavior changes as maintenance to suppress a release.

Write concise operator descriptions under `[Unreleased]` in `CHANGELOG.md` when
PR titles alone would not explain the impact. Generated release notes include
changes from all internal crates in the product changelog. Release preparation
only updates workspace lockfile versions (`dependencies_update = false`); it
must not refresh unrelated dependency versions. For root Cargo.toml/Cargo.lock
changes, release-plz generates a dependency summary rather than keeping the PR
title; add the operator impact to `[Unreleased]`. The exact generated dependency
messages are explicitly allowed by the release filter, and both root-only cases
are tested against the pinned tool.

## Release PR

The Release preparation workflow runs after pushes to main and completed main
Release runs, and can be retried with workflow_dispatch. The completion trigger
rechecks current main so changes merged while a candidate was awaiting its tag
are not forgotten. It checks out current main with complete history and
maintains a reviewable release-plz PR updating Cargo.toml, Cargo.lock and the
product changelog. CI checks that every crate still inherits the product version
and that its lockfile entry matches. An untagged version already merged into main
is an outstanding release candidate: preparation defers until it has passed
verification and been tagged, avoiding recursive version bumps. The wrapper skips
maintenance-only commit history before invoking release-plz, including documentation
inside crate directories; the pinned tool's release filter alone can otherwise
change only the workspace version for those commits. Before creating a PR, the
wrapper rehearses the update in an isolated clone and verifies exactly the three
product files change, all internal versions match, and external dependencies stay
locked.

Review the proposed increment and notes. For a minor release, rewrite the
operator-facing welcome copy in `crates/cuepool-gui/src/app/mod.rs` and set
`RELEASE_NOTES_VERSION` to that major.minor. The existing
`release_notes_match_the_release` test remains required. A patch release leaves
the constant and welcome copy alone, so it does not reopen the modal.
Add the welcome copy and matching constant to the release PR before merging.
Release-plz may replace that PR after human edits when more changes arrive;
reapply and recheck the copy in the final proposal. Do not advance the constant
on main while its product version still names the previous minor release.
Never bypass the failing notes gate.

## One publication owner

Only `.github/workflows/release.yml` publishes GitHub releases. Each main push
checks for an outstanding untagged product version; a pushed product tag also
starts the workflow. A source fix after failed verification can therefore recover
an untagged candidate without another version bump. Once tagged, that source is
immutable: subsequent fixes go through a new release PR. It validates the
version/changelog and runs the complete reusable CI suite at the exact candidate
SHA. After verification it creates the product tag, refusing to move an existing
tag. It then builds and packages that same SHA on macOS and Windows.

A read-only prerequisite gate runs even on artifacts-only rehearsals and failed
builds. The final publication job requires that gate to pass, including successful
verification and both platform jobs. It also requires exactly one nonempty DMG,
portable ZIP, MSI and source archive for each platform, validates
their basic container structure, and checks the ZIP contains the executable and
runtime DLLs. The Windows job additionally runs the portable executable with a
clean runtime path and exercises MSI installation, upgrade, uninstall and rollback
on its disposable hosted runner. Profile/project preservation and installed
payload hashes are checked; logs are retained as `windows-package-validation`.
Windows builds explicitly enable ASIO; production packages omit the test harness.
macOS packaging verifies the DMG and application signature, includes dependency
notices in the app, and checks that its source index covers every bundled dylib.

The publication script creates or reuses a **draft**, writes the exact changelog
notes and source attestation (rejecting conflicting source claims), uploads the packages and
SHA256SUMS, downloads each uploaded file and verifies its SHA-256, then makes the
draft public. A failed build, missing package, upload failure or readback mismatch
leaves publication blocked. Nothing is published by release preparation itself.

After confirmed publication the same workflow creates an annotated
`published/vX.Y.Z` receipt with the exact commit, release URL and publication time.
It never triggers another product release. These tags are publication metadata;
the product version and product tag remain unchanged. Local builds use receipts
offline as described in [build identity](build-identity.md).

A failed run can be rerun, or manually dispatched with the existing product tag
created by this automation. Older tags that predate these scripts require a new
release candidate; they do not satisfy this publication contract.
An existing tag is never moved; a draft is reused. A previously public release is
never overwritten: recovery downloads its packages and checksum manifest, checks
its source attestation, and repairs only a missing publication receipt. Therefore
retrying after publication does not depend on reproducing byte-identical binaries.
The workflow serializes release attempts with `queue: max` (up to 100 waiting
runs), so a later push does not replace a queued candidate. Interrupted runs can
leave a private draft or an unreceipted public release, both recoverable by retry.
An older retry is never marked Latest when a newer product version is public.

Manual dispatch **without** `release_tag` builds artifacts only. It creates no
product tag, release or receipt. Use this on an implementation branch to exercise
both packagers without publishing.

## GitHub setup and token behavior

Build identification is local: Cargo reads the workspace version and Git revision
without a GitHub account, token or network connection. A development build reports
how many commits it is beyond its version tag, its exact commit and local edits.
Rebuilding the same source keeps the same identity. Product versions change through
reviewed release PRs; they do not need to change for every identifiable build.

Release preparation uses release-plz with the repository's built-in `GITHUB_TOKEN`.
No custom GitHub App, private key, personal access token or crates.io token is needed.
The job requests Contents and Pull requests write permissions; other jobs keep
their existing permissions.

For automated release PRs, the repository owner must enable **Settings → Actions
→ General → Workflow permissions → Allow GitHub Actions to create and approve
pull requests** if the organisation policy permits it. The workflow declares its
required write permissions, so
there is no need to change the default token permission for all workflows.
After setup, run **Actions → Release preparation → Run workflow** on `main` to
create or refresh the proposal. If GitHub refuses PR creation and the organisation
policy prevents enabling the setting, use the maintainer procedure below.
Despite the setting's name, this workflow never approves or merges its own PR.

GitHub puts CI runs for PRs created or updated with `GITHUB_TOKEN` into an
approval-required state. A user with **write access** can select **Approve workflows
to run** in the PR merge box; admin access is not needed for that per-proposal step.
After the bot updates a release PR, approve the checks for its latest revision.
Wait for those checks, review the version/changelog and update minor-release welcome
copy before squash merging. Do not interpret pending or approval-required checks
as a pass. See [GitHub's token event rules](https://docs.github.com/en/actions/how-tos/write-workflows/choose-when-workflows-run/trigger-a-workflow#triggering-a-workflow-from-a-workflow).

### Maintainer release PR when bot PRs are disabled

The current organisation policy prevents GitHub Actions from creating PRs. A
maintainer can prepare the same proposal locally and open a normal PR using their
existing GitHub login. No policy change or additional repository secret is needed.
While that policy applies, disable only the **Release preparation** workflow in
Actions to avoid repeated bot-PR failures. Keep **CI** and **Release** enabled:
publication and receipt recovery do not depend on release preparation. Re-enable
the preparation workflow if automated PRs become permitted.
Install the pinned `release-plz 0.3.169`, then start from a clean checkout:

```sh
git fetch origin main --tags
git switch --create release/cuepool-next origin/main
python3 .github/scripts/release.py can-prepare
python3 scripts/prepare-release.py update
git diff -- Cargo.toml Cargo.lock CHANGELOG.md
```

Continue only if `can-prepare` reports `ready=true`. The wrapper enforces the same
Git baseline, version and dependency checks used by automation, but `update` makes
no remote changes. Review the proposed version and changelog. For a minor bump,
update the welcome copy and `RELEASE_NOTES_VERSION` before running the checks in
AGENTS.md. Commit the proposal, push the branch, and open a PR against `main` in
`kovvbojAV/cuePool`. Use a title such as `chore: release CuePool 0.13.0` with the
actual proposed version. Existing GitHub CLI authentication is sufficient for
`gh pr create --repo kovvbojAV/cuePool`; do not pass it to release-plz or create a
long-lived workflow credential.

For an installer-only fix outside the Cargo packages, ordinary `update` may find
no crate changes. Use the explicit local preparation mode instead:

```sh
python3 scripts/prepare-release.py update --packaging-patch
```

This mode requires a clean checkout, the ancestral current-version tag, and a
`fix:` commit that changes an installer or packaging input. It rejects application
files, Cargo manifests/lockfiles, unrelated paths, features and breaking changes
anywhere in the new first-parent history. Packaging documentation, tests and
release tooling may accompany the fix. It increments only the patch version,
prepares notes without changing earlier releases, and delegates workspace and
lockfile edits to `release-plz set-version` in a temporary clone. Only the verified
Cargo.toml, Cargo.lock and CHANGELOG.md are copied back; external dependencies and
all inherited crate versions remain checked. `release-pr --packaging-patch` is
unsupported: review the local proposal and open the maintainer PR as above.

The 0.3.169 tool pin is needed for upstream
[workspace-preserving set-version support](https://github.com/release-plz/release-plz/pull/3047).
Every internal package maps to the same root changelog for this command; only
`cuepool` enables changelog generation during ordinary `update`.

CI on a maintainer-created PR runs through the normal pull-request event. Review
and merge it after all required checks pass. The main push then starts the same
verified publication workflow described above. A green Release run that reports
`active=false` means the workspace still names an older existing tag; it is not
evidence that new installers were built. Do not move that tag or reuse its version
for changed source.

Publication uses the job-scoped GITHUB_TOKEN with Contents write. The verification
and packaging jobs continue in the **same release workflow** after it creates a
tag, so they do not depend on a second tag-triggered run. Publication receipts
use a different tag prefix and cannot trigger packaging. Repository tag rules must
allow this workflow to create `v*` and `published/v*` refs; it requires no direct
write or branch-protection bypass on main. Keep the release PR's CI checks required.

## Verification

Run the commands in AGENTS.md, `npm ci && npm test` in `mcp`, and:

```
python3 .github/scripts/release.py validate
python3 -m unittest discover -s .github/scripts -p 'test_*.py'
python3 scripts/test-release-policy.py
```

The policy test requires release-plz 0.3.169 on PATH (or `RELEASE_PLZ` pointing to
it). It exercises the production wrapper and real `release-plz update` in temporary
workspaces with the same crate names and inherited version: fix, feature, harness
feature, crate/root-manifest/lockfile dependencies, docs inside/outside crates,
CI-only and breaking changes. It checks every lockfile version, the single changelog and an
unrelated older locked dependency. It also exercises the explicit packaging patch
and its rejection guards against the real tool. It never creates a remote PR or publishes.

A passing Linux suite does not prove Windows or macOS packaging. If the Windows
AprilTag build still lacks pthread.h, its build job must fail and publication must
remain blocked. Resolve that platform prerequisite through its own reviewed change;
do not remove Windows from the release gate.

## Windows dependency sources

Windows builds use the BtbN FFmpeg 8.0-branch shared SDK pinned in
`packaging/windows-dependencies.json`. Keep headers and DLLs together: CuePool's
D3D12VA layout guard relies on this ABI. The dependency setup verifies the SDK
archives before restoring Cargo builds. Its cache key includes the dependency
manifest, setup action, hosted Windows image and SDK paths, so even fallback
restores cannot reuse native bindings from a different SDK or toolchain image.
Keep Windows Rust caching inside that action: ASIO can reuse stale generated
files even when its build script reruns. Packaging checks all seven DLL hashes
against the pin.

`cuepool-windows-sources.zip` is required for publication alongside the MSI,
portable ZIP and macOS DMG. The workflow creates it from the exact CuePool
checkout, verified upstream FFmpeg/ASIO archives, the matching BtbN build-script
snapshot, Cargo source indexes, and the actual vcpkg pthreads port. The release
checks its member hashes and CuePool commit, then includes it in `SHA256SUMS`
and the same remote readback check as the binaries. An artifacts-only rehearsal
produces it too. See [source access](../packaging/windows-sources.md).

When changing the FFmpeg pin, update the binary, seven DLL hashes, corresponding
FFmpeg source and BtbN build-script snapshots together. Verify the SDK's D3D12VA
layout, Windows loader and media playback before promotion. Preserve the source
asset as long as its binary release is offered; upstream BtbN monthly assets
have a two-year retention policy.

## macOS dependency sources

`cuepool-macos-sources.zip` is also required for publication and remote readback.
It records the libraries actually linked by the original Cargo binary and
compares that inventory with the dylibs copied into the app. Source directions,
installed license/notice files, exact Homebrew keg recipes and installation
receipts accompany CuePool's source snapshot and Rust dependency index.

Use installed keg metadata, not the current Homebrew formula API: the latest
formula may already describe a different version. Collect notices into the app
before signing and creating the DMG. The macOS source archive is separate from
the Windows archive because the platforms can ship different FFmpeg builds and
dependency sets. See [macOS source access](../packaging/macos-sources.md).
