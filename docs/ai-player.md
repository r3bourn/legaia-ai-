# AI Player Architecture

This fork adds an opt-in AI spectator mode around the clean-room Legaia engine.
The goal is deliberately narrower than "give an agent the computer": the game
publishes structured state, a local guide/orchestrator handles routine progress,
a decision provider is consulted only when judgment is needed, and a bounded
controller owns the actual PSX pad word.

No game disc bytes, saves, provider credentials, full walkthrough text or login
state belong in this repository.

> Development status: the source-level foundation is implemented on
> `feature/ai-player-foundation`, but it has not yet completed its first real
> Cargo/WASM compile in this fork. Run `bash scripts/ai/homelab-check.sh` on a
> Linux checkout before treating the branch as runnable.

## Trust boundaries

```text
browser / LegaiaRuntime
  |  semantic telemetry + watcher heartbeat
  v
checkpoint orchestrator
  |  deterministic directive when possible
  |  bounded DecisionRequest only when judgment is needed
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
deterministic navigation / controller / battle policy
  |
  v
16-bit PSX pad word -> World::set_pad
```

The language model never receives an arbitrary controller or filesystem API
from this project. The broker launches subscription CLIs in a fresh empty
temporary directory. Codex is requested read-only with approval escalation
disabled; Claude is requested in one-turn plan mode. Their own official clients
own login state.

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
- A provider response that arrives after the page is hidden, unfocused,
  replaced or disarmed is discarded before it can queue input.

That makes browser suspension on mobile a useful safety property: if the page
stops running, heartbeats stop and AI input dies shortly afterward.

## Browser spectator controls

`site/js/ai-player.js` attaches to the play page's existing
`window.__playRuntime` export rather than forking the enormous upstream frame
loop. The helper adds a small overlay with:

- `AI PLAY: ON/OFF`
- `TAKE CONTROL`
- current objective
- current checkpoint
- controller/broker status

The helper shadows only the live runtime instance's `set_pad` method so human
input still flows through the existing page, but Rust performs the arbitration.
Blur, page hide, page close, or manual input revoke AI authority.

`site/js/pad-bindings.js` loads the helper only on `play.html`.

## Semantic telemetry

The first telemetry schema is versioned (`schema_version = 1`) and is built from
engine-owned state rather than screenshots or raw RAM. The browser bridge
currently exposes:

- scene and mode;
- simulation frame;
- player actor slot, world XYZ and heading;
- party names, level, HP, MP and AP;
- dialog-active state;
- battle state, enemy slots and HP when available;
- money and party count.

Screenshots remain a fallback for visual ambiguity and commentary, not the main
control loop.

## Structured walkthroughs

`legaia-ai-guide` accepts a local YAML or JSON checkpoint file. The original
walkthrough can remain under `/srv/legaia-private`; the runtime needs only
small structured facts such as:

```yaml
- id: example-door
  objective: Reach the next door
  scene: town01
  action: navigate
  target: { x: 1000, z: 800, radius: 48 }
  completion: { kind: near_target }
```

Supported checkpoint actions are `navigate`, `interact`, `battle`, `open_menu`,
`wait`, and `observe`. Completion can be based on coordinates, scene changes,
mode, dialog state, frame number, manual confirmation, or `any`/`all`
combinations.

`legaia-ai-orchestrator` advances checkpoints locally. Routine navigation,
interaction and menu opening do not consume a Claude/Codex call. Unexpected
scenes and explicit `observe`/judgment checkpoints escalate to the broker.

`deploy/guide.example.yaml` documents the format. Its coordinates are
illustrative only.

## Deterministic field navigation

`legaia-ai-navigation` uses the engine's own authored field floor/collision
samplers. It performs a bounded BFS over the 64-unit field lattice, then
inverts the live camera-direction remap to produce real PSX pad input.

Navigation is deliberately incremental:

1. plan a collision-aware route;
2. hold the selected direction for only a few frames;
3. release it;
4. re-read semantic telemetry;
5. plan again.

This makes camera movement, encounters, doors and small errors self-correcting.
The final approach supports all eight directions and closes the last gap inside
a routing cell, so checkpoint radii smaller than one 64-unit cell still work.

## Routine battles

