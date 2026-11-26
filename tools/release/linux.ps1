# Packages the Linux x64 executable as a tarball and a .deb, installs both fresh and
# launches each on a virtual display. Run on the oldest supported build system
# (Ubuntu 22.04, glibc 2.35) so the packages also run on Debian 12, Fedora and Arch.
param(
    [Parameter(Mandatory)][string]$BinaryPath,
    [Parameter(Mandatory)][string]$OutputDirectory,
    [Parameter(Mandatory)][string]$Version,
    [Parameter(Mandatory)][string]$Commit
)
. "$PSScriptRoot/common.ps1"

$output = Initialize-Output $OutputDirectory
$work = New-WorkDirectory 'linux'
$name = "orange-editor-$Version-linux-x86_64"

# Dependencies: every shared library the program links is on the system.
$linked = & ldd $BinaryPath
$linked | Write-Host
if ($linked -match 'not found') { throw 'The program links a library the system does not have.' }

# What the launch checks need: a virtual display and software rendering.
& sudo apt-get update
& sudo apt-get install --yes --no-install-recommends xvfb mesa-vulkan-drivers libgl1-mesa-dri libegl1 libxkbcommon-x11-0

# The tarball: a folder with the program and its license.
$folder = Join-Path $work $name
$null = New-Item -ItemType Directory -Path $folder
Copy-Item -LiteralPath $BinaryPath -Destination (Join-Path $folder 'orange-editor')
& chmod 755 (Join-Path $folder 'orange-editor')
foreach ($file in Get-LicenseFiles) { Copy-Item -LiteralPath $file.FullName -Destination $folder }
$tarball = Join-Path $output "$name.tar.gz"
& tar --owner=0 --group=0 -C $work -czf $tarball $name

# The .deb. Debian sorts `~` before anything, so 0.1.0~preview.1 comes before 0.1.0.
$debVersion = $Version -replace '-', '~'
$root = Join-Path $work 'deb'
foreach ($dir in 'DEBIAN', 'usr/bin', 'usr/share/applications', 'usr/share/doc/orange-editor') {
    $null = New-Item -ItemType Directory -Path (Join-Path $root $dir) -Force
}
Copy-Item -LiteralPath $BinaryPath -Destination (Join-Path $root 'usr/bin/orange-editor')
& chmod 755 (Join-Path $root 'usr/bin/orange-editor')
foreach ($file in Get-LicenseFiles) {
    Copy-Item -LiteralPath $file.FullName -Destination (Join-Path $root 'usr/share/doc/orange-editor')
}
Write-TextFile (Join-Path $root 'usr/share/applications/orange-editor.desktop') @"
[Desktop Entry]
Type=Application
Name=OrangeEditor
Comment=Node editor for automation and reactive graphs
Exec=orange-editor %f
Terminal=false
Categories=Development;
"@
# Ubuntu 24.04 renamed the ALSA library package, so either name satisfies it.
Write-TextFile (Join-Path $root 'DEBIAN/control') @"
Package: orange-editor
Version: $debVersion
Architecture: amd64
Maintainer: OrangeEditor maintainers
Depends: libc6 (>= 2.35), libasound2 | libasound2t64
Section: devel
Priority: optional
Description: Node editor for automation and reactive graphs
 OrangeEditor edits and runs typed node graphs that react to timers, keys,
 files, network messages and audio.
"@
$deb = Join-Path $output "orange-editor-$Version-linux-amd64.deb"
& dpkg-deb --root-owner-group --build $root $deb

# Installation: the tarball unpacked somewhere new, and the .deb through apt, which
# also installs what it depends on. Each is launched where it was installed.
$unpacked = New-WorkDirectory 'linux-install'
& tar -xzf $tarball -C $unpacked
$display = Start-Process -FilePath Xvfb -ArgumentList @(':99', '-screen', '0', '1280x800x24', '-nolisten', 'tcp') -PassThru
$env:DISPLAY = ':99'
$env:WAYLAND_DISPLAY = ''
try {
    Start-Sleep -Seconds 2
    if ($display.HasExited) { throw 'The virtual display did not start.' }
    $fromTarball = Test-Launch (Join-Path $unpacked "$name/orange-editor")
    & sudo apt-get install --yes $deb
    try {
        $fromDeb = Test-Launch '/usr/bin/orange-editor'
    } finally {
        & sudo apt-get remove --yes orange-editor
    }
} finally {
    if (-not $display.HasExited) { $display.Kill() }
}

Write-Receipt $output 'x86_64-unknown-linux-gnu' $Version $Commit ([ordered]@{
    dependencies = $true
    installation = $true
    launch       = $fromTarball -and $fromDeb
})
