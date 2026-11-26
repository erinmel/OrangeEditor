# Steps the three packaging adapters share. Dot-source it: . "$PSScriptRoot/common.ps1"
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true

# Longest a launch check may take before the package counts as broken.
$LaunchSeconds = 180

# Whether this run builds an unsigned preview (release.json `unsigned_preview`).
function Test-UnsignedPreview {
    $env:RELEASE_UNSIGNED -eq 'true'
}

# Refuses to sign: no adapter has signing credentials yet.
function Assert-UnsignedPreview([string]$Platform) {
    if (-not (Test-UnsignedPreview)) {
        throw "Signing is not set up for $Platform yet: only unsigned previews can be packaged."
    }
}

# An empty output directory, created if it does not exist.
function Initialize-Output([string]$OutputDirectory) {
    if (Test-Path -LiteralPath $OutputDirectory) {
        if (Get-ChildItem -LiteralPath $OutputDirectory -Force) {
            throw "The output directory must be empty: $OutputDirectory"
        }
    } else {
        $null = New-Item -ItemType Directory -Path $OutputDirectory
    }
    (Resolve-Path -LiteralPath $OutputDirectory).Path
}

# A fresh directory for one adapter's working files, outside the output directory.
function New-WorkDirectory([string]$Name) {
    $path = Join-Path ([System.IO.Path]::GetTempPath()) "orange-release-$Name-$([guid]::NewGuid().ToString('N'))"
    $null = New-Item -ItemType Directory -Path $path
    $path
}

# The license and notice files at the repository root, to ship beside the program.
function Get-LicenseFiles {
    $root = Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '../..')
    @(Get-ChildItem -LiteralPath $root -File | Where-Object { $_.Name -match '^(LICENSE|COPYING|NOTICE)' })
}

# Starts the program from its installed place and requires the picture it saves of its
# own window: the window opened, drew and closed by itself.
function Test-Launch([string]$Executable) {
    $work = New-WorkDirectory 'launch'
    $picture = Join-Path $work 'launch.png'
    $out = Join-Path $work 'stdout.txt'
    $err = Join-Path $work 'stderr.txt'
    Write-Host "Launch check: $Executable --screenshot <picture>"
    $process = Start-Process -FilePath $Executable -ArgumentList @('--screenshot', $picture) -PassThru `
        -RedirectStandardOutput $out -RedirectStandardError $err
    if (-not $process.WaitForExit($LaunchSeconds * 1000)) {
        $process.Kill($true)
        Get-Content -LiteralPath $err -ErrorAction SilentlyContinue | Write-Host
        throw "The program did not close within $LaunchSeconds seconds."
    }
    $drawn = (Test-Path -LiteralPath $picture) -and (Get-Item -LiteralPath $picture).Length -gt 0
    if (-not $drawn) {
        Get-Content -LiteralPath $out, $err -ErrorAction SilentlyContinue | Write-Host
        throw "The program closed (exit code $($process.ExitCode)) without drawing its window."
    }
    Write-Host "Launch check passed: the window drew and closed."
    $true
}

# Writes receipt.json: exactly target, version, commit, checks and files.
function Write-Receipt(
    [string]$OutputDirectory,
    [string]$Target,
    [string]$Version,
    [string]$Commit,
    [System.Collections.Specialized.OrderedDictionary]$Checks
) {
    $files = @(Get-ChildItem -LiteralPath $OutputDirectory -File | ForEach-Object Name | Sort-Object)
    if (-not $files) { throw 'No package was written.' }
    $receipt = [ordered]@{
        target  = $Target
        version = $Version
        commit  = $Commit
        checks  = $Checks
        files   = $files
    }
    $receipt | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $OutputDirectory 'receipt.json') -Encoding utf8NoBOM
    Write-Host "Receipt written for $($files -join ', ')."
}
