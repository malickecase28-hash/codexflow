use crate::RuntimeHarness;
use std::time::Duration;

impl RuntimeHarness {
    /// Return the polling cadence from the pinned subswap daemon settings.
    ///
    /// The TUI owns the task lifecycle, but the cadence remains an embedded
    /// subswap policy detail rather than a second Codex-specific setting.
    pub fn quota_poll_interval(&self) -> Duration {
        let settings = subswap_core::settings::current();
        Duration::from_millis(settings.daemon.poll_interval_ms.max(1))
    }
}
