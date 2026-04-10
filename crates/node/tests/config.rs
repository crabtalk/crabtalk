use crabtalk_node::{NodeConfig, storage::DEFAULT_CONFIG};

#[test]
fn parse_default_config_template() {
    NodeConfig::from_toml(DEFAULT_CONFIG).expect("default config template should parse");
}