`legaia-ai-battle` contains a conservative semantic battle policy for ordinary
physical fights. It follows the engine's real command phases rather than timed
macros:

```text
Round prompt -> Begin
Command ring -> Attack
Attack mode  -> Auto
Target       -> confirm current living target
Commit       -> Begin round
```

It refuses to continue the routine physical loop when party HP falls below 35%,
when a non-routine submenu is already open, or when the battle state is not one
it understands. Those cases escalate instead of guessing.

The policy crate is implemented but its browser WASM method is not yet wired
into `web-viewer/src/lib.rs`; that final integration should be done after the
first compile verifies the new crate graph.

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

One-shot stdin mode is useful for direct provider testing:

```bash
printf '%s' '{
  "objective":"Reach the next checkpoint",
  "state":{"scene":"town01","mode":"field"},
  "walkthrough_context":"checkpoint=test"
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
POST /decision        direct model decision
POST /guide           guide/orchestrator only
POST /step            guide first, provider only if needed
POST /guide/advance   manual checkpoint advance
```

For normal spectator play, the browser calls `/step`. The broker reads the
current telemetry, advances/checks the guide, returns a deterministic directive
when it can, and invokes the configured model only for `need_decision`.

Set the local checkpoint file with:

```bash
export LEGAIA_GUIDE_PATH=/srv/legaia-private/guide.yaml
```

The server refuses a non-loopback bind unless `LEGAIA_AI_BEARER_TOKEN` is set.
Requests are capped at 512 KiB. A same-host reverse proxy is preferred over
binding the broker directly to a LAN or public interface.

## Recommended Proxmox/Linux layout

Use a small Debian/Ubuntu LXC or VM rather than the Proxmox host itself.

```text
/opt/legaia-ai/             compiled application / static site
/var/lib/legaia-ai/         broker scratch/runtime data
/srv/legaia-private/        user-owned disc, walkthrough, guide, saves (not Git)
/etc/legaia-ai/broker.env   local settings/secrets, mode 0600
/home/legaia-ai/            dedicated account + official CLI login state
```

Create a dedicated unprivileged account with no sudo membership. Log into
Claude Code and/or Codex as that account so their official authentication state
is isolated from other homelab services.

A systemd unit, environment example, nginx example and checkpoint example live
under `deploy/`.

## First verification on the homelab

Before installing services, compile the branch:

```bash
git clone -b feature/ai-player-foundation https://github.com/r3bourn/legaia-ai-.git
cd legaia-ai-
bash scripts/ai/homelab-check.sh
```

The check runs rustfmt validation, targeted native checks/tests for all AI
crates and the browser crate, then a `wasm32-unknown-unknown` type-check. If the
WASM target is missing it prints the one-time `rustup target add` command.

Do not install/start the systemd service until this check is green.

## Example service install after a green build

```bash
cargo build --release -p legaia-ai-broker
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

Once authenticated and the local guide path is configured:

```bash
sudo systemctl enable --now legaia-ai-broker
curl http://127.0.0.1:4317/health
```

## Remote browser access

The recommended topology is:

```text
phone / PC browser
      |
      v
Cloudflare Access / tunnel
      |
      v
nginx on homelab :8080
      |
      +--> static/WASM game page
      |
      `--> /ai/step -> 127.0.0.1:4317/step
```

`deploy/nginx-legaia.conf.example` implements the same-origin `/ai/` proxy.
This keeps the broker loopback-only and removes the need for browser-visible
provider credentials or CORS in the common deployment.

Do not proxy the Claude Code or Codex processes themselves.

## Remaining work after the first green compile

The major pieces still requiring implementation or live validation are:

1. wire the routine battle policy into the compiled WASM module and browser;
2. create/validate real story checkpoint coordinates from live gameplay and
   the user-supplied walkthrough;
3. add navigation stall detection/recovery around moving NPCs and unusual
   collision scenes;
4. add healing, magic, boss and scripted-battle policies;
5. add save/checkpoint recovery and optional rolling save-state support;
6. add optional screenshot/commentary fallback for unknown visual situations;
7. run full opening-sequence and longer playthrough tests on the real disc.

The safety and control boundary is already designed so those layers can be
added without letting the language model press arbitrary keys indefinitely.
