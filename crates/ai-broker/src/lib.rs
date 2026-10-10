use anyhow::{Context, Result, anyhow, bail};
use legaia_ai_controller::HighLevelAction;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::env;
use std::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DecisionRequest {
    pub objective: String,
    pub state: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub walkthrough_context: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DecisionResponse {
    pub action: HighLevelAction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub confidence: f32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderConfig {
    Claude { command: String },
    Codex { command: String },
    Ollama { command: String, model: String },
}

impl ProviderConfig {
    pub fn from_env() -> Result<Self> {
        let provider = env::var("LEGAIA_AI_PROVIDER")
            .unwrap_or_else(|_| "claude".to_string())
            .to_ascii_lowercase();

        match provider.as_str() {
            "claude" => Ok(Self::Claude {
                command: env::var("LEGAIA_CLAUDE_COMMAND")
                    .unwrap_or_else(|_| "claude".to_string()),
            }),
            "codex" => Ok(Self::Codex {
                command: env::var("LEGAIA_CODEX_COMMAND")
                    .unwrap_or_else(|_| "codex".to_string()),
            }),
            "ollama" => Ok(Self::Ollama {
                command: env::var("LEGAIA_OLLAMA_COMMAND")
                    .unwrap_or_else(|_| "ollama".to_string()),
                model: env::var("LEGAIA_OLLAMA_MODEL")
                    .unwrap_or_else(|_| "qwen3:8b".to_string()),
            }),
            other => bail!("unsupported LEGAIA_AI_PROVIDER: {other}"),
        }
    }
}

pub fn decide(config: &ProviderConfig, request: &DecisionRequest) -> Result<DecisionResponse> {
    let prompt = build_prompt(request)?;
    let output = match config {
        ProviderConfig::Claude { command } => run_claude(command, &prompt)?,
        ProviderConfig::Codex { command } => run_codex(command, &prompt)?,
        ProviderConfig::Ollama { command, model } => run_ollama(command, model, &prompt)?,
    };

    parse_decision(&output)
}

pub fn build_prompt(request: &DecisionRequest) -> Result<String> {
    let state = serde_json::to_string_pretty(&request.state)
        .context("failed to serialize game state")?;

    let walkthrough = request
        .walkthrough_context
        .as_deref()
        .unwrap_or("No walkthrough context supplied.");

    Ok(format!(
        "You are the high-level decision layer for an AI playing Legend of Legaia.\n\
         You do not press buttons directly and you do not use tools. Choose one bounded, high-level next action.\n\
         Prefer deterministic progress toward the objective. Do not invent game state.\n\
         Return exactly one JSON object and no markdown.\n\
         The action MUST be one of: navigate, interact, open_menu, battle, wait, observe, pause.\n\
         Required schema:\n\
         {{\"action\":\"navigate\",\"target\":\"optional target or null\",\"reason\":\"brief reason\",\"confidence\":0.0}}\n\n\
         Objective:\n{}\n\n\
         Walkthrough context:\n{}\n\n\
         Current structured game state:\n{}",
        request.objective, walkthrough, state
    ))
}

fn run_claude(command: &str, prompt: &str) -> Result<String> {
    // Print mode is the supported non-interactive path. Plan permission mode
    // and one turn keep the gameplay broker decision-only. Running from a
    // fresh empty directory also prevents project files from becoming ambient
    // context for a model that only needs the supplied game-state JSON.
    run_command_isolated(
        command,
        &[
            "-p",
            prompt,
            "--output-format",
            "text",
            "--max-turns",
            "1",
            "--permission-mode",
            "plan",
        ],
        "Claude Code",
    )
}

fn run_codex(command: &str, prompt: &str) -> Result<String> {
    // Codex is explicitly pinned read-only with no approval escalation. The
    // empty working directory means even its read surface contains no game,
    // repository, save or secret files; authentication remains owned by the
    // official CLI under the dedicated service user's HOME.
    run_command_isolated(
        command,
        &[
            "exec",
            "--sandbox",
            "read-only",
            "--ask-for-approval",
            "never",
            "--skip-git-repo-check",
            prompt,
        ],
        "Codex CLI",
    )
}

fn run_ollama(command: &str, model: &str, prompt: &str) -> Result<String> {
    run_command_isolated(command, &["run", model, prompt], "Ollama")
}

fn run_command_isolated(command: &str, args: &[&str], label: &str) -> Result<String> {
    let cwd = tempfile::tempdir().context("failed to create isolated AI working directory")?;
    let output = Command::new(command)
        .args(args)
        .current_dir(cwd.path())
        .output()
        .with_context(|| format!("failed to launch {label} command `{command}`"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("{label} exited with {}: {}", output.status, stderr.trim());
    }

    let stdout = String::from_utf8(output.stdout)
        .with_context(|| format!("{label} returned non-UTF-8 output"))?;

    if stdout.trim().is_empty() {
        bail!("{label} returned an empty response");
    }

    Ok(stdout)
}

pub fn parse_decision(raw: &str) -> Result<DecisionResponse> {
    let trimmed = raw.trim();

    if let Ok(value) = serde_json::from_str::<DecisionResponse>(trimmed) {
        return validate_decision(value);
    }

    let start = trimmed
        .find('{')
        .ok_or_else(|| anyhow!("AI response did not contain a JSON object"))?;
    let end = trimmed
        .rfind('}')
        .ok_or_else(|| anyhow!("AI response did not contain a complete JSON object"))?;

    if end < start {
        bail!("AI response contained malformed JSON boundaries");
    }

    let candidate = &trimmed[start..=end];
    let value: DecisionResponse = serde_json::from_str(candidate)
        .context("AI response contained JSON, but not the expected decision schema")?;

    validate_decision(value)
}

fn validate_decision(value: DecisionResponse) -> Result<DecisionResponse> {
    if !(0.0..=1.0).contains(&value.confidence) {
        bail!("AI decision confidence must be between 0.0 and 1.0");
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn prompt_contains_objective_state_and_action_allowlist() {
        let request = DecisionRequest {
            objective: "Reach the Genesis Tree".to_string(),
            state: json!({"scene": "town01", "battle": false}),
            walkthrough_context: Some("Walk north through Rim Elm.".to_string()),
        };

        let prompt = build_prompt(&request).unwrap();
        assert!(prompt.contains("Reach the Genesis Tree"));
        assert!(prompt.contains("town01"));
        assert!(prompt.contains("Walk north through Rim Elm."));
        assert!(prompt.contains("navigate, interact, open_menu, battle, wait, observe, pause"));
    }

    #[test]
    fn parses_clean_json() {
        let decision = parse_decision(
            r#"{"action":"navigate","target":"genesis_tree","reason":"objective","confidence":0.91}"#,
        )
        .unwrap();

        assert_eq!(decision.action, HighLevelAction::Navigate);
        assert_eq!(decision.target.as_deref(), Some("genesis_tree"));
    }

    #[test]
    fn parses_json_surrounded_by_text() {
        let decision = parse_decision(
            "result:\n{\"action\":\"wait\",\"target\":null,\"reason\":\"cutscene\",\"confidence\":0.7}\n",
        )
        .unwrap();

        assert_eq!(decision.action, HighLevelAction::Wait);
    }

    #[test]
    fn rejects_unknown_action() {
        let result = parse_decision(
            r#"{"action":"delete_save","target":null,"reason":"x","confidence":0.5}"#,
        );
        assert!(result.is_err());
    }

    #[test]
    fn rejects_out_of_range_confidence() {
        let result = parse_decision(
            r#"{"action":"navigate","target":null,"reason":"x","confidence":1.5}"#,
        );
        assert!(result.is_err());
    }
}
