#requires -Version 7.0
param(
    [int]$Warmup = 2,
    [int]$Runs = 5
)

$ErrorActionPreference = 'Stop'
if ($Warmup -lt 0) { throw 'Warmup must be zero or greater.' }
if ($Runs -lt 1) { throw 'Runs must be at least one.' }

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$artifactsRoot = Join-Path $repoRoot '.artifacts'
$artifactsResolved = [System.IO.Path]::GetFullPath($artifactsRoot)
if (-not (Test-Path -LiteralPath $artifactsResolved -PathType Container)) {
    New-Item -ItemType Directory -Path $artifactsResolved | Out-Null
}
$guid = [guid]::NewGuid().ToString('N')
$scratchPath = [System.IO.Path]::GetFullPath((Join-Path $artifactsResolved "session-memory-$guid"))
$artifactsPrefix = $artifactsResolved.TrimEnd([System.IO.Path]::DirectorySeparatorChar) + [System.IO.Path]::DirectorySeparatorChar
if (-not $scratchPath.StartsWith($artifactsPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw "Scratch path escaped the repository artifacts directory: $scratchPath"
}
if (Test-Path -LiteralPath $scratchPath) { throw "Unexpected existing scratch path: $scratchPath" }

$sourceFiles = @(
    (Join-Path $repoRoot 'Cargo.toml'),
    (Join-Path $repoRoot 'Cargo.lock'),
    (Join-Path $repoRoot 'rust-toolchain.toml')
)
foreach ($sourceFile in $sourceFiles) {
    if (-not (Test-Path -LiteralPath $sourceFile -PathType Leaf)) { throw "Missing required source file: $sourceFile" }
}
$sourceApp = Join-Path $repoRoot 'app'
if (-not (Test-Path -LiteralPath $sourceApp -PathType Container)) { throw "Missing application source tree: $sourceApp" }

New-Item -ItemType Directory -Path $scratchPath | Out-Null
foreach ($sourceFile in $sourceFiles) {
    Copy-Item -LiteralPath $sourceFile -Destination $scratchPath
}

$sourceTreeFiles = Get-ChildItem -LiteralPath $sourceApp -File -Recurse | Where-Object {
    $relative = [System.IO.Path]::GetRelativePath($sourceApp, $_.FullName)
    $components = $relative -split '[\\/]'
    -not ($components | Where-Object { $_ -in @('.git', '.artifacts', 'target', 'worktrees') })
} | Sort-Object FullName
foreach ($sourceFile in $sourceTreeFiles) {
    $relative = [System.IO.Path]::GetRelativePath($sourceApp, $sourceFile.FullName)
    $destination = Join-Path (Join-Path $scratchPath 'app') $relative
    $destinationDirectory = Split-Path -Parent $destination
    if (-not (Test-Path -LiteralPath $destinationDirectory -PathType Container)) {
        New-Item -ItemType Directory -Path $destinationDirectory -Force | Out-Null
    }
    Copy-Item -LiteralPath $sourceFile.FullName -Destination $destination
}

$measureManifest = Join-Path $scratchPath 'app/crates/orange-measure/Cargo.toml'
$manifestText = Get-Content -LiteralPath $measureManifest -Raw
if ($manifestText -match '(?m)^\[\[bin\]\]') { throw 'Copied orange-measure manifest already has an explicit binary target.' }
$manifestText = $manifestText.TrimEnd() + @'


[[bin]]
name = "orange-session-memory"
path = "src/session_memory.rs"
'@
Set-Content -LiteralPath $measureManifest -Value $manifestText -NoNewline

$reportPath = Join-Path $scratchPath 'app/crates/orange-model/src/validation/report.rs'
$reportText = Get-Content -LiteralPath $reportPath -Raw
$reportAnchor = 'impl ValidationAnalysis {'
if ([regex]::Matches($reportText, [regex]::Escape($reportAnchor)).Count -ne 1) { throw 'ValidationAnalysis injection anchor must occur exactly once.' }
$reportHelper = @'
impl ValidationAnalysis {
    /// Scratch-only ownership partitioner; injected by tools/measure-session.ps1.
    pub fn __measurement_parts(self) -> Vec<(&'static str, Box<dyn std::any::Any>)> {
        let Self { report, document_state, registry_generation, covered } = self;
        let ValidationReport {
            diagnostics,
            resolved,
            incomplete_nodes,
            functions,
            incomplete_functions,
            data_cycle_paths,
            regions,
            control_cycle_paths,
        } = report;
        vec![
            ("published.signatures", Box::new(resolved)),
            ("published.diagnostics", Box::new(diagnostics)),
            ("published.report_rest", Box::new((
                incomplete_nodes,
                functions,
                incomplete_functions,
                data_cycle_paths,
                regions,
                control_cycle_paths,
            ))),
            ("published.freshness", Box::new((document_state, registry_generation, covered))),
        ]
    }
'@
$reportText = $reportText.Replace($reportAnchor, $reportHelper)
Set-Content -LiteralPath $reportPath -Value $reportText -NoNewline

$sessionPath = Join-Path $scratchPath 'app/crates/orange-model/src/validation/session.rs'
$sessionText = Get-Content -LiteralPath $sessionPath -Raw
$sessionAnchor = 'impl AnalysisSession {'
if ([regex]::Matches($sessionText, [regex]::Escape($sessionAnchor)).Count -ne 1) { throw 'AnalysisSession injection anchor must occur exactly once.' }
$sessionHelper = @'
impl AnalysisSession {
    /// Scratch-only ownership partitioner; injected by tools/measure-session.ps1.
    pub fn __measurement_parts(self) -> Vec<(&'static str, Box<dyn std::any::Any>)> {
        let Self {
            analysis, effects, limits, revision, content_state, stale, index,
            local, retained, changed_subjects, changed_nodes, stats,
        } = self;
        let mut parts = analysis.__measurement_parts();
        if let Some(mut retained) = retained {
            parts.push(("retained.args", Box::new(std::mem::take(&mut retained.args))));
            parts.push(("retained.node_diagnostics", Box::new(std::mem::take(&mut retained.node_diagnostics))));
            parts.push(("retained.inputs", Box::new(std::mem::take(&mut retained.inputs))));
            parts.push(("retained.config", Box::new(std::mem::take(&mut retained.config))));
            parts.push(("retained.functions", Box::new(std::mem::take(&mut retained.functions))));
            parts.push(("retained.base", Box::new(std::mem::take(&mut retained.base))));
            parts.push(("retained.flow", Box::new(std::mem::take(&mut retained.flow))));
            parts.push(("retained.state", Box::new(std::mem::take(&mut retained.state))));
            parts.push(("retained.metadata", Box::new(retained)));
        }
        parts.push(("session.local_validation", Box::new(local)));
        parts.push(("session.index", Box::new(index)));
        parts.push(("session.effects", Box::new(effects)));
        parts.push(("session.tracking", Box::new((
            limits,
            revision,
            content_state,
            stale,
            changed_subjects,
            changed_nodes,
            stats,
        ))));
        parts
    }
'@
$sessionText = $sessionText.Replace($sessionAnchor, $sessionHelper)
Set-Content -LiteralPath $sessionPath -Value $sessionText -NoNewline

$probeSource = Join-Path $PSScriptRoot 'session-memory.rs'
if (-not (Test-Path -LiteralPath $probeSource -PathType Leaf)) { throw "Missing probe source: $probeSource" }
$probeHash = (Get-FileHash -LiteralPath $probeSource -Algorithm SHA256).Hash.ToLowerInvariant()
$scriptHash = (Get-FileHash -LiteralPath $PSCommandPath -Algorithm SHA256).Hash.ToLowerInvariant()
$probeDestination = Join-Path $scratchPath 'app/crates/orange-measure/src/session_memory.rs'
Copy-Item -LiteralPath $probeSource -Destination $probeDestination

$hashLines = [System.Collections.Generic.List[string]]::new()
foreach ($sourceFile in $sourceFiles) {
    $hash = (Get-FileHash -LiteralPath $sourceFile -Algorithm SHA256).Hash.ToLowerInvariant()
    $hashLines.Add("$hash  $([System.IO.Path]::GetRelativePath($repoRoot, $sourceFile))")
}
foreach ($sourceFile in $sourceTreeFiles) {
    $hash = (Get-FileHash -LiteralPath $sourceFile.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
    $relative = [System.IO.Path]::GetRelativePath($repoRoot, $sourceFile.FullName)
    $hashLines.Add("$hash  $relative")
}
$hashLines.Add("$probeHash  tools/session-memory.rs")
$hashLines.Add("$scriptHash  tools/measure-session.ps1")
$hashLines | Set-Content -LiteralPath (Join-Path $scratchPath 'source-hashes.sha256')

$lockHash = (Get-FileHash -LiteralPath (Join-Path $repoRoot 'Cargo.lock') -Algorithm SHA256).Hash.ToLowerInvariant()
$rustVersion = (& rustc -Vv) -join [Environment]::NewLine
$cargoVersion = (& cargo -V) -join [Environment]::NewLine
Write-Output "scratch=$scratchPath"
Write-Output "source-copy-hashes=$(Join-Path $scratchPath 'source-hashes.sha256')"
Write-Output "Cargo.lock SHA256=$lockHash"
Write-Output $rustVersion
Write-Output $cargoVersion

$manifestPath = Join-Path $scratchPath 'Cargo.toml'
$targetPath = Join-Path $artifactsResolved 'sm-target'
Write-Output "target=$targetPath (kept short for the Windows MSVC linker)"
foreach ($nodes in @(2000, 10000)) {
    foreach ($mode in @('valid', 'warnings', 'error')) {
        & cargo run --release --locked --offline --target-dir $targetPath --manifest-path $manifestPath -p orange-measure --bin orange-session-memory -- $nodes $mode --warmup $Warmup --runs $Runs
        if ($LASTEXITCODE -ne 0) { throw "Probe failed for nodes=$nodes mode=$mode (exit $LASTEXITCODE). Scratch retained at $scratchPath" }
    }
}
