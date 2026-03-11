use crate::aggregator::Aggregator;
use crate::config::{Config, OutputFormat};
use crate::engine::EngineId;
use crate::error::Error;
use crate::browser::output;

pub async fn run(
    query: String,
    engines: Option<String>,
    max_results: Option<usize>,
    format: Option<String>,
    mut config: Config,
) -> Result<(), Error> {
    // Override config with CLI flags
    if let Some(engines_str) = engines {
        config.engines = parse_engine_list(&engines_str)?;
    }
    if let Some(max) = max_results {
        config.max_results = max;
    }
    if let Some(fmt) = format {
        config.output_format = parse_format(&fmt)?;
    }

    let aggregator = Aggregator::new(config.clone())?;
    let results = aggregator.search(&query, 0).await?;
    let output = output::format_results(&results, &config.output_format);
    print!("{output}");
    Ok(())
}

fn parse_engine_list(s: &str) -> Result<Vec<EngineId>, Error> {
    s.split(',')
        .map(|name| match name.trim().to_lowercase().as_str() {
            "wikipedia" => Ok(EngineId::Wikipedia),
            "duckduckgo" | "ddg" => Ok(EngineId::DuckDuckGo),
            other => Err(Error::Config(format!("unknown engine: {other}"))),
        })
        .collect()
}

fn parse_format(s: &str) -> Result<OutputFormat, Error> {
    match s.to_lowercase().as_str() {
        "json" => Ok(OutputFormat::Json),
        "text" => Ok(OutputFormat::Text),
        "compact" => Ok(OutputFormat::Compact),
        other => Err(Error::Config(format!("unknown format: {other}"))),
    }
}
