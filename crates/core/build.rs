use std::io::Result;

fn main() -> Result<()> {
    prost_build::Config::new()
        .type_attribute(".", "#[derive(serde::Serialize, serde::Deserialize)]")
        .compile_protos(&["proto/walrus.proto", "proto/whs.proto"], &["proto/"])?;
    Ok(())
}
