#requires -Version 7.0
# Windows working-set companion for the Rust development measurement harness.
# Build the release binary first. Each invocation runs one profile in one child process.
[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [ValidateSet('index-update','import-profile','ownership-memory','authoring-compare','w9','session-edit')]
    [string]$Profile,
    [ValidateRange(1,1000)][int]$Runs = 7,
    [ValidateRange(1,1000)][int]$Warmup = 3,
    [string]$Binary = (Join-Path $PSScriptRoot '../target/release/orange-measure.exe'),
    [string]$OutputDirectory = (Join-Path $PSScriptRoot '../.artifacts/measurements'),
    [ValidatePattern('^[a-zA-Z0-9_-]+$')][string]$Label = 'sample'
)
$ErrorActionPreference = 'Stop'
$binaryPath = (Resolve-Path -LiteralPath $Binary).Path
$outputPath = [System.IO.Path]::GetFullPath($OutputDirectory)
New-Item -ItemType Directory -Force -Path $outputPath | Out-Null
$stdoutPath = Join-Path $outputPath "$Label-$Profile.log"
$stderrPath = Join-Path $outputPath "$Label-$Profile.stderr.log"
$memoryPath = Join-Path $outputPath "$Label-$Profile.memory.json"
$runProcess = Start-Process -FilePath $binaryPath -ArgumentList @('--warmup',"$Warmup",'--runs',"$Runs",$Profile) -WindowStyle Hidden -PassThru -RedirectStandardOutput $stdoutPath -RedirectStandardError $stderrPath
$peakWorkingSet = [long]0
$samples = 0
try {
    while (-not $runProcess.HasExited) {
        try {
            $runProcess.Refresh()
            $peakWorkingSet = [Math]::Max($peakWorkingSet, $runProcess.PeakWorkingSet64)
            $samples++
        } catch [System.InvalidOperationException] {
            # The child can exit between HasExited and refreshing the OS counter.
        }
        Start-Sleep -Milliseconds 10
    }
    $runProcess.WaitForExit()
    $exitCode = $runProcess.ExitCode
    [ordered]@{
        profile = $Profile
        binary = $binaryPath
        binary_sha256 = (Get-FileHash -LiteralPath $binaryPath -Algorithm SHA256).Hash
        runs = $Runs
        warmup = $Warmup
        exit_code = $exitCode
        observed_peak_working_set_bytes = $peakWorkingSet
        samples = $samples
        poll_interval_ms = 10
        scope = 'whole child process including fixture construction; Windows PeakWorkingSet64 sampled while alive; not allocator bytes or Linux RSS'
    } | ConvertTo-Json | Set-Content -LiteralPath $memoryPath -Encoding utf8
    if ($exitCode -ne 0) {
        Get-Content -LiteralPath $stderrPath
        throw "Measurement process failed with exit code $exitCode"
    }
    Write-Output "$Profile completed; results: $stdoutPath; working set: $memoryPath"
} finally {
    $runProcess.Dispose()
}
