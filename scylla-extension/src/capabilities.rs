pub trait CapabilityProvider: Send + Sync + 'static {
    fn edition(&self) -> &'static str;
    fn capabilities(&self) -> Vec<&'static str>;
}

pub struct CommunityCapabilities;

impl CapabilityProvider for CommunityCapabilities {
    fn edition(&self) -> &'static str {
        "community"
    }

    fn capabilities(&self) -> Vec<&'static str> {
        Vec::new()
    }
}
