/* Optional "Load disc from homelab" button for the play page.
 *
 * Upstream play reads the disc from a file the viewer picks, which is awkward
 * on a phone (a 467 MB .bin has to be on the device first). A private
 * deployment can instead serve the owner's disc from its own server. This
 * helper looks for a server-local `homelab-disc.json` ({"url": "..."}), which
 * is deployment config and never committed; without it the page is unchanged.
 *
 * The fetched bytes go through the page's own `#play-file` input, so the
 * upstream load path and the IndexedDB disc cache behave exactly as if the
 * viewer had picked the file. Later visits use that cache and skip the fetch.
 */
(function () {
  'use strict';

  const CONFIG_URL = 'homelab-disc.json';

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

  async function fetchWithProgress(url, onProgress) {
    const r = await fetch(url, { cache: 'no-store' });
    if (!r.ok) throw new Error('HTTP ' + r.status);
    const total = Number(r.headers.get('Content-Length')) || 0;
    if (!r.body || !r.body.getReader) return new Uint8Array(await r.arrayBuffer());
    const reader = r.body.getReader();
    const out = total ? new Uint8Array(total) : null;
    const chunks = out ? null : [];
    let got = 0;
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      if (out) out.set(value, got); else chunks.push(value);
      got += value.length;
      onProgress(got, total);
    }
    if (out) return got === total ? out : out.subarray(0, got);
    const all = new Uint8Array(got);
    let o = 0;
    for (const c of chunks) { all.set(c, o); o += c.length; }
    return all;
  }

  function mb(n) { return (n / 1048576).toFixed(0); }

  async function init() {
    const input = document.getElementById('play-file');
    if (!input) return;
    const cfg = await readConfig();
    if (!cfg) return;

    const wrap = document.createElement('div');
    wrap.style.margin = '8px 0';
    const btn = document.createElement('button');
    btn.type = 'button';
    btn.textContent = 'Load disc from homelab';
    const status = document.createElement('span');
    status.style.marginLeft = '10px';
    wrap.append(btn, status);
    input.insertAdjacentElement('afterend', wrap);

    btn.addEventListener('click', async () => {
      btn.disabled = true;
      status.textContent = 'Downloading...';
      try {
        const bytes = await fetchWithProgress(cfg.url, (got, total) => {
          status.textContent = total
            ? `Downloading ${mb(got)} / ${mb(total)} MB`
            : `Downloading ${mb(got)} MB`;
        });
        const name = cfg.name || 'Legend of Legaia.bin';
        const dt = new DataTransfer();
        dt.items.add(new File([bytes], name, { type: 'application/octet-stream' }));
        input.files = dt.files;
        input.dispatchEvent(new Event('change', { bubbles: true }));
        status.textContent = `Loaded ${mb(bytes.length)} MB from homelab`;
      } catch (err) {
        status.textContent = 'Homelab disc failed: ' + err.message;
        btn.disabled = false;
      }
    });
  }

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', init);
  } else {
    init();
  }
}());
