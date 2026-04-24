use crate::engine::EngineId;

pub fn run() {
    let engines: Vec<serde_json::Value> = EngineId::ALL
        .iter()
        .map(|id| {
            serde_json::json!({
                "id": id,
                "name": id.name(),
                "description": id.description(),
            })
        })
        .collect();

    let json = serde_json::to_string_pretty(&engines).unwrap_or_default();
    println!("{json}");
}
