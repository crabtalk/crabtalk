use crabtalk_node::config::{DEFAULT_CONFIG, NodeConfig};

#[test]
fn parse_default_config_template() {
    NodeConfig::from_toml(DEFAULT_CONFIG).expect("default config template should parse");
}
