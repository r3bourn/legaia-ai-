# Legaia AI Broker

`legaia-ai-broker` is the narrow decision boundary between the game engine and an AI provider.
It is intentionally **not** a general desktop-control agent. The broker receives structured game
state and returns one bounded high-level decision.

The first supported provider adapters are:

- `claude` — invokes the locally installed Claude Code CLI. Authentication stays inside Claude Code.
- `codex` — invokes the locally installed Codex CLI. Authentication stays inside Codex.
- `ollama` — invokes a local Ollama model and needs no cloud account.

No account cookies, subscription tokens, passwords, API keys, ROM bytes, saves, or walkthrough
files belong in this crate or in Git.

## Request/response contract

The executable reads one JSON request from stdin:

```json
{
  "objective": "Reach the Genesis Tree",
  "walkthrough_context": "Continue north through Rim Elm.",
  "state": {
    "scene": "town01",
    "mode": "field",
    "battle": false,
    "player": { "x": 2624, "z": 2710 }
  }
}
```

It emits one JSON decision on stdout:

```json
{
  "action": "move_to",
  "target": "genesis_tree",
  "reason": "This advances the current objective.",
  "confidence": 0.91
}
```

The game controller, not the model, will translate that decision into safe controller actions.

## Provider selection

Set `LEGAIA_AI_PROVIDER` to `claude`, `codex`, or `ollama`.

Optional command overrides:

- `LEGAIA_CLAUDE_COMMAND`
- `LEGAIA_CODEX_COMMAND`
- `LEGAIA_OLLAMA_COMMAND`
- `LEGAIA_OLLAMA_MODEL` (defaults to `qwen3:8b`)

Example:

```bash
export LEGAIA_AI_PROVIDER=claude
cargo run -p legaia-ai-broker < request.json
```

For Codex:

```bash
export LEGAIA_AI_PROVIDER=codex
cargo run -p legaia-ai-broker < request.json
```

For Ollama:

```bash
export LEGAIA_AI_PROVIDER=ollama
export LEGAIA_OLLAMA_MODEL=qwen3:8b
cargo run -p legaia-ai-broker < request.json
```

## Design rules

1. Real-time movement and button timing stay deterministic and local.
2. The AI is called at decision points, not every frame.
3. Model output is untrusted until parsed and validated.
4. Manual input will always override AI control.
5. A later watcher lease/heartbeat will fail closed: if the viewer disappears, queued AI input stops.
6. Provider authentication is owned by the provider's official local client, never copied into this project.
