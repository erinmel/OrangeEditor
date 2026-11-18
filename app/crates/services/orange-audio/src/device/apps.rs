//! Per-application playback capture. Platform identities are scoped to the OS session.
use super::{Capturing, Levels, Meter};
use std::sync::Arc;

#[cfg(any(target_os = "linux", test))]
mod pipewire;
#[cfg(any(target_os = "linux", test))]
pub(super) use pipewire::Recording;

pub(crate) const PREFIX: &str = "app:";

/// Formats a device's display name. The selection still holds the original endpoint
/// identifier (or device name when the operating system supplies no ID).
pub(crate) fn device_label(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    let virtual_device = [
        "vb-audio",
        "vb-cable",
        "voicemeeter",
        "virtual audio cable",
        "blackhole",
        "loopback audio",
        "loopback (",
    ]
    .iter()
    .any(|hint| lower.contains(hint))
        || lower == "loopback";
    if virtual_device {
        format!("{name} (virtual audio)")
    } else {
        name.to_owned()
    }
}

pub(crate) fn choices() -> Result<Vec<(String, String)>, &'static str> {
    #[cfg(any(target_os = "linux", test))]
    if cfg!(target_os = "linux") {
        return pipewire::choices();
    }
    Err(unavailable())
}

pub(super) fn open(
    app: &str,
    meter: impl Fn(u32, u16) -> Meter,
    levels: Arc<Levels>,
) -> Result<Capturing, String> {
    #[cfg(any(target_os = "linux", test))]
    if cfg!(target_os = "linux") {
        return pipewire::open(app, meter, levels).map_err(str::to_owned);
    }
    let _ = (app, meter, levels);
    Err(unavailable().to_owned())
}

fn unavailable() -> &'static str {
    if cfg!(target_os = "macos") {
        "Per-app capture is unavailable on macOS. Route the app through a virtual audio device such as BlackHole, then select that device here."
    } else if cfg!(windows) {
        "Per-app capture is unavailable in this Windows build. Select a virtual audio cable to capture the apps routed to it."
    } else {
        "Per-app capture is unavailable on this system."
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn identifies_known_virtual_endpoints_by_name() {
        assert_eq!(
            super::device_label("BlackHole 2ch"),
            "BlackHole 2ch (virtual audio)"
        );
        assert_eq!(
            super::device_label("CABLE Output (VB-Audio Virtual Cable)"),
            "CABLE Output (VB-Audio Virtual Cable) (virtual audio)",
        );
        assert_eq!(super::device_label("USB Microphone"), "USB Microphone");
    }
}
