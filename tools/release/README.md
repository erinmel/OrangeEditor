# Desktop release contract

Release CD is implemented in [release.yml](../../.github/workflows/release.yml). The workspace builds the `orange-editor` executable from `orange-desktop`. [.github/release.json](../../.github/release.json) selects that package and binary and declares `unsigned_preview`; `qualified` stays false until a rehearsal has passed on every platform, so a tagged release still stops at preflight.

Before the first release, the package needs a reviewed license declaration (the root `Cargo.toml` `license` field and a `LICENSE` file, which the adapters ship beside the program), a passing rehearsal, and `qualified: true`.

**Unsigned previews (2026-11-26).** With `"unsigned_preview": true`, only prerelease tags such as `v0.1.0-preview.1` release; a stable tag is refused. Receipts still report the signing checks (`signature`, `timestamp`, `notarization`, `staple`) but may report them `false`; every other check must pass. The release notes are [unsigned-preview.md](unsigned-preview.md), which tells people how to get past SmartScreen and Gatekeeper and how to check the checksums and provenance attestation. Signing adapters replace this once accounts exist.

**Rehearsals.** A manual run of the workflow rehearses by default: it builds, packages, installs and launches every platform from the chosen commit without a tag, qualification or license, and never publishes. Use one to qualify the adapters before tagging.

**Packages and baselines.** Linux builds and packages on Ubuntu 22.04 (glibc 2.35), so the packages also run on Ubuntu 24.04 and later, Debian 12 and later, Fedora and Arch: a `.tar.gz` and a `.deb` that depends on libc6 2.35 and ALSA. Windows ships a `.zip`. macOS ships a `.dmg` holding an ad-hoc-signed `OrangeEditor.app` for macOS 11 or later. Each adapter checks the real package: it unpacks or installs it fresh and launches the installed program with `--screenshot`, which draws the window and closes it. Linux launches on an Xvfb display with Mesa's software renderers.

The executable work unit also chooses release features, assets and runtime baselines. The three native adapters below implement packaging. `qualified` is an explicit maintainer declaration backed by the applicable WU-12/G5 or documented early-preview qualification evidence; it is not inferred from passing unit tests. Any later feature/target change requires renewed review.

Release triggers are annotated `v<package-version>` tags (including prerelease versions), or manual dispatch on such a tag. The tag must identify the checked-out commit and be an ancestor of protected `main`. Release branches need an explicit extension before separate release-line tags are accepted. Publication rechecks the live tag's commit and never overwrites an existing GitHub release. Do not move published tags.

## Native packaging adapters

`tools/release/linux.ps1`, `windows.ps1` and `macos.ps1` share [common.ps1](common.ps1). They run under PowerShell on native Ubuntu 22.04 x64, Windows x64/MSVC and macOS arm64 builders, with arguments:

```text
-BinaryPath <built executable>
-OutputDirectory <empty package output directory>
-Version <package version>
-Commit <exact source commit>
```

Each adapter must bundle the executable, assets and required runtime libraries into the selected package format, sign/notarize as appropriate, install/extract the actual final package and verify launch. It must throw or exit nonzero for any failed command, use bounded runtime checks, and log redacted commands/results. Emit a receipt only after real verification succeeds. A receipt is an assertion from trusted adapter code; the JSON validator does not independently verify platform signatures or invent hardware evidence. Real UI/GPU/audio qualification remains in its named delivery gates.

Required receipt checks:

| Platform | `target` | Required true checks (signing checks may be false in an unsigned preview) |
|---|---|---|
| Linux | `x86_64-unknown-linux-gnu` | `dependencies`, `installation`, `launch` |
| Windows | `x86_64-pc-windows-msvc` | `signature`, `timestamp`, `installation`, `launch` |
| macOS | `aarch64-apple-darwin` | `signature`, `notarization`, `staple`, `installation`, `launch` |

Write `receipt.json` with exactly `target`, `version`, `commit`, `checks`, `files`. The files array lists unique nonempty regular package files in the same directory; filenames must include a platform-specific name and use zip, tar.gz, msi, dmg, pkg, deb, rpm or AppImage extensions. There must be no extra output files, nested paths or symlinks. The verifier computes SHA-256 from package bytes with bounded memory and rejects a missing platform or colliding filename at collection. The workflow uploads the native executable without secrets, then packages it in a separate job; publish consumes only those verified packages from the same run without rebuilding.

## Credentials and publication

Packaging jobs use `release-linux`, `release-windows`, `release-macos` GitHub environments; publish uses `release-publish`. Configure their branch/tag restrictions and access before activation. Packaging receives OIDC capability and an optional platform environment secret `RELEASE_CREDENTIALS` only in its adapter step. The chosen adapter documents the provider-specific encoding; none is selected yet. Prefer short-lived provider credentials when available; do not place credentials, raw signing material or tokens in artifacts, receipts, caches or logs. Certificate/notarization accounts remain unconfigured today.

PR and reusable validation jobs have read-only permissions and receive no release secrets. Build jobs have read-only access. Only publish receives repository write and provenance-attestation permissions. It verifies all platform outputs, attests the final files, and creates the versioned GitHub release with checksums. A prerelease tag produces a GitHub prerelease. Release concurrency queues runs rather than cancelling signing or publishing midway.

## Validation

`node --test tools/release.test.mjs` exercises disabled/unqualified releases, wrong package/version, library targets, license/development-feature guards, missing packagers, failed native verification, artifact paths, actual checksums and complete platform collection. `node tools/release.mjs check` validates the tracked configuration without claiming release readiness. Run actionlint against both workflow files for schema/expression checks. No real desktop package can be built or published until the prerequisites above are implemented.

For each final desktop artifact, run the following from a fresh install with a clean
profile and record the platform, artifact checksum and result.

| Flow | Expected result |
| --- | --- |
| Save and reopen | Save a graph with custom names, positions, wires and inputs. Close and reopen the application. The graph retains the edits and produces the same result when triggered. |
| Package lifecycle | Export a pure function, import it, use it in a different graph and reopen that graph. After removal, the saved node remains visible with an unavailable-provider diagnostic. Reinstalling the exact archive restores it. |
| File access | Import a function that writes a file. Select a fresh folder connection and run the graph. Verify the expected contents in that folder and the failure route for an invalid path. |
| Audio playback and routing | Play a short file through an explicitly selected output. Verify which endpoint receives the signal and that completion fires. Repeat with a different endpoint where supported. Record any loopback or per-application capture result separately. |
