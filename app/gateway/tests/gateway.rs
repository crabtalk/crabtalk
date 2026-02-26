//! Gateway integration tests.

use llm::NoopProvider;
use runtime::{InMemory, Runtime};
use walrus_gateway::Gateway;

/// Verify that `Gateway::new` constructs with the unit hook `()`.
#[test]
fn gateway_new_with_unit_hook() {
    let config = llm::General::default();
    let rt = Runtime::<()>::new(config, NoopProvider, InMemory::new());
    let gw_config = walrus_gateway::GatewayConfig::from_toml(
        r#"
[server]

[llm]
model = "test"
api_key = "key"
"#,
    )
    .unwrap();
    let gw = Gateway::new(gw_config, rt);
    assert_eq!(gw.config.bind_address(), "127.0.0.1:3000");
}
