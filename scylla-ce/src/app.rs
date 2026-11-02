use std::sync::Arc;

use scylla_core::app::Edition;
use scylla_extension::{AllowAll, CommunityCapabilities, NoExtraServices};

pub fn edition() -> Edition {
    Edition {
        guard: Arc::new(AllowAll),
        capabilities: Arc::new(CommunityCapabilities),
        services: Arc::new(NoExtraServices),
        migrations: &[],
    }
}
