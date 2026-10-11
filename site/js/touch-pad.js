/* On-screen controller and "phone mode" for the play page on touch devices.
 *
 * The touch buttons do not synthesize key events. They feed the same path a
 * Bluetooth controller uses: `pad-bindings.js` merges
 * `window.legaiaVirtualPadButtons` (PSX button names) into its gamepad poll,
 * which the title loop and the play frame loop both call every frame. So a
 * touch press reaches the field, menus and the naming prompt exactly like a
 * physical controller, needs no canvas focus, and is unaffected by Key Config
 * rebinding (which maps keys, not controllers).
 *
 * Phone mode pins the canvas wrap to the screen at 4:3 (the same box the
 * page's real :fullscreen rule uses, so the dialogue box and menu overlay stay
 * aligned) and lays the controls over the letterbox. iPhone Safari has no
 * Fullscreen API for non-video elements, so this is the fullscreen there;
 * where the API exists (Android, iPad) it is requested as well. The injected
 * home-screen metadata makes "Add to Home Screen" open without Safari's bars.
 */
(function () {
  'use strict';

  const touchCapable = (window.matchMedia && matchMedia('(pointer: coarse)').matches)
    || 'ontouchstart' in window;
  if (!touchCapable) return;

  const held = new Set();
  window.legaiaVirtualPadButtons = held;

  const CSS = `
  .lgp-toggle { position: fixed; right: 12px; top: max(76px, calc(env(safe-area-inset-top) + 64px));
    z-index: 2147483647;
    padding: 10px 14px; font: 600 15px -apple-system, system-ui, sans-serif;
    color: #eee; background: rgba(12,14,20,.92); border: 1px solid rgba(255,255,255,.3);
    border-radius: 10px; }
  body.lgp-phone { overflow: hidden; background: #05070c; }
  body.lgp-phone .play-canvas-wrap { position: fixed !important; z-index: 2147483000;
    inset: 0; margin: auto; max-width: none !important;
    width: min(100vw, calc(100dvh * 4 / 3)); height: min(100dvh, calc(100vw * 3 / 4)); }
  body.lgp-phone .play-canvas { border: 0; width: 100%; height: 100%; }
  body.lgp-phone::before { content: ''; position: fixed; inset: 0; background: #05070c;
    z-index: 2147482999; }
  @media (orientation: portrait) {
    body.lgp-phone .play-canvas-wrap { top: 0; bottom: auto; height: calc(100vw * 3 / 4); width: 100vw; }
    /* Watching has no controls to make room for: centre the picture. */
    body.lgp-phone.lgp-watch .play-canvas-wrap { top: 0; bottom: 0; margin: auto; }
  }
  .lgp-pad { display: none; position: fixed; inset: 0; z-index: 2147483100;
    pointer-events: none; touch-action: none; user-select: none; -webkit-user-select: none; }
  body.lgp-phone .lgp-pad { display: block; }
  body.lgp-phone.lgp-watch .lgp-pad .lgp-btn:not(.lgp-exit), body.lgp-watch .lgp-dpad,
  body.lgp-watch .lgp-toggle { display: none !important; }
  body.lgp-watch #legaia-ai-panel { display: none !important; }
  .lgp-pad * { pointer-events: auto; touch-action: none; -webkit-touch-callout: none;
    -webkit-tap-highlight-color: transparent; }
  .lgp-btn { position: absolute; display: flex; align-items: center; justify-content: center;
    font: 700 18px -apple-system, system-ui, sans-serif; color: rgba(255,255,255,.85);
    background: rgba(255,255,255,.14); border: 2px solid rgba(255,255,255,.35); }
  .lgp-btn.lgp-on { background: rgba(255,255,255,.4); }
  .lgp-face { width: 58px; height: 58px; border-radius: 50%; }
  .lgp-small { width: 70px; height: 30px; border-radius: 15px; font-size: 12px; }
  .lgp-shoulder { width: 70px; height: 36px; border-radius: 10px; font-size: 14px; }
  .lgp-dpad { position: absolute; width: 150px; height: 150px; border-radius: 50%;
    background: rgba(255,255,255,.10); border: 2px solid rgba(255,255,255,.3); }
  .lgp-dpad .lgp-arm { position: absolute; background: rgba(255,255,255,.18); }
  .lgp-dpad.lgp-up .lgp-u, .lgp-dpad.lgp-down .lgp-d, .lgp-dpad.lgp-left .lgp-l,
  .lgp-dpad.lgp-right .lgp-r { background: rgba(255,255,255,.45); }
  .lgp-exit { position: absolute; top: max(8px, env(safe-area-inset-top)); right: 50%;
    transform: translateX(50%); width: 44px; height: 30px; border-radius: 8px; font-size: 14px; }
  body.lgp-phone #legaia-ai-panel { top: max(8px, env(safe-area-inset-top)) !important;
    bottom: auto !important; left: 8px !important; right: auto !important;
    width: min(240px, 40vw) !important; padding: 6px 8px !important; font-size: 11px !important;
    opacity: .9; }
  `;

  /* Button positions: landscape puts the pad on the letterbox either side of
   * the 4:3 picture; portrait stacks it under the picture. */
  function layout(pad) {
    const land = innerWidth > innerHeight;
    const L = (el, css) => Object.assign(el.style, { left: '', right: '', top: '', bottom: '' }, css);
    const s = pad.parts;
    const safeB = 'max(16px, env(safe-area-inset-bottom))';
    if (land) {
      L(s.dpad, { left: 'max(14px, env(safe-area-inset-left))', bottom: safeB });
      const fx = 'max(14px, env(safe-area-inset-right))';
      L(s.Triangle, { right: `calc(${fx} + 52px)`, bottom: 'calc(' + safeB + ' + 104px)' });
      L(s.Square, { right: `calc(${fx} + 104px)`, bottom: 'calc(' + safeB + ' + 52px)' });
      L(s.Circle, { right: fx, bottom: 'calc(' + safeB + ' + 52px)' });
      L(s.Cross, { right: `calc(${fx} + 52px)`, bottom: safeB });
      L(s.L1, { left: 'max(14px, env(safe-area-inset-left))', top: 'max(50px, env(safe-area-inset-top))' });
      L(s.R1, { right: 'max(14px, env(safe-area-inset-right))', top: 'max(50px, env(safe-area-inset-top))' });
      L(s.Select, { left: 'max(14px, env(safe-area-inset-left))', top: 'calc(max(50px, env(safe-area-inset-top)) + 50px)' });
      L(s.Start, { right: 'max(14px, env(safe-area-inset-right))', top: 'calc(max(50px, env(safe-area-inset-top)) + 50px)' });
    } else {
      L(s.dpad, { left: '18px', bottom: 'calc(' + safeB + ' + 40px)' });
      L(s.Triangle, { right: '70px', bottom: 'calc(' + safeB + ' + 144px)' });
      L(s.Square, { right: '122px', bottom: 'calc(' + safeB + ' + 92px)' });
      L(s.Circle, { right: '18px', bottom: 'calc(' + safeB + ' + 92px)' });
      L(s.Cross, { right: '70px', bottom: 'calc(' + safeB + ' + 40px)' });
      L(s.L1, { left: '18px', top: 'calc(100vw * 3 / 4 + 14px)' });
      L(s.R1, { right: '18px', top: 'calc(100vw * 3 / 4 + 14px)' });
      L(s.Select, { left: 'calc(50% - 78px)', bottom: safeB });
      L(s.Start, { right: 'calc(50% - 78px)', bottom: safeB });
    }
  }

  /* A button is held while any pointer that went down on it is still down. */
  function bindButton(el, name) {
    const pointers = new Set();
    const sync = () => {
      if (pointers.size) held.add(name); else held.delete(name);
      el.classList.toggle('lgp-on', pointers.size > 0);
    };
    el.addEventListener('pointerdown', (e) => {
      e.preventDefault();
      try { el.setPointerCapture(e.pointerId); } catch (_) {}
      pointers.add(e.pointerId); sync();
    });
    const up = (e) => { pointers.delete(e.pointerId); sync(); };
    ['pointerup', 'pointercancel', 'lostpointercapture'].forEach((t) => el.addEventListener(t, up));
  }

  /* The d-pad reads the finger's angle from the centre, so sliding round it
   * changes direction without lifting, and the 45-degree sectors between
   * arms press two directions (diagonal walking). */
  function bindDpad(el) {
    const DIRS = ['Up', 'Down', 'Left', 'Right'];
    const pointers = new Map();
    const apply = () => {
      const on = new Set();
      for (const d of pointers.values()) d.forEach((x) => on.add(x));
      DIRS.forEach((d) => {
        if (on.has(d)) held.add(d); else held.delete(d);
        el.classList.toggle('lgp-' + d.toLowerCase(), on.has(d));
      });
    };
    const dirsAt = (e) => {
      const r = el.getBoundingClientRect();
      const dx = e.clientX - (r.left + r.width / 2);
      const dy = e.clientY - (r.top + r.height / 2);
      if (Math.hypot(dx, dy) < r.width * 0.12) return [];
      const a = (Math.atan2(dy, dx) * 180 / Math.PI + 360) % 360; /* 0 = right, 90 = down */
      const out = [];
      if (a < 67.5 || a >= 292.5) out.push('Right');
      if (a >= 22.5 && a < 157.5) out.push('Down');
      if (a >= 112.5 && a < 247.5) out.push('Left');
      if (a >= 202.5 && a < 337.5) out.push('Up');
      return out;
    };
    el.addEventListener('pointerdown', (e) => {
      e.preventDefault();
      try { el.setPointerCapture(e.pointerId); } catch (_) {}
      pointers.set(e.pointerId, dirsAt(e)); apply();
    });
    el.addEventListener('pointermove', (e) => {
      if (!pointers.has(e.pointerId)) return;
      pointers.set(e.pointerId, dirsAt(e)); apply();
    });
    const up = (e) => { pointers.delete(e.pointerId); apply(); };
    ['pointerup', 'pointercancel', 'lostpointercapture'].forEach((t) => el.addEventListener(t, up));
  }

  function build() {
    const pad = document.createElement('div');
    pad.className = 'lgp-pad';
    pad.parts = {};
    const mk = (cls, label, name) => {
      const b = document.createElement('div');
      b.className = 'lgp-btn ' + cls;
      b.textContent = label;
      pad.appendChild(b);
      if (name) { pad.parts[name] = b; bindButton(b, name); }
      return b;
    };
    const dpad = document.createElement('div');
    dpad.className = 'lgp-dpad';
    [['lgp-u', 'left:52px;top:6px;width:42px;height:50px'],
      ['lgp-d', 'left:52px;bottom:6px;width:42px;height:50px'],
      ['lgp-l', 'left:6px;top:52px;width:50px;height:42px'],
      ['lgp-r', 'right:6px;top:52px;width:50px;height:42px']].forEach(([c, st]) => {
      const arm = document.createElement('div');
      arm.className = 'lgp-arm ' + c;
      arm.style.cssText = st + ';border-radius:8px';
      dpad.appendChild(arm);
    });
    pad.appendChild(dpad);
    pad.parts.dpad = dpad;
    bindDpad(dpad);
    mk('lgp-face', '△', 'Triangle');
    mk('lgp-face', '○', 'Circle');
    mk('lgp-face', '✕', 'Cross');
    mk('lgp-face', '□', 'Square');
    mk('lgp-shoulder', 'L1', 'L1');
    mk('lgp-shoulder', 'R1', 'R1');
    mk('lgp-small', 'SELECT', 'Select');
    mk('lgp-small', 'START', 'Start');
    const exit = mk('lgp-small lgp-exit', '✕', null);
    exit.title = 'Leave phone mode';
    exit.addEventListener('click', () => setPhoneMode(false));
    document.body.appendChild(pad);
    return pad;
  }

  function addHomeScreenMeta() {
    const add = (attrs) => {
      const m = document.createElement('meta');
      Object.entries(attrs).forEach(([k, v]) => m.setAttribute(k, v));
      document.head.appendChild(m);
    };
    add({ name: 'apple-mobile-web-app-capable', content: 'yes' });
    add({ name: 'mobile-web-app-capable', content: 'yes' });
    add({ name: 'apple-mobile-web-app-status-bar-style', content: 'black-translucent' });
    add({ name: 'apple-mobile-web-app-title', content: 'Legaia' });
    const vp = document.querySelector('meta[name=viewport]');
    if (vp) vp.setAttribute('content',
      'width=device-width, initial-scale=1, maximum-scale=1, user-scalable=no, viewport-fit=cover');
  }

  let pad = null;
  let toggle = null;

  function setPhoneMode(on) {
    document.body.classList.toggle('lgp-phone', on);
    if (toggle) toggle.style.display = on ? 'none' : '';
    if (!on) held.clear();
    if (on) {
      layout(pad);
      const wrap = document.querySelector('.play-canvas-wrap');
      const req = wrap && (wrap.requestFullscreen || wrap.webkitRequestFullscreen);
      if (req && !document.fullscreenElement) {
        Promise.resolve(req.call(document.documentElement)).catch(() => {});
      }
    } else if (document.fullscreenElement && document.exitFullscreen) {
      document.exitFullscreen().catch(() => {});
    }
  }

  window.legaiaPhoneMode = (on) => setPhoneMode(on);

  function init() {
    if (!document.querySelector('.play-canvas-wrap')) return;
    const style = document.createElement('style');
    style.textContent = CSS;
    document.head.appendChild(style);
    addHomeScreenMeta();
    pad = build();
    toggle = document.createElement('button');
    toggle.type = 'button';
    toggle.className = 'lgp-toggle';
    toggle.textContent = 'Phone mode';
    toggle.addEventListener('click', () => setPhoneMode(true));
    document.body.appendChild(toggle);
    /* The page's own Fullscreen button is disabled where the Fullscreen API
     * is missing (every iPhone browser). On a touch device it becomes the
     * phone-mode entry instead: a clone sheds play.html's listener. */
    const fsBtn = document.getElementById('play-fullscreen');
    if (fsBtn) {
      const phoneBtn = fsBtn.cloneNode(false);
      phoneBtn.disabled = false;
      phoneBtn.title = 'Fill the screen with on-screen controls';
      phoneBtn.textContent = '⛶ Phone mode';
      phoneBtn.addEventListener('click', () => setPhoneMode(true));
      fsBtn.replaceWith(phoneBtn);
    }
    addEventListener('resize', () => { if (document.body.classList.contains('lgp-phone')) layout(pad); });
    /* A held button must not stick when the page is hidden mid-press. */
    document.addEventListener('visibilitychange', () => { if (document.hidden) held.clear(); });
  }

  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', init);
  else init();
}());
