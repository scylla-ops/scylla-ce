//! The checkout settings of a pipeline: how the agent fetches the commit of a run before the
//! first node.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// How the agent fetches the commit of a run before the first node. Only a run with a commit
/// has a checkout.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CheckoutSettings {
    /// `false` gives the nodes an empty work folder. Default: `true`.
    #[serde(default = "enabled_default")]
    pub enabled: bool,
    /// The number of commits to fetch. `0` fetches the full history. Default: `1`.
    #[serde(default = "depth_default")]
    pub depth: u32,
    /// `true` also fetches the Git submodules. Default: `false`.
    #[serde(default)]
    pub submodules: bool,
    /// `true` also fetches the Git LFS files. Default: `false`.
    #[serde(default)]
    pub lfs: bool,
}

impl CheckoutSettings {
    /// Returns `true` if each setting has its default value.
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

impl Default for CheckoutSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            depth: 1,
            submodules: false,
            lfs: false,
        }
    }
}

/// The default of `enabled`.
fn enabled_default() -> bool {
    true
}

/// The default of `depth`.
fn depth_default() -> u32 {
    1
}
