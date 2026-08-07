// SPDX-License-Identifier: MPL-2.0

use cosmic::cosmic_config::{self, CosmicConfigEntry, cosmic_config_derive::CosmicConfigEntry};
use serde::{Deserialize, Serialize};

/// Persistent configuration for the applet.
#[derive(Debug, Default, Clone, CosmicConfigEntry, Eq, PartialEq, Serialize, Deserialize)]
#[version = 1]
pub struct Config {
    /// Token used by the COSMIC runtime to restore the applet's position
    /// and state after a panel restart.
    #[serde(default)]
    pub restore_token: Option<String>,
}
