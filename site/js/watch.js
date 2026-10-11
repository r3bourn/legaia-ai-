/* Watch mode: the play page shows a game an AI is playing on the homelab.
 *
 * The AI's own session runs elsewhere and records every input-side step it
 * applies (pad writes, ticks, FMV finish/hand-off). The engine's spectator
 * mode (`spectate_*` in crates/web-viewer/src/spectate.rs) applies that
 * stream to this page's session with the headless host's semantics, so the
 * picture here is exactly the AI's run.
 *
 * For that to hold, nothing on this page may touch the session: the page's
 * own per-frame duties (pad, camera azimuth, pause menu, naming overlay,
 * shop) are switched off on the runtime instance, and the page's tick becomes
 * one stream tick. The page still renders every frame from the session.
 * Pausing is free: a hidden tab stops requestAnimationFrame, so the page
 * stops consuming the stream and the game freezes exactly where it is.
 *
 * Stream source: `watch-stream.json` ({"url": "..."}) beside the page, a
 * server-local deployment file like homelab-disc.json. Without it there is no
 * Watch button.
 */
(function () {
  'use strict';

  const CONFIG_URL = 'watch-stream.json';

  async function readConfig() {
    try {
      const r = await fetch(CONFIG_URL, { cache: 'no-store' });
      if (!r.ok) return null;
      const cfg = await r.json();
      return cfg && typeof cfg.url === 'string' ? cfg : null;
    } catch (_) {
      return null;
    }
  }

  /* Runtime calls the page makes that change the session. In watch mode the
   * stream is the only writer. Queries the page uses to decide whether to
   * drive an overlay answer "closed" so the page does not try. */
  const SILENCE = {
    set_pad: undefined,
    set_camera_azimuth: undefined,
    set_left_stick: undefined,
    play_menu_input: false,
    play_menu_open: false,
    play_menu_take_load: '',
    play_menu_is_open: false,
    play_menu_sub_is_open: false,
    play_menu_scripted_open_pending: false,
    name_entry_input: undefined,
    name_entry_advance_frames: undefined,
    name_entry_is_active: false,
    play_shop_input: undefined,
    play_shop_is_open: false,
    play_shop_suspends_field: false,
    play_bgm_title_handoff: undefined,
  };

  /* Instance properties, not assignments: ai-player.js already shadows
   * `set_pad` with a non-writable property, which plain assignment cannot
   * replace. */
  function shadow(rt, name, fn) {
    Object.defineProperty(rt, name, { configurable: true, writable: true, value: fn });
  }

  /* `spectate_begin` resets the spectator state, queue included, so the
   * stream is fed after the New Game it begins, not before. */
  function patchRuntime(rt, streamText) {
    if (rt.__watchPatched) return;
    rt.__watchPatched = true;
    for (const [name, value] of Object.entries(SILENCE)) {
      if (typeof rt[name] === 'function') shadow(rt, name, () => value);
    }
    shadow(rt, 'play_new_game', () => {
      const scene = rt.spectate_begin();
      rt.spectate_feed(streamText);
      let state = null;
      try { state = JSON.parse(rt.state_json()); } catch (_) {}
      return JSON.stringify({ landing: 'new_game', scene, entered: true, state });
    });
    /* The page runs `play_drain_sim_steps` ticks a frame (60 a second); each
     * becomes one stream tick. A dry stream holds the frame. */
    shadow(rt, 'tick_frame', () => {
      const r = JSON.parse(rt.spectate_step(1));
      watch.ticks = r.total_ticks;
      watch.pending = r.pending;
      return r.entered || '';
    });
  }

  const watch = { active: false, ticks: 0, pending: 0, hud: null };
  window.LegaiaWatch = watch;

  function showHud() {
    const el = document.createElement('div');
    Object.assign(el.style, {
      position: 'fixed', left: '8px', bottom: 'max(8px, env(safe-area-inset-bottom))',
      zIndex: '2147483647', font: '600 11px -apple-system, system-ui, sans-serif',
      color: 'rgba(255,255,255,.75)', background: 'rgba(0,0,0,.35)',
      padding: '3px 7px', borderRadius: '6px', pointerEvents: 'none',
    });
    document.body.appendChild(el);
    watch.hud = el;
    setInterval(() => {
      const secs = Math.floor(watch.ticks / 60);
      const t = `${Math.floor(secs / 60)}:${String(secs % 60).padStart(2, '0')}`;
      el.textContent = watch.pending > 0 ? `AI playing · ${t}` : `AI thinking… · ${t}`;
    }, 500);
  }

  async function startWatching(cfg, btn) {
    const rt = window.__playRuntime;
    if (!rt || typeof rt.spectate_begin !== 'function' || !window.__playStartNewGame) {
      btn.textContent = 'Load the disc first';
      return;
    }
    btn.disabled = true;
    btn.textContent = 'Starting…';
    const r = await fetch(cfg.url, { cache: 'no-store' });
    if (!r.ok) { btn.textContent = 'Stream unavailable'; btn.disabled = false; return; }
    patchRuntime(rt, await r.text());
    watch.active = true;
    document.body.classList.add('lgp-watch');
    if (window.legaiaPhoneMode) window.legaiaPhoneMode(true);
    showHud();
    window.__playStartNewGame();
    btn.remove();
  }

  async function init() {
    const cfg = await readConfig();
    if (!cfg) return;
    const btn = document.createElement('button');
    btn.type = 'button';
    btn.textContent = '▶ Watch AI play';
    Object.assign(btn.style, {
      position: 'fixed', right: '12px', top: 'max(124px, calc(env(safe-area-inset-top) + 112px))',
      zIndex: '2147483647', padding: '12px 16px',
      font: '700 16px -apple-system, system-ui, sans-serif', color: '#fff',
      background: '#2f7d4f', border: '0', borderRadius: '10px',
    });
    btn.addEventListener('click', () => startWatching(cfg, btn).catch((e) => {
      btn.textContent = 'Watch failed: ' + e.message;
      btn.disabled = false;
    }));
    document.body.appendChild(btn);
    if (/[?&]watch=1\b/.test(location.search)) {
      const wait = setInterval(() => {
        if (window.__playRuntime && window.__playStartNewGame) {
          clearInterval(wait);
          btn.click();
        }
      }, 500);
    }
  }

  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', init);
  else init();
}());
