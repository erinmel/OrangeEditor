# Packages the macOS Apple Silicon executable as an app in a disk image, installs it
# from the image fresh and launches it. Unsigned previews only: the app carries an
# ad-hoc signature, which Apple Silicon requires to run, but no Developer ID and no
# notarization, so Gatekeeper asks before the first launch.
param(
    [Parameter(Mandatory)][string]$BinaryPath,
    [Parameter(Mandatory)][string]$OutputDirectory,
    [Parameter(Mandatory)][string]$Version,
    [Parameter(Mandatory)][string]$Commit
)
. "$PSScriptRoot/common.ps1"

Assert-UnsignedPreview 'macOS'
$output = Initialize-Output $OutputDirectory
$work = New-WorkDirectory 'macos'

# The app bundle. Bundle versions are numbers only, so a prerelease suffix is left out.
$number = ($Version -split '-', 2)[0]
$app = Join-Path $work 'staging/OrangeEditor.app'
$null = New-Item -ItemType Directory -Path (Join-Path $app 'Contents/MacOS') -Force
Copy-Item -LiteralPath $BinaryPath -Destination (Join-Path $app 'Contents/MacOS/orange-editor')
& chmod 755 (Join-Path $app 'Contents/MacOS/orange-editor')
$resources = Join-Path $app 'Contents/Resources'
$null = New-Item -ItemType Directory -Path $resources
foreach ($file in Get-LicenseFiles) { Copy-Item -LiteralPath $file.FullName -Destination $resources }
# A graph can listen to input devices, and macOS ends an app that asks for the
# microphone without saying why.
@"
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleExecutable</key><string>orange-editor</string>
  <key>CFBundleIdentifier</key><string>org.orange.editor</string>
  <key>CFBundleName</key><string>OrangeEditor</string>
  <key>CFBundleDisplayName</key><string>OrangeEditor</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$number</string>
  <key>CFBundleVersion</key><string>$number</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSMicrophoneUsageDescription</key><string>OrangeEditor listens to the input devices your graphs use.</string>
</dict>
</plist>
"@ | Set-Content -LiteralPath (Join-Path $app 'Contents/Info.plist') -Encoding utf8NoBOM
& plutil -lint (Join-Path $app 'Contents/Info.plist')
& codesign --force --sign - --timestamp=none $app
& codesign --verify --verbose=2 $app

# The disk image: the app and a link to Applications to drag it onto.
& ln -s /Applications (Join-Path $work 'staging/Applications')
$dmg = Join-Path $output "orange-editor-$Version-macos-arm64.dmg"
& hdiutil create -volname OrangeEditor -srcfolder (Join-Path $work 'staging') -fs HFS+ -format UDZO -ov $dmg

# Installation: copy the app out of the mounted image, as a person would.
$mount = Join-Path $work 'mount'
$installed = Join-Path (New-WorkDirectory 'macos-install') 'OrangeEditor.app'
$null = New-Item -ItemType Directory -Path $mount
& hdiutil attach $dmg -nobrowse -readonly -mountpoint $mount
try {
    & ditto (Join-Path $mount 'OrangeEditor.app') $installed
} finally {
    & hdiutil detach $mount
}
& codesign --verify --verbose=2 $installed
$launch = Test-Launch (Join-Path $installed 'Contents/MacOS/orange-editor')

Write-Receipt $output 'aarch64-apple-darwin' $Version $Commit ([ordered]@{
    signature    = $false
    notarization = $false
    staple       = $false
    installation = $true
    launch       = $launch
})
