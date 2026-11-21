# Desktop release contract

Release CD is implemented in [release.yml](../../.github/workflows/release.yml). The workspace builds the `orange-editor` executable from `orange-desktop`. [.github/release.json](../../.github/release.json) keeps package and binary unset and qualification disabled until the native adapters and their evidence are ready. Configuration validation succeeds. An attempted release stops at preflight.

Before enabling CD, the executable work unit must select its package and binary, declare a reviewed license, choose release features/assets/runtime baselines, and implement the three native adapters below. `qualified` is an explicit maintainer declaration backed by the applicable WU-12/G5 or documented early-preview qualification evidence; it is not inferred from passing unit tests. Any later feature/target change requires renewed review.

Release triggers are annotated `v<package-version>` tags (including prerelease versions), or manual dispatch on such a tag. The tag must identify the checked-out commit and be an ancestor of protected `main`. Release branches need an explicit extension before separate release-line tags are accepted. Publication rechecks the live tag's commit and never overwrites an existing GitHub release. Do not move published tags.

## Native packaging adapters

Provide `tools/release/linux.ps1`, `windows.ps1` and `macos.ps1` as real implementations when their application artifacts exist. They run under PowerShell on native Ubuntu x64, Windows x64/MSVC and macOS arm64 builders, with arguments:

```text
-BinaryPath <built executable>
-OutputDirectory <empty package output directory>
-Version <package version>
-Commit <exact source commit>
```

Each adapter must bundle the executable, assets and required runtime libraries into the selected package format, sign/notarize as appropriate, install/extract the actual final package and verify launch. It must throw or exit nonzero for any failed command, use bounded runtime checks, and log redacted commands/results. Emit a receipt only after real verification succeeds. A receipt is an assertion from trusted adapter code; the JSON validator does not independently verify platform signatures or invent hardware evidence. Real UI/GPU/audio qualification remains in its named delivery gates.

Required receipt checks:

| Platform | `target` | Required true checks |
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
