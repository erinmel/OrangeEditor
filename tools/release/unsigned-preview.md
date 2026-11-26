This is an early, unsigned preview of OrangeEditor. Expect rough edges, and keep
copies of graphs you care about.

## Install

| System | Download | First launch |
|---|---|---|
| Windows 10 and 11 (x64) | `orange-editor-…-windows-x86_64.zip` | Unzip and run `orange-editor.exe`. Windows SmartScreen says it protected your PC: choose **More info**, then **Run anyway**. |
| macOS 11 or later (Apple Silicon) | `orange-editor-…-macos-arm64.dmg` | Drag OrangeEditor to Applications. macOS says it cannot check the app: right-click it and choose **Open**, or allow it in **System Settings → Privacy & Security**. |
| Ubuntu 22.04+, Debian 12+ (x64) | `orange-editor-…-linux-amd64.deb` | `sudo apt install ./orange-editor-…-linux-amd64.deb`, then start OrangeEditor from the menu. |
| Other Linux (x64), such as Fedora or Arch | `orange-editor-…-linux-x86_64.tar.gz` | Unpack and run `orange-editor`. It needs glibc 2.35 or later and the ALSA library. |

## Why the warnings

The program is not code-signed yet, so Windows and macOS cannot confirm who made
it. Check the download against `SHA256SUMS` in this release, and the build's
provenance with `gh attestation verify <file> --repo erinmel/OrangeEditor`.
