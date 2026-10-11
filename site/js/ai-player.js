/* Legend of Legaia AI spectator-mode browser client.
 *
 * Additive by design: upstream exposes the live wasm runtime as
 * `window.__playRuntime`. This helper watches for that object, wraps only the
 * instance's `set_pad` call so human input is arbitrated by the Rust
 * fail-closed controller, and drives the local homelab broker through the
 * same-origin `/ai/step` endpoint.
 *
 * No provider credentials live here. The reverse proxy authenticates the
 * browser session; Claude/Codex login state remains on the Linux broker host.
 */
(function () {
  'use strict';

  if (window.LegaiaAiPlayer) return;

  const DEFAULT_STEP_URL = '/ai/step';
  const LOOP_MS = 250;
  const HEARTBEAT_MS = 1000;

  const state = {
    runtime: null,
    originalSetPad: null,
    active: false,
    requestInFlight: false,
    nextRequestAt: 0,
    stepUrl: window.LEGAIA_AI_STEP_URL || DEFAULT_STEP_URL,
    panel: null,
    statusEl: null,
    objectiveEl: null,
    checkpointEl: null,
    toggleBtn: null,
  };

  function now() { return performance.now(); }

  function parseJson(fn, fallback) {
    try { return JSON.parse(fn()); } catch (_) { return fallback; }
  }

  function statusText(text) {
    if (state.statusEl) state.statusEl.textContent = text;
  }

  function syncButton() {
    if (!state.toggleBtn) return;
    state.toggleBtn.textContent = state.active ? 'AI PLAY: ON' : 'AI PLAY: OFF';
    state.toggleBtn.dataset.active = state.active ? '1' : '0';
    state.toggleBtn.style.borderColor = state.active ? '#69d38b' : '#777';
  }

  function ensurePanel() {
    if (state.panel && state.panel.isConnected) return;
    const root = document.createElement('div');
    root.id = 'legaia-ai-panel';
    Object.assign(root.style, {
      position: 'fixed', right: '12px', bottom: '12px', zIndex: '2147483646',
      width: 'min(360px, calc(100vw - 24px))', padding: '10px 12px',
      border: '1px solid rgba(255,255,255,.24)', borderRadius: '10px',
      background: 'rgba(12,14,20,.92)', color: '#eee',
      font: '12px/1.35 system-ui, -apple-system, sans-serif',
      boxShadow: '0 8px 30px rgba(0,0,0,.4)', backdropFilter: 'blur(8px)'
    });

    const top = document.createElement('div');
    Object.assign(top.style, { display: 'flex', gap: '7px', marginBottom: '8px' });

    const toggle = document.createElement('button');
    Object.assign(toggle.style, buttonStyle());
    toggle.addEventListener('click', () => state.active ? disarm('Paused by viewer') : arm());
    state.toggleBtn = toggle;

    const take = document.createElement('button');
    take.textContent = 'TAKE CONTROL';
    Object.assign(take.style, buttonStyle());
    take.addEventListener('click', takeControl);

    top.append(toggle, take);

    const objective = document.createElement('div');
    const checkpoint = document.createElement('div');
    const status = document.createElement('div');
    objective.textContent = 'Objective: waiting for guide';
    checkpoint.textContent = 'Checkpoint: -';
    status.textContent = 'AI disarmed';
    objective.style.fontWeight = '600';
    checkpoint.style.opacity = '.8';
    status.style.marginTop = '5px';
    status.style.opacity = '.85';

    root.append(top, objective, checkpoint, status);
    document.body.appendChild(root);
    state.panel = root;
    state.objectiveEl = objective;
    state.checkpointEl = checkpoint;
    state.statusEl = status;
    syncButton();
  }

  function buttonStyle() {
    return {
      appearance: 'none', background: '#202530', color: '#fff',
      border: '1px solid #777', borderRadius: '6px', padding: '6px 9px',
      cursor: 'pointer', font: '600 11px system-ui, -apple-system, sans-serif'
    };
  }

  function attachRuntime(rt) {
    if (!rt || state.runtime === rt) return;
    detachRuntime();
    state.runtime = rt;
    state.active = false;

    if (typeof rt.ai_set_pad !== 'function' || typeof rt.ai_telemetry_json !== 'function') {
      statusText('AI hooks unavailable. Rebuild the WASM bundle.');
      return;
    }

    // play-app.js calls the instance's `set_pad` once each frame. Shadow that
    // one method on this instance so the page does not need a forked frame
    // loop. Rust's `ai_set_pad` forwards the selected mask to its native
    // `LegaiaRuntime::set_pad`, so this wrapper does not recurse.
    try {
      state.originalSetPad = rt.set_pad.bind(rt);
      Object.defineProperty(rt, 'set_pad', {
        configurable: true,
        value(mask) { return rt.ai_set_pad(mask >>> 0, now()); }
      });
      rt.ai_disarm();
      statusText('AI ready, viewer must arm it');
    } catch (err) {
      state.originalSetPad = null;
      statusText('Could not attach AI pad arbitration: ' + err);
    }
    syncButton();
  }

  function detachRuntime() {
    const rt = state.runtime;
    if (rt) {
      try { rt.ai_disarm(); } catch (_) {}
      if (state.originalSetPad) {
        try {
          Object.defineProperty(rt, 'set_pad', {
            configurable: true,
            value: state.originalSetPad
          });
        } catch (_) {}
      }
    }
    state.runtime = null;
    state.originalSetPad = null;
    state.active = false;
    syncButton();
  }

  function viewerPresent() {
    return document.visibilityState === 'visible' && document.hasFocus();
  }

  function arm() {
    const rt = state.runtime;
    if (!rt || typeof rt.ai_arm !== 'function') return;
    if (!viewerPresent()) {
      statusText('Focus the game window before arming AI');
      return;
    }
    rt.ai_arm(now());
    state.active = true;
    state.nextRequestAt = 0;
    statusText('AI authorized while this page stays visible and focused');
    syncButton();
  }

  function disarm(reason) {
    if (state.runtime) {
      try { state.runtime.ai_disarm(); } catch (_) {}
    }
    state.active = false;
    state.requestInFlight = false;
    statusText(reason || 'AI disarmed');
    syncButton();
  }

  function takeControl() {
    if (state.runtime) {
      try { state.runtime.ai_manual_takeover(); } catch (_) {}
    }
    state.active = false;
    state.requestInFlight = false;
    statusText('Manual control has priority. Re-arm AI explicitly to resume.');
    syncButton();
  }

  function queueTap(button) {
    const rt = state.runtime;
    if (!rt) return false;
    try {
      rt.ai_set_plan_json(JSON.stringify({
        steps: [
          { buttons: [button], frames: 1 },
          { buttons: [], frames: 1 }
        ]
      }));
      return true;
    } catch (err) {
      statusText('Controller plan rejected: ' + err);
      return false;
    }
  }

  function applyGuideDirective(directive) {
    const rt = state.runtime;
    if (!rt || !directive) return;
    switch (directive.kind) {
      case 'navigate': {
        const nav = parseJson(
          () => rt.ai_navigate_to(directive.x, directive.z, directive.radius, now()),
          { status: 'error' }
        );
        statusText('Navigation: ' + (nav.status || 'unknown'));
        break;
      }
      case 'controller':
        try {
          rt.ai_set_plan_json(JSON.stringify(directive.plan));
          statusText('Executing bounded controller step');
        } catch (err) {
          statusText('Controller directive rejected: ' + err);
        }
        break;
      case 'wait':
        statusText(directive.reason || 'Waiting for game state');
        state.nextRequestAt = now() + 750;
        break;
      case 'pause':
        disarm(directive.reason || 'Guide paused AI');
        break;
      case 'complete':
        disarm('Guide complete');
        break;
      case 'need_decision':
        // `/step` includes the provider decision beside this directive. It is
        // applied by `applyModelDecision`, not here.
        break;
      default:
        disarm('Unknown guide directive: ' + directive.kind);
    }
  }

  function applyModelDecision(decision) {
    if (!decision) {
      disarm('Model decision missing');
      return;
    }
    switch (decision.action) {
      case 'interact':
        queueTap('cross');
        statusText('Model: interact');
        break;
      case 'open_menu':
        queueTap('start');
        statusText('Model: open menu');
        break;
      case 'wait':
      case 'observe':
        statusText('Model: ' + decision.action + (decision.reason ? ' · ' + decision.reason : ''));
        state.nextRequestAt = now() + 3000;
        break;
      case 'pause':
        disarm(decision.reason || 'Model requested pause');
        break;
      case 'navigate':
        // A prose target from the model is not enough to move safely. The
        // structured checkpoint must supply coordinates before navigation.
        disarm('Model requested navigation without a structured coordinate target');
        break;
      case 'battle':
        // Routine battle execution is a separate deterministic policy layer;
        // until that lands, fail closed rather than burn subscription calls or
        // improvise menu presses.
        disarm('Battle encountered. Battle executor is not enabled yet.');
        break;
      default:
        disarm('Unknown model action: ' + decision.action);
    }
  }

  async function requestStep() {
    const rt = state.runtime;
    if (!state.active || !rt || state.requestInFlight || now() < state.nextRequestAt) return;

    const aiStatus = parseJson(() => rt.ai_status_json(now()), null);
    if (!aiStatus || !aiStatus.authorized) {
      if (aiStatus && aiStatus.manual_override) takeControl();
      return;
    }
    if (!aiStatus.idle) return;

    let telemetry;
    try { telemetry = JSON.parse(rt.ai_telemetry_json()); }
    catch (err) {
      disarm('Telemetry error: ' + err);
      return;
    }

    state.requestInFlight = true;
    try {
      const response = await fetch(state.stepUrl, {
        method: 'POST',
        credentials: 'same-origin',
        cache: 'no-store',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(telemetry)
      });
      if (!response.ok) throw new Error(await response.text() || ('HTTP ' + response.status));
      const result = await response.json();
      const guide = result.guide || {};
      if (state.objectiveEl) state.objectiveEl.textContent = 'Objective: ' + (guide.objective || 'complete');
      if (state.checkpointEl) state.checkpointEl.textContent = 'Checkpoint: ' + (guide.checkpoint_id || '-');

      const directive = guide.directive;
      applyGuideDirective(directive);
      if (directive && directive.kind === 'need_decision') {
        applyModelDecision(result.decision);
      }
    } catch (err) {
      // Network/provider failure should not produce blind controller input.
      disarm('AI broker unavailable: ' + err.message);
    } finally {
      state.requestInFlight = false;
    }
  }

  // Watch the upstream play page's verification/runtime export. A load/save
  // recovery can replace the runtime, so this remains a cheap recurring check.
  setInterval(() => {
    ensurePanel();
    const rt = window.__playRuntime || null;
    if (rt && rt !== state.runtime) attachRuntime(rt);
  }, 250);

  setInterval(() => {
    const rt = state.runtime;
    if (!state.active || !rt) return;
    if (!viewerPresent()) {
      disarm('AI stopped because the page is no longer visible/focused');
      return;
    }
    try { rt.ai_heartbeat(now()); }
    catch (err) { disarm('AI heartbeat failed: ' + err); }
  }, HEARTBEAT_MS);

  setInterval(requestStep, LOOP_MS);

  document.addEventListener('visibilitychange', () => {
    if (document.visibilityState !== 'visible' && state.active) {
      disarm('AI stopped because the page was hidden');
    }
  });
  window.addEventListener('blur', () => {
    if (state.active) disarm('AI stopped because the window lost focus');
  });
  window.addEventListener('pagehide', () => disarm('AI stopped because the page closed'));

  window.LegaiaAiPlayer = {
    arm,
    disarm,
    takeControl,
    status: () => state.runtime ? parseJson(() => state.runtime.ai_status_json(now()), null) : null,
    telemetry: () => state.runtime ? parseJson(() => state.runtime.ai_telemetry_json(), null) : null,
    setStepUrl(url) { state.stepUrl = String(url || DEFAULT_STEP_URL); },
  };
}());
