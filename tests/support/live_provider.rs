use std::sync::Arc;

use cyber_agent::adapters::credentials::ProcessEnvironmentCredentialSource;
use cyber_agent::provider_composition::ProviderComposition;

pub fn load() -> ProviderComposition {
    let path = std::env::var_os("CYBER_AGENT_LIVE_CONFIG")
        .expect("CYBER_AGENT_LIVE_CONFIG must select the explicit Live Test configuration");
    ProviderComposition::load(path, Arc::new(ProcessEnvironmentCredentialSource))
        .expect("the explicit Live Test Provider configuration must be valid")
}
