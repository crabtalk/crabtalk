use crate::config::Config;

pub fn run(config: &Config, init: bool) {
    if init {
        println!("{}", Config::default_toml());
    } else {
        let toml = toml::to_string_pretty(config).unwrap_or_default();
        println!("{toml}");
    }
}
