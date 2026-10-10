# AI Player Architecture

This fork adds an opt-in AI spectator mode around the clean-room Legaia engine.
The goal is deliberately narrower than "give an agent the computer": the game
publishes structured state, a decision provider chooses a bounded high-level
action, and a local deterministic controller owns the actual PSX pad word.

No game disc bytes, saves, provider credentials, walkthrough text or login
state belong in this repository.

## Trust boundaries

```text
browser / LegaiaRuntime
  |  semantic telemetry + watcher heartbeat
  v
local AI orchestration
  |  bounded DecisionRequest
  v
legaia-ai-broker
  |-- Claude Code CLI  (official account login)
  |-- Codex CLI        (official ChatGPT login)
  `-- Ollama           (local fallback)
  |
  v
validated HighLevelAction
  |
  v
deterministic controller / guide runner
  |
  v
16-bit PSX pad word -> World::set_pad
```

The language model never receives a shell or arbitrary controller endpoint from
this project. The broker launches subscription CLIs in a fresh empty temporary
directory. Codex is requested read-only with approval escalation disabled;
Claude is requested in one-turn plan mode. Their own official clients own login
state.

## Watcher lease and takeover

`legaia-ai-controller` is fail-closed.

- A page load starts disarmed.
- `ai_arm(now_ms)` creates a five-second watcher lease.
- A visible/focused page renews it with `ai_heartbeat(now_ms)`.
- Lease expiry clears the queued plan and emits pad word `0`.
- Any non-zero human pad word through `ai_set_pad` immediately performs manual
  takeover, clears the AI queue and revokes authority.
- Heartbeats do not undo manual takeover. The user must explicitly arm again.
- Individual controller steps and whole plans have hard length limits.

That makes browser suspension on mobile a useful safety property: if iOS stops
the page, heartbeats stop and AI input dies shortly afterward.

## Semantic telemetry

The first telemetry schema is versioned (`schema_version = 1`) and is built from
engine-owned state rather than screenshots or raw RAM. The browser bridge
currently exposes scene, mode, frame, player actor slot, world XYZ and heading,
plus placeholders for party/battle/dialog/story data that can be expanded
without changing the control protocol.

Screenshots remain a fallback for visual ambiguity and commentary, not the main
control loop.

## Provider selection

The broker reads `LEGAIA_AI_PROVIDER`:

```text
claude   -> `claude -p ...` through the official Claude Code client
codex    -> `codex exec ...` through the official Codex client
ollama   -> `ollama run MODEL ...`
```

Provider usage still counts against whatever allowance the authenticated
product account supplies. The project does not extract cookies, tokens or
subscription credentials and does not emulate private provider APIs.

The default provider is Claude. Change it without rebuilding:

```bash
export LEGAIA_AI_PROVIDER=codex
```

For Ollama:

```bash
export LEGAIA_AI_PROVIDER=ollama
export LEGAIA_OLLAMA_MODEL=qwen3:8b
```

## Broker modes

One-shot stdin mode is useful for development:

```bash
printf '%s' '{
  "objective":"Reach the Genesis Tree",
  "state":{"scene":"town01","mode":"field"},
  "walkthrough_context":"Continue north through Rim Elm."
}' | ./target/release/legaia-ai-broker
```

Server mode is intended for the homelab:

```bash
LEGAIA_AI_PROVIDER=claude ./target/release/legaia-ai-broker serve
```

It binds to `127.0.0.1:4317` by default.

Endpoints:

```text
GET  /health
POST /decision
```

`POST /decision` accepts the same `DecisionRequest` JSON as stdin mode and
returns one validated `DecisionResponse`.

The server refuses a non-loopback bind unless `LEGAIA_AI_BEARER_TOKEN` is set.
Requests are capped at 512 KiB. A reverse proxy on the same host is preferred
over binding the broker directly to a LAN or public interface.

## Recommended Proxmox/Linux layout

Use a small Debian/Ubuntu LXC or VM rather than the Proxmox host itself.

```text
/opt/legaia-ai/             compiled application / checked-out source
/var/lib/legaia-ai/         broker scratch/runtime data
/srv/legaia-private/        user-owned disc, walkthrough, saves (not Git)
/etc/legaia-ai/broker.env   local settings/secrets, mode 0600
/home/legaia-ai/            dedicated account + official CLI login state
```

Create a dedicated unprivileged account with no sudo membership. Log into
Claude Code and/or Codex as that account so their official authentication state
is isolated from other homelab services.

A systemd unit and environment example live under `deploy/`.

Example install after building:

```bash
sudo useradd --create-home --shell /bin/bash legaia-ai
sudo install -d -o legaia-ai -g legaia-ai -m 0700 /var/lib/legaia-ai
sudo install -d -o root -g root -m 0755 /opt/legaia-ai/bin /etc/legaia-ai
sudo install -m 0755 target/release/legaia-ai-broker /opt/legaia-ai/bin/
sudo cp deploy/broker.env.example /etc/legaia-ai/broker.env
sudo chmod 0600 /etc/legaia-ai/broker.env
sudo cp deploy/legaia-ai-broker.service /etc/systemd/system/
sudo systemctl daemon-reload
```

Then switch into the service user to perform the official interactive logins
for the providers you intend to use. Do not paste those credentials into an
environment file.

Once authenticated:

```bash
sudo systemctl enable --now legaia-ai-broker
curl http://127.0.0.1:4317/health
```

## Remote browser access

The preferred eventual topology is:

```text
phone / PC browser
      |
      v
Cloudflare Access / reverse proxy
      |
      +--> static/WASM game page
      |
      `--> /ai/decision -> 127.0.0.1:4317/decision
```

This keeps the broker loopback-only. The reverse proxy can require Cloudflare
Access or another authenticated session, rate-limit `/ai/decision`, and remove
any need for browser-visible provider credentials.

Do not proxy the Claude Code or Codex processes themselves.

## What is not implemented yet

The foundation is intentionally layered. The following are subsequent stages,
not claims about current completeness:

1. richer party/battle/dialog telemetry;
2. structured walkthrough checkpoints and a checkpoint runner;
3. deterministic waypoint navigation with stall detection;
4. routine battle policy and menu executor;
5. browser UI controls for AI Play / Take Control / Pause;
6. same-origin reverse-proxy glue between the browser and broker;
7. save-state/checkpoint recovery and optional commentary.

The control boundary is already designed so those layers can be added without
letting the language model press arbitrary keys indefinitely.
