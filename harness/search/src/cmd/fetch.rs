use crate::browser::fetch;
use crate::config::OutputFormat;
use crate::error::Error;

pub async fn run(url: String, format: &OutputFormat) -> Result<(), Error> {
    let client = fetch::default_client()?;
    let result = fetch::fetch_url(&url, &client).await?;

    match format {
        OutputFormat::Text | OutputFormat::Compact => {
            println!("{}", result.content);
        }
        OutputFormat::Json => {
            let json = serde_json::to_string_pretty(&result)?;
            println!("{json}");
        }
    }

    Ok(())
}
