use std::sync::Arc;

use echidna::adapters::credentials::ProcessEnvironmentCredentialSource;
use echidna::provider_composition::ProviderComposition;

pub fn load() -> ProviderComposition {
    let path = std::env::var_os("CYBER_AGENT_LIVE_CONFIG")
        .expect("CYBER_AGENT_LIVE_CONFIG must select the explicit Live Test configuration");
    ProviderComposition::load(path, Arc::new(ProcessEnvironmentCredentialSource))
        .expect("the explicit Live Test Provider configuration must be valid")
}
