mod pipeline;
mod profile;
mod qos;

pub use pipeline::{CopyTier, ReceiverOutput, SenderCapture};
pub use profile::{ProfileKind, ReceiverCapability, StreamProfile};
pub use qos::{DeviceClass, QosAction, QosController, QosConfig, StreamMetricsSample};

/// Workspace version string (`CARGO_PKG_VERSION` from the building crate).
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Crate name and version, e.g. `zerocast_desktop 0.2.0`.
pub fn name_and_version() -> String {
    format!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_matches_workspace() {
        assert_eq!(version(), "0.2.0");
    }
}
