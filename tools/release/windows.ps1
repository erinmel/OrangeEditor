# Packages the Windows x64 executable as a zip, unpacks it fresh and launches it.
# Unsigned previews only: signing is not set up yet (tools/release/README.md).
param(
    [Parameter(Mandatory)][string]$BinaryPath,
    [Parameter(Mandatory)][string]$OutputDirectory,
    [Parameter(Mandatory)][string]$Version,
    [Parameter(Mandatory)][string]$Commit
)
. "$PSScriptRoot/common.ps1"

Assert-UnsignedPreview 'Windows'
$output = Initialize-Output $OutputDirectory
$work = New-WorkDirectory 'windows'
$name = "orange-editor-$Version-windows-x86_64"

# The folder people unpack: the program and its license.
$folder = Join-Path $work $name
$null = New-Item -ItemType Directory -Path $folder
Copy-Item -LiteralPath $BinaryPath -Destination (Join-Path $folder 'orange-editor.exe')
foreach ($file in Get-LicenseFiles) { Copy-Item -LiteralPath $file.FullName -Destination $folder }
$zip = Join-Path $output "$name.zip"
Compress-Archive -Path $folder -DestinationPath $zip -CompressionLevel Optimal

# Installation: unpack the published zip somewhere new, as a person would.
$installed = Join-Path (New-WorkDirectory 'windows-install') 'unpacked'
Expand-Archive -LiteralPath $zip -DestinationPath $installed
$program = Join-Path $installed "$name/orange-editor.exe"
if (-not (Test-Path -LiteralPath $program)) { throw 'The zip does not hold orange-editor.exe.' }
$launch = Test-Launch $program

Write-Receipt $output 'x86_64-pc-windows-msvc' $Version $Commit ([ordered]@{
    signature    = $false
    timestamp    = $false
    installation = $true
    launch       = $launch
})
