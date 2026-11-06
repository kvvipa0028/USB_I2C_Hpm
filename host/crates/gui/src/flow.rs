//! UDS Bootloader host path, stage P1 of
//! `docs/uds-bootloader-plugin-proposal.md`: the versioned backend
//! interface (`api`, §4.3), the host task manager with the §5.3 states and
//! an event archive (`host`), and one simulated backend (`mock`) that reads
//! a small demo package format invented for this stage. `manifest` parses
//! and checks the JSON `uds-flow-manifest` package; no backend reads it yet.
//!
//! Nothing here opens a bus, reaches the USB worker, runs a script found in
//! a package, or sends a UDS request. The only backend is the simulated
//! one, the host refuses any other mode, and every run it reports is a
//! simulated run. Real CAN / ISO-TP / UDS execution is P2 and not here.

pub mod api;
pub mod host;
pub mod manifest;
pub mod mock;

/// FNV-1a 64 of `bytes`. The host uses it to see that a package changed
/// between prepare and start; it is not a cryptographic hash.
pub fn fnv1a64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv_matches_reference_values() {
        assert_eq!(fnv1a64(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a64(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv1a64(b"foobar"), 0x8594_4171_f739_67e8);
    }

    /// The host path stays off the wire: none of its files reach the USB
    /// worker, the bridge crate, or a child process.
    #[test]
    fn host_path_never_reaches_usb_or_processes() {
        let files = [
            ("flow/api.rs", include_str!("flow/api.rs")),
            ("flow/host.rs", include_str!("flow/host.rs")),
            ("flow/manifest.rs", include_str!("flow/manifest.rs")),
            ("flow/mock.rs", include_str!("flow/mock.rs")),
            ("uds/sim.rs", include_str!("uds/sim.rs")),
        ];
        let banned = [
            ["crate::", "worker"].concat(),
            ["usb_i2c", "::"].concat(),
            ["Bri", "dge"].concat(),
            ["std::", "process"].concat(),
            ["Command", "::new"].concat(),
            ["std::", "net"].concat(),
        ];
        for (name, text) in files {
            for b in &banned {
                assert!(!text.contains(b.as_str()), "{name} mentions {b}");
            }
        }
    }
}
