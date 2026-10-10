use anyhow::{Context, Result};
use legaia_ai_broker::{DecisionRequest, ProviderConfig, decide};
use std::io::{self, Read};

fn main() {
    if let Err(err) = run() {
        eprintln!("legaia-ai-broker: {err:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let mut input = String::new();
    io::stdin()
        .read_to_string(&mut input)
        .context("failed to read decision request from stdin")?;

    let request: DecisionRequest = serde_json::from_str(&input)
        .context("stdin must contain one DecisionRequest JSON object")?;

    let config = ProviderConfig::from_env()?;
    let decision = decide(&config, &request)?;

    println!("{}", serde_json::to_string(&decision)?);
    Ok(())
}
