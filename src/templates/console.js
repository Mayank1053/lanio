(() => {
  'use strict';

  const bodyDataset = document.body.dataset;
  const AUTH_REQUIRED = bodyDataset.authRequired === 'true';
  const STORAGE_KEY = 'lanio_console_token';

  const VIEW_META = {
    overview: {
      title: 'Overview & Add-on',
      subtitle: 'LAN streaming telemetry, Stremio manifest & library health',
    },
    media: {
      title: 'Media Library',
      subtitle: 'Browse matched titles, series episodes, unmatched files & TMDB overrides',
    },
    files: {
      title: 'File Explorer',
      subtitle: 'Inspect MEDIA_PATH folders, sidecar files, renames & single-file indexing',
    },
    server: {
      title: 'Server & Config',
      subtitle: 'Manage .env configuration, cron rescanning, TMDB cache & daemon lifecycle',
    },
    logs: {
      title: 'Live Logs',
      subtitle: 'Real-time tracing stream from scanner, file watcher & HTTP handlers',
    },
  };

  const state = {
    token: sessionStorage.getItem(STORAGE_KEY) || '',
    activeView: 'overview',
    mediaTab: 'movies',
    mediaData: { movies: [], series: [], unmatched: [], skipped: [] },
    overview: null,
    currentRelPath: '',
    logLevel: 'all',
    wasScanning: false,
    rematchTargetPath: '',
    posterTargetPath: '',
    posterPayload: null,
  };

  const $ = (id) => document.getElementById(id);

  function escapeHtml(str) {
    return String(str ?? '')
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;')
      .replace(/"/g, '&quot;');
  }

  function formatBytes(bytes) {
    if (bytes == null || isNaN(bytes)) return '—';
    if (bytes === 0) return '0 B';
    const units = ['B', 'KB', 'MB', 'GB', 'TB'];
    const i = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), units.length - 1);
    const val = bytes / Math.pow(1024, i);
    return `${val.toFixed(i === 0 ? 0 : 1)} ${units[i]}`;
  }

  function formatUptime(secs) {
    if (secs == null) return '—';
    const d = Math.floor(secs / 86400);
    const h = Math.floor((secs % 86400) / 3600);
    const m = Math.floor((secs % 3600) / 60);
    const s = secs % 60;
    if (d > 0) return `${d}d ${h}h ${m}m`;
    if (h > 0) return `${h}h ${m}m`;
    if (m > 0) return `${m}m ${s}s`;
    return `${s}s`;
  }

  function toast(message, kind = 'info') {
    const stack = $('toast-stack');
    const el = document.createElement('div');
    el.className = `toast ${kind}`;
    el.textContent = message;
    stack.appendChild(el);
    setTimeout(() => {
      el.style.opacity = '0';
      el.style.transition = 'opacity 0.25s ease';
      setTimeout(() => el.remove(), 260);
    }, 3400);
  }

  async function copyToClipboard(text) {
    if (navigator.clipboard && window.isSecureContext) {
      try {
        await navigator.clipboard.writeText(text);
        return true;
      } catch (_) {
        // Fall through to execCommand fallback
      }
    }
    const ta = document.createElement('textarea');
    ta.value = text;
    ta.setAttribute('readonly', '');
    ta.style.position = 'fixed';
    ta.style.top = '-9999px';
    ta.style.left = '-9999px';
    document.body.appendChild(ta);
    ta.focus();
    ta.select();
    let ok = false;
    try {
      ok = document.execCommand('copy');
    } catch (_) {
      ok = false;
    }
    ta.remove();
    return ok;
  }

  // Pure-JS SHA-512 fallback for non-HTTPS LAN origins (e.g. http://192.168.x.x) where crypto.subtle is unavailable
  function sha512Fallback(bytes) {
    const M = 0xffffffffffffffffn;
    const rotr = (x, n) => ((x >> BigInt(n)) | (x << BigInt(64 - n))) & M;
    const K = [
      0x428a2f98d728ae22n, 0x7137449123ef65cdn, 0xb5c0fbcfec4d3b2fn, 0xe9b5dba58189dbbcn,
      0x3956c25bf348b538n, 0x59f111f1b605d019n, 0x923f82a4af194f9bn, 0xab1c5ed5da6d8118n,
      0xd807aa98a3030242n, 0x12835b0145706fben, 0x243185be4ee4b28cn, 0x550c7dc3d5ffb4e2n,
      0x72be5d74f27b896fn, 0x80deb1fe3b1696b1n, 0x9bdc06a725c71235n, 0xc19bf174cf692694n,
      0xe49b69c19ef14ad2n, 0xefbe4786384f25e3n, 0x0fc19dc68b8cd5b5n, 0x240ca1cc77ac9c65n,
      0x2de92c6f592b0275n, 0x4a7484aa6ea6e483n, 0x5cb0a9dcbd41fbd4n, 0x76f988da831153b5n,
      0x983e5152ee66dfabn, 0xa831c66d2db43210n, 0xb00327c898fb213fn, 0xbf597fc7beef0ee4n,
      0xc6e00bf33da88fc2n, 0xd5a79147930aa725n, 0x06ca6351e003826fn, 0x142929670a0e6e70n,
      0x27b70a8546d22ffcn, 0x2e1b21385c26c926n, 0x4d2c6dfc5ac42aedn, 0x53380d139d95b3dfn,
      0x650a73548baf63den, 0x766a0abb3c77b2a8n, 0x81c2c92e47edaee6n, 0x92722c851482353bn,
      0xa2bfe8a14cf10364n, 0xa81a664bbc423001n, 0xc24b8b70d0f89791n, 0xc76c51a30654be30n,
      0xd192e819d6ef5218n, 0xd69906245565a910n, 0xf40e35855771202an, 0x106aa07032bbd1b8n,
      0x19a4c116b8d2d0c8n, 0x1e376c085141ab53n, 0x2748774cdf8eeb99n, 0x34b0bcb5e19b48a8n,
      0x391c0cb3c5c95a63n, 0x4ed8aa4ae3418acbn, 0x5b9cca4f7763e373n, 0x682e6ff3d6b2b8a3n,
      0x748f82ee5defb2fcn, 0x78a5636f43172f60n, 0x84c87814a1f0ab72n, 0x8cc702081a6439ecn,
      0x90befffa23631e28n, 0xa4506cebde82bde9n, 0xbef9a3f7b2c67915n, 0xc67178f2e372532bn,
      0xca273eceea26619cn, 0xd186b8c721c0c207n, 0xeada7dd6cde0eb1en, 0xf57d4f7fee6ed178n,
      0x06f067aa72176fban, 0x0a637dc5a2c898a6n, 0x113f9804bef90daen, 0x1b710b35131c471bn,
      0x28db77f523047d84n, 0x32caab7b40c72493n, 0x3c9ebe0a15c9bebcn, 0x431d67c49c100d4cn,
      0x4cc5d4becb3e42b6n, 0x597f299cfc657e2an, 0x5fcb6fab3ad6faecn, 0x6c44198c4a475817n,
    ];
    let H = [
      0x6a09e667f3bcc908n, 0xbb67ae8584caa73bn, 0x3c6ef372fe94f82bn, 0xa54ff53a5f1d36f1n,
      0x510e527fade682d1n, 0x9b05688c2b3e6c1fn, 0x1f83d9abfb41bd6bn, 0x5be0cd19137e2179n,
    ];

    const bitLen = BigInt(bytes.length) * 8n;
    const padLen = ((112 - ((bytes.length + 1) % 128) + 128) % 128) + 1;
    const padded = new Uint8Array(bytes.length + padLen + 16);
    padded.set(bytes);
    padded[bytes.length] = 0x80;
    const view = new DataView(padded.buffer);
    view.setBigUint64(padded.length - 16, bitLen >> 64n, false);
    view.setBigUint64(padded.length - 8, bitLen & M, false);

    const W = new Array(80);
    for (let off = 0; off < padded.length; off += 128) {
      for (let i = 0; i < 16; i++) {
        W[i] = view.getBigUint64(off + i * 8, false);
      }
      for (let i = 16; i < 80; i++) {
        const s0 = rotr(W[i - 15], 1) ^ rotr(W[i - 15], 8) ^ (W[i - 15] >> 7n);
        const s1 = rotr(W[i - 2], 19) ^ rotr(W[i - 2], 61) ^ (W[i - 2] >> 6n);
        W[i] = (W[i - 16] + s0 + W[i - 7] + s1) & M;
      }
      let [a, b, c, d, e, f, g, h] = H;
      for (let i = 0; i < 80; i++) {
        const S1 = rotr(e, 14) ^ rotr(e, 18) ^ rotr(e, 41);
        const ch = (e & f) ^ ((~e & M) & g);
        const temp1 = (h + S1 + ch + K[i] + W[i]) & M;
        const S0 = rotr(a, 28) ^ rotr(a, 34) ^ rotr(a, 39);
        const maj = (a & b) ^ (a & c) ^ (b & c);
        const temp2 = (S0 + maj) & M;
        h = g; g = f; f = e; e = (d + temp1) & M;
        d = c; c = b; b = a; a = (temp1 + temp2) & M;
      }
      H = [
        (H[0] + a) & M, (H[1] + b) & M, (H[2] + c) & M, (H[3] + d) & M,
        (H[4] + e) & M, (H[5] + f) & M, (H[6] + g) & M, (H[7] + h) & M,
      ];
    }
    return H.map((x) => x.toString(16).padStart(16, '0')).join('');
  }

  async function computeToken(password) {
    const enc = new TextEncoder();
    const bytes = enc.encode('lanio_auth:' + password);
    if (window.crypto && window.crypto.subtle) {
      const buf = await window.crypto.subtle.digest('SHA-512', bytes);
      return Array.from(new Uint8Array(buf))
        .map((b) => b.toString(16).padStart(2, '0'))
        .join('');
    }
    return sha512Fallback(bytes);
  }

  async function apiFetch(path, options = {}) {
    const headers = new Headers(options.headers || {});
    if (state.token) {
      headers.set('X-Lanio-Token', state.token);
    }
    if (options.body && !headers.has('Content-Type')) {
      headers.set('Content-Type', 'application/json');
    }
    const resp = await fetch(path, { ...options, headers });
    if (resp.status === 401 && AUTH_REQUIRED) {
      lockConsole();
      throw new Error('Session expired or unauthorized');
    }
    const text = await resp.text();
    let json = null;
    try {
      json = text ? JSON.parse(text) : null;
    } catch (_) {
      json = null;
    }
    if (!resp.ok) {
      throw new Error((json && json.error) || text || `HTTP ${resp.status}`);
    }
    return json;
  }

  function lockConsole() {
    state.token = '';
    sessionStorage.removeItem(STORAGE_KEY);
    if (AUTH_REQUIRED) {
      $('console-shell').classList.add('hidden');
      $('auth-gate').classList.remove('hidden');
      $('auth-password').value = '';
    }
  }

  async function unlockWithToken(token) {
    state.token = token;
    const overview = await apiFetch('/api/overview');
    sessionStorage.setItem(STORAGE_KEY, token);
    $('auth-gate').classList.add('hidden');
    $('auth-error').classList.add('hidden');
    $('console-shell').classList.remove('hidden');
    applyOverview(overview);
    await Promise.all([loadMedia(), loadConfig(), loadFiles(''), loadLogs()]);
  }

  async function initAuth() {
    $('auth-form').addEventListener('submit', async (e) => {
      e.preventDefault();
      const pw = $('auth-password').value;
      if (!pw) return;
      try {
        const token = await computeToken(pw);
        await unlockWithToken(token);
      } catch (_) {
        state.token = '';
        $('auth-error').classList.remove('hidden');
      }
    });

    $('btn-lock').addEventListener('click', lockConsole);

    if (AUTH_REQUIRED) {
      if (state.token) {
        try {
          await unlockWithToken(state.token);
          return;
        } catch (_) {
          lockConsole();
        }
      }
    } else {
      await bootstrapConsole();
    }
  }

  async function bootstrapConsole() {
    await Promise.all([
      loadOverview(),
      loadMedia(),
      loadConfig(),
      loadFiles(''),
      loadLogs(),
    ]);
  }

  // Navigation
  function openSidebar() {
    $('sidebar').classList.add('open');
    $('sidebar-backdrop').classList.add('open');
  }

  function closeSidebar() {
    $('sidebar').classList.remove('open');
    $('sidebar-backdrop').classList.remove('open');
  }

  function switchView(view) {
    state.activeView = view;
    document.querySelectorAll('.nav-item, .mobile-nav-item').forEach((btn) => {
      btn.classList.toggle('active', btn.dataset.view === view);
    });
    document.querySelectorAll('.view-pane').forEach((pane) => {
      pane.classList.toggle('active', pane.id === `view-${view}`);
    });
    const meta = VIEW_META[view];
    if (meta) {
      $('view-title').textContent = meta.title;
      $('view-subtitle').textContent = meta.subtitle;
    }
    closeSidebar();
    window.scrollTo({ top: 0, behavior: 'instant' });

    if (view === 'logs') loadLogs();
    if (view === 'files') loadFiles(state.currentRelPath);
    if (view === 'server') loadConfig();
  }

  // Overview
  function applyOverview(ov) {
    state.overview = ov;
    $('kpi-movies').textContent = ov.counts.movies;
    $('kpi-series').textContent = ov.counts.series;
    $('kpi-episodes').textContent = `${ov.counts.episodes} episodes total`;
    const unmatchedTotal = ov.counts.unmatched + ov.counts.skipped;
    $('kpi-unmatched').textContent = unmatchedTotal;
    $('kpi-unmatched-mode').textContent = ov.unmatched_catalog
      ? `${ov.counts.unmatched} in catalog`
      : `${ov.counts.skipped} skipped (catalog off)`;
    $('kpi-size').textContent = formatBytes(ov.total_size_bytes);

    if (ov.last_scan) {
      $('kpi-last-scan').textContent = `Last scan: ${ov.last_scan.duration_ms}ms (${ov.last_scan.matched} matched)`;
      $('ov-scan-dur').textContent = `${ov.last_scan.duration_ms} ms @ ${new Date(ov.last_scan.finished_at).toLocaleTimeString()}`;
    } else {
      $('kpi-last-scan').textContent = 'Awaiting initial scan';
      $('ov-scan-dur').textContent = '—';
    }

    const totalMediaCount = ov.counts.movies + ov.counts.series + unmatchedTotal;
    $('nav-badge-media').textContent = totalMediaCount;
    const mobileBadge = $('mobile-badge-media');
    mobileBadge.textContent = totalMediaCount;
    mobileBadge.classList.toggle('hidden', totalMediaCount === 0);
    $('side-watcher').textContent = ov.watcher_active ? 'ACTIVE' : 'OFF';
    $('side-cache').textContent = `${ov.tmdb_cache_entries} entries`;
    $('side-uptime').textContent = formatUptime(ov.uptime_secs);

    $('manifest-url-box').textContent = ov.manifest_url;
    $('btn-install-web').href = ov.install_url;
    $('btn-install-app').href = ov.manifest_url.replace(/^https?:\/\//, 'stremio://');
    $('btn-open-manifest').href = ov.manifest_url;

    const authPill = $('addon-auth-pill');
    authPill.textContent = ov.auth_enabled ? 'TOKEN PROTECTED' : 'OPEN LAN';
    authPill.className = `chip ${ov.auth_enabled ? 'warn' : 'ok'}`;

    $('ov-media-path').textContent = ov.media_path + (ov.media_path_exists ? '' : ' (missing!)');
    $('ov-base-url').textContent = ov.base_url || `http://localhost:${ov.port}`;
    $('ov-public-url').textContent = ov.public_url || 'Not set';
    $('ov-cron').textContent = ov.scan_cron
      ? `${ov.scan_cron}${ov.next_scheduled_scan ? ' · next ' + new Date(ov.next_scheduled_scan).toLocaleTimeString() : ''}`
      : 'Disabled (watcher only)';
    $('ov-watcher').textContent = ov.watcher_active ? 'Watching recursively' : 'Inactive';

    const scanning = Boolean(ov.scanning);
    $('scan-banner').classList.toggle('hidden', !scanning);
    const pulse = $('sidebar-pulse');
    pulse.classList.toggle('status-scanning', scanning);
    $('sidebar-pulse-text').textContent = scanning ? 'SCANNING' : 'ONLINE';

    if (state.wasScanning && !scanning) {
      loadMedia();
      if (state.activeView === 'files') loadFiles(state.currentRelPath);
      toast('Media library scan completed', 'ok');
    }
    state.wasScanning = scanning;
  }

  async function loadOverview() {
    try {
      const ov = await apiFetch('/api/overview');
      applyOverview(ov);
    } catch (e) {
      console.error(e);
    }
  }

  // Media Library
  async function loadMedia() {
    try {
      const data = await apiFetch('/api/media');
      state.mediaData = data;
      $('cnt-movies').textContent = data.movies.length;
      $('cnt-series').textContent = data.series.length;
      $('cnt-unmatched').textContent = data.unmatched.length;
      $('cnt-skipped').textContent = data.skipped.length;
      renderMedia();
    } catch (e) {
      toast(`Failed to load media: ${e.message}`, 'err');
    }
  }

  function posterImgHtml(url, alt, fallbackText) {
    if (!url) return fallbackText;
    return `<img src="${escapeHtml(url)}" alt="${escapeHtml(alt)}" loading="lazy" onerror="this.replaceWith(document.createTextNode('${fallbackText}'))">`;
  }

  function renderMedia() {
    const container = $('media-content');
    const q = $('media-search').value.trim().toLowerCase();
    const tab = state.mediaTab;

    if (tab === 'movies') {
      const items = state.mediaData.movies.filter((m) =>
        !q ||
        m.title.toLowerCase().includes(q) ||
        m.imdb_id.toLowerCase().includes(q) ||
        m.file_name.toLowerCase().includes(q)
      );
      if (!items.length) {
        container.innerHTML = `<div class="empty-state">No movies match your filter.</div>`;
        return;
      }
      container.innerHTML = items.map((m) => `
        <article class="media-card">
          <div class="media-card-top">
            <div class="poster-thumb">
              ${posterImgHtml(m.poster, m.title, 'NO POSTER')}
            </div>
            <div class="media-info">
              <div class="media-title">${escapeHtml(m.title)}</div>
              <div class="media-meta-row">
                <span class="chip ok">${escapeHtml(m.imdb_id)}</span>
                ${m.year ? `<span class="chip">${m.year}</span>` : ''}
                ${m.tmdb_rating ? `<span class="chip warn">★ ${m.tmdb_rating.toFixed(1)}</span>` : ''}
                ${m.size_bytes != null ? `<span class="chip">${formatBytes(m.size_bytes)}</span>` : ''}
              </div>
              <p class="media-desc">${escapeHtml(m.description || 'No overview available.')}</p>
            </div>
          </div>
          <div class="media-file-bar" title="${escapeHtml(m.file_path)}">${escapeHtml(m.file_name)}</div>
          <div class="media-actions">
            <button class="btn btn-amber btn-sm" data-play="${escapeHtml(m.stream_url)}" data-title="${escapeHtml(m.title)}">▶ Stream</button>
            <button class="btn btn-ghost btn-sm" data-copy="${escapeHtml(m.stream_url)}">Copy URL</button>
            <button class="btn btn-ghost btn-sm" data-rematch="${escapeHtml(m.file_path)}" data-rtype="movie" data-rtitle="${escapeHtml(m.title)}" data-ryear="${m.year || ''}">Rematch</button>
          </div>
        </article>
      `).join('');
    } else if (tab === 'series') {
      const items = state.mediaData.series.filter((s) =>
        !q ||
        s.title.toLowerCase().includes(q) ||
        s.imdb_id.toLowerCase().includes(q) ||
        s.episodes.some((ep) => ep.file_name.toLowerCase().includes(q))
      );
      if (!items.length) {
        container.innerHTML = `<div class="empty-state">No TV series match your filter.</div>`;
        return;
      }
      container.innerHTML = items.map((s) => `
        <article class="media-card">
          <div class="media-card-top">
            <div class="poster-thumb">
              ${posterImgHtml(s.poster, s.title, 'NO POSTER')}
            </div>
            <div class="media-info">
              <div class="media-title">${escapeHtml(s.title)}</div>
              <div class="media-meta-row">
                <span class="chip ok">${escapeHtml(s.imdb_id)}</span>
                ${s.year ? `<span class="chip">${s.year}</span>` : ''}
                <span class="chip info">${s.episode_count} ep${s.episode_count === 1 ? '' : 's'}</span>
                <span class="chip">${formatBytes(s.total_size_bytes)}</span>
              </div>
              <p class="media-desc">${escapeHtml(s.description || 'No overview available.')}</p>
            </div>
          </div>
          <div class="episodes-drawer">
            ${s.episodes.map((ep) => {
              const code = `S${String(ep.season ?? 0).padStart(2, '0')}E${String(ep.episode ?? 0).padStart(2, '0')}`;
              return `
                <div class="episode-row">
                  <span class="episode-code">${code}</span>
                  <span class="episode-name" title="${escapeHtml(ep.file_path)}">${escapeHtml(ep.file_name)}</span>
                  <button class="btn btn-ghost btn-sm" data-play="${escapeHtml(ep.stream_url)}" data-title="${escapeHtml(s.title + ' ' + code)}">▶</button>
                  <button class="btn btn-ghost btn-sm" data-rematch="${escapeHtml(ep.file_path)}" data-rtype="series" data-rtitle="${escapeHtml(s.title)}" data-ryear="${s.year || ''}" data-season="${ep.season ?? ''}" data-episode="${ep.episode ?? ''}">✎</button>
                </div>
              `;
            }).join('')}
          </div>
        </article>
      `).join('');
    } else {
      const list = tab === 'unmatched' ? state.mediaData.unmatched : state.mediaData.skipped;
      const items = list.filter((u) =>
        !q || u.title.toLowerCase().includes(q) || u.file_path.toLowerCase().includes(q)
      );
      if (!items.length) {
        container.innerHTML = `<div class="empty-state">No ${tab} files found.</div>`;
        return;
      }
      container.innerHTML = items.map((u) => `
        <article class="media-card">
          <div class="media-card-top">
            <div class="poster-thumb">
              ${posterImgHtml(u.poster_url, u.title, 'UNMATCHED')}
            </div>
            <div class="media-info">
              <div class="media-title">${escapeHtml(u.title)}</div>
              <div class="media-meta-row">
                <span class="chip ${tab === 'unmatched' ? 'warn' : 'err'}">${tab.toUpperCase()}</span>
                ${u.extension ? `<span class="chip">${escapeHtml(u.extension.toUpperCase())}</span>` : ''}
                ${u.size_bytes != null ? `<span class="chip">${formatBytes(u.size_bytes)}</span>` : ''}
                ${u.has_sidecar ? `<span class="chip ok">Sidecar Poster</span>` : ''}
              </div>
              <p class="media-desc mono">${escapeHtml(u.file_path)}</p>
            </div>
          </div>
          <div class="media-actions">
            <button class="btn btn-amber btn-sm" data-play="${escapeHtml(u.stream_url)}" data-title="${escapeHtml(u.title)}">▶ Stream</button>
            <button class="btn btn-outline btn-sm" data-rematch="${escapeHtml(u.file_path)}" data-rtype="movie" data-rtitle="${escapeHtml(u.title.replace(/\.[^/.]+$/, '').replace(/[._]/g, ' '))}">Match TMDB</button>
            <button class="btn btn-ghost btn-sm" data-poster-file="${escapeHtml(u.file_path)}">Set Poster</button>
          </div>
        </article>
      `).join('');
    }
  }

  // File Explorer
  async function loadFiles(relPath) {
    state.currentRelPath = relPath;
    try {
      const data = await apiFetch(`/api/files?path=${encodeURIComponent(relPath)}`);
      $('btn-files-up').disabled = data.parent_relative === null;
      $('btn-files-up').dataset.parent = data.parent_relative ?? '';

      const crumbsEl = $('files-breadcrumbs');
      const parts = data.current_relative ? data.current_relative.split('/').filter(Boolean) : [];
      let accum = '';
      const crumbs = [`<button class="crumb" data-path="">MEDIA_PATH</button>`];
      for (const part of parts) {
        accum = accum ? `${accum}/${part}` : part;
        crumbs.push(`<span class="muted">/</span><button class="crumb" data-path="${escapeHtml(accum)}">${escapeHtml(part)}</button>`);
      }
      crumbsEl.innerHTML = crumbs.join('');

      const tbody = $('files-tbody');
      if (!data.entries.length) {
        tbody.innerHTML = `<tr><td colspan="5" class="muted">Directory is empty.</td></tr>`;
        return;
      }

      tbody.innerHTML = data.entries.map((entry) => {
        const badgeClass =
          entry.index_status === 'matched' ? 'ok' :
          entry.index_status === 'unmatched' ? 'warn' :
          entry.index_status === 'skipped' ? 'err' : '';
        const nameCell = entry.is_dir
          ? `<button class="dir-link" data-open-dir="${escapeHtml(entry.relative_path)}">📁 ${escapeHtml(entry.name)}</button>`
          : `<span class="mono">${escapeHtml(entry.name)}</span>${entry.matched_title ? ` <span class="muted">(${escapeHtml(entry.matched_title)})</span>` : ''}`;

        return `
          <tr>
            <td>${nameCell}</td>
            <td data-label="Kind"><span class="chip">${escapeHtml(entry.is_dir ? 'folder' : entry.kind)}</span></td>
            <td data-label="Status">${entry.is_dir ? '<span class="muted">—</span>' : `<span class="chip ${badgeClass}">${escapeHtml(entry.imdb_id || entry.index_status)}</span>`}</td>
            <td data-label="Size" class="mono">${entry.is_dir ? '—' : formatBytes(entry.size_bytes)}</td>
            <td class="right actions-cell">
              ${entry.stream_url ? `<button class="btn btn-ghost btn-sm" data-play="${escapeHtml(entry.stream_url)}" data-title="${escapeHtml(entry.name)}">▶</button>` : ''}
              ${entry.kind === 'video' ? `<button class="btn btn-ghost btn-sm" data-rescan-file="${escapeHtml(entry.full_path)}">↻</button>` : ''}
              ${entry.kind === 'video' ? `<button class="btn btn-ghost btn-sm" data-rematch="${escapeHtml(entry.full_path)}" data-rtype="movie" data-rtitle="${escapeHtml(entry.name.replace(/\.[^/.]+$/, '').replace(/[._]/g, ' '))}">Match</button>` : ''}
              <button class="btn btn-ghost btn-sm" data-rename-path="${escapeHtml(entry.full_path)}" data-rename-cur="${escapeHtml(entry.name)}">Rename</button>
            </td>
          </tr>
        `;
      }).join('');
    } catch (e) {
      toast(`File explorer error: ${e.message}`, 'err');
    }
  }

  // Server & Config
  async function loadConfig() {
    try {
      const cfg = await apiFetch('/api/config');
      $('cfg-media-path').value = cfg.media_path;
      $('cfg-port').value = cfg.port;
      $('cfg-base-url').value = cfg.base_url;
      $('cfg-public-url').value = cfg.public_url;
      $('cfg-poster-url').value = cfg.poster_url;
      $('cfg-scan-cron').value = cfg.scan_cron;
      $('cfg-unmatched').checked = cfg.unmatched_catalog;
      $('cfg-tmdb-masked').textContent = cfg.has_tmdb_api_key ? `(${cfg.tmdb_api_key_masked})` : '(unset)';
      $('cfg-pw-status').textContent = cfg.password_set ? '(set)' : '(none)';
      $('cfg-env-path').textContent = cfg.env_file_path;
      $('cfg-env-badge').textContent = cfg.env_file_exists ? '.env found' : '.env will be created';
      $('cfg-clear-pw').checked = false;
      $('cfg-tmdb-key').value = '';
      $('cfg-password').value = '';
    } catch (e) {
      toast(`Failed to load config: ${e.message}`, 'err');
    }
  }

  // Logs
  async function loadLogs() {
    try {
      const level = state.logLevel;
      const filterText = $('log-filter-text').value.trim().toLowerCase();
      const data = await apiFetch(`/api/logs?limit=250&level=${encodeURIComponent(level)}`);
      const container = $('logs-stream');
      const entries = (data.entries || []).filter((e) =>
        !filterText ||
        e.message.toLowerCase().includes(filterText) ||
        e.target.toLowerCase().includes(filterText)
      );
      if (!entries.length) {
        container.innerHTML = `<div class="muted" style="padding:16px">No log entries match the current filter.</div>`;
        return;
      }
      container.innerHTML = entries.map((e) => {
        const ts = e.timestamp.includes('T') ? e.timestamp.split('T')[1].replace('Z', '').slice(0, 12) : e.timestamp;
        return `
          <div class="log-row">
            <span class="log-ts">${escapeHtml(ts)}</span>
            <span class="log-lvl lvl-${escapeHtml(e.level)}">${escapeHtml(e.level)}</span>
            <span class="log-tgt" title="${escapeHtml(e.target)}">${escapeHtml(e.target)}</span>
            <span class="log-msg">${escapeHtml(e.message)}</span>
          </div>
        `;
      }).join('');
      container.scrollTop = container.scrollHeight;
    } catch (e) {
      console.error(e);
    }
  }

  // Modals
  function openModal(id) {
    $(id).classList.remove('hidden');
  }

  function closeModal(id) {
    $(id).classList.add('hidden');
    if (id === 'player-modal') {
      const v = $('player-video');
      v.pause();
      v.removeAttribute('src');
      v.load();
    }
  }

  function openRematchModal(opts) {
    state.rematchTargetPath = opts.path;
    $('rematch-file-label').textContent = opts.path;
    $('rematch-type').value = opts.type || 'movie';
    $('rematch-query').value = opts.title || '';
    $('rematch-year').value = opts.year || '';
    $('rematch-imdb').value = '';
    $('rematch-season').value = opts.season ?? '';
    $('rematch-episode').value = opts.episode ?? '';
    $('rematch-results').innerHTML = '';
    syncRematchSeriesFields();
    openModal('rematch-modal');
    if (opts.title) runTmdbSearch();
  }

  function syncRematchSeriesFields() {
    const isSeries = $('rematch-type').value === 'series';
    $('rematch-season').classList.toggle('hidden', !isSeries);
    $('rematch-episode').classList.toggle('hidden', !isSeries);
  }

  async function runTmdbSearch() {
    const q = $('rematch-query').value.trim();
    const type = $('rematch-type').value;
    const year = $('rematch-year').value.trim();
    if (!q) return;

    const resultsEl = $('rematch-results');
    resultsEl.innerHTML = `<div class="muted">Searching TMDB…</div>`;
    try {
      const qs = new URLSearchParams({ q, type });
      if (year) qs.set('year', year);
      const data = await apiFetch(`/api/tmdb/search?${qs.toString()}`);
      if (!data.results.length) {
        resultsEl.innerHTML = `<div class="muted">No TMDB matches found. Try editing the search query or entering an IMDb ID below.</div>`;
        return;
      }
      resultsEl.innerHTML = data.results.map((r) => `
        <button type="button" class="tmdb-card" data-pick-tmdb="${r.tmdb_id}" data-pick-type="${escapeHtml(r.media_type)}">
          ${r.poster_url ? `<img src="${escapeHtml(r.poster_url)}" alt="">` : `<img alt="">`}
          <div>
            <div style="font-weight:600;font-size:13px">${escapeHtml(r.title)}</div>
            <div class="muted" style="font-size:11px">${r.year || '—'} · TMDB #${r.tmdb_id}</div>
          </div>
        </button>
      `).join('');
    } catch (e) {
      resultsEl.innerHTML = `<div style="color:var(--crimson)">TMDB search failed: ${escapeHtml(e.message)}</div>`;
    }
  }

  async function triggerRescan() {
    try {
      await apiFetch('/api/rescan', { method: 'POST' });
      toast('Full library rescan started', 'info');
      await loadOverview();
    } catch (e) {
      toast(`Rescan failed: ${e.message}`, 'err');
    }
  }

  async function clearCache() {
    try {
      await apiFetch('/api/cache/clear', { method: 'POST' });
      toast('TMDB metadata cache cleared', 'ok');
      await loadOverview();
    } catch (e) {
      toast(`Cache clear failed: ${e.message}`, 'err');
    }
  }

  function wireEvents() {
    $('mobile-menu-btn').addEventListener('click', () => {
      if ($('sidebar').classList.contains('open')) {
        closeSidebar();
      } else {
        openSidebar();
      }
    });
    $('sidebar-close-btn').addEventListener('click', closeSidebar);
    $('sidebar-backdrop').addEventListener('click', closeSidebar);

    document.querySelectorAll('.nav-item, .mobile-nav-item').forEach((btn) => {
      btn.addEventListener('click', () => switchView(btn.dataset.view));
    });

    document.addEventListener('keydown', (e) => {
      if (e.key === 'Escape') {
        closeSidebar();
        document.querySelectorAll('.modal:not(.hidden)').forEach((m) => closeModal(m.id));
      }
    });

    document.querySelectorAll('[data-media-tab]').forEach((btn) => {
      btn.addEventListener('click', () => {
        state.mediaTab = btn.dataset.mediaTab;
        document.querySelectorAll('[data-media-tab]').forEach((b) => b.classList.remove('active'));
        btn.classList.add('active');
        renderMedia();
      });
    });

    $('media-search').addEventListener('input', renderMedia);
    $('btn-media-refresh').addEventListener('click', loadMedia);
    $('btn-top-rescan').addEventListener('click', triggerRescan);
    $('btn-srv-rescan').addEventListener('click', triggerRescan);
    $('btn-refresh-overview').addEventListener('click', loadOverview);
    $('btn-ov-clear-cache').addEventListener('click', clearCache);
    $('btn-srv-clear-cache').addEventListener('click', clearCache);
    $('btn-ov-goto-logs').addEventListener('click', () => switchView('logs'));
    $('btn-ov-goto-config').addEventListener('click', () => switchView('server'));

    $('btn-copy-manifest').addEventListener('click', async () => {
      const ok = await copyToClipboard($('manifest-url-box').textContent);
      toast(ok ? 'Manifest URL copied to clipboard' : 'Could not copy automatically — select & copy URL', ok ? 'ok' : 'warn');
    });

    // Global delegated clicks for media/files/modals
    document.addEventListener('click', async (e) => {
      const closeTarget = e.target.closest('[data-close-modal]');
      if (closeTarget) {
        closeModal(closeTarget.dataset.closeModal);
        return;
      }

      const playBtn = e.target.closest('[data-play]');
      if (playBtn) {
        const url = playBtn.dataset.play;
        $('player-title').textContent = playBtn.dataset.title || 'Media Stream';
        $('player-url-box').textContent = url;
        $('player-video').src = url;
        openModal('player-modal');
        return;
      }

      const copyBtn = e.target.closest('[data-copy]');
      if (copyBtn) {
        const ok = await copyToClipboard(copyBtn.dataset.copy);
        toast(ok ? 'Stream URL copied' : 'Could not copy automatically', ok ? 'ok' : 'warn');
        return;
      }

      const rematchBtn = e.target.closest('[data-rematch]');
      if (rematchBtn) {
        openRematchModal({
          path: rematchBtn.dataset.rematch,
          type: rematchBtn.dataset.rtype,
          title: rematchBtn.dataset.rtitle,
          year: rematchBtn.dataset.ryear,
          season: rematchBtn.dataset.season,
          episode: rematchBtn.dataset.episode,
        });
        return;
      }

      const posterBtn = e.target.closest('[data-poster-file]');
      if (posterBtn) {
        state.posterTargetPath = posterBtn.dataset.posterFile;
        state.posterPayload = null;
        $('poster-file-label').textContent = state.posterTargetPath;
        $('poster-file-input').value = '';
        $('poster-preview-wrap').classList.add('hidden');
        $('btn-poster-upload').disabled = true;
        openModal('poster-modal');
        return;
      }

      const openDir = e.target.closest('[data-open-dir]');
      if (openDir) {
        loadFiles(openDir.dataset.openDir);
        return;
      }

      const crumb = e.target.closest('.crumb[data-path]');
      if (crumb) {
        loadFiles(crumb.dataset.path);
        return;
      }

      const rescanFileBtn = e.target.closest('[data-rescan-file]');
      if (rescanFileBtn) {
        try {
          const res = await apiFetch('/api/media/rescan-file', {
            method: 'POST',
            body: JSON.stringify({ path: rescanFileBtn.dataset.rescanFile }),
          });
          toast(res.matched ? 'File matched and indexed' : 'File re-scanned (no TMDB match)', res.matched ? 'ok' : 'warn');
          await Promise.all([loadFiles(state.currentRelPath), loadMedia(), loadOverview()]);
        } catch (err) {
          toast(`Rescan error: ${err.message}`, 'err');
        }
        return;
      }

      const renameBtn = e.target.closest('[data-rename-path]');
      if (renameBtn) {
        const cur = renameBtn.dataset.renameCur;
        const next = prompt(`Rename "${cur}" to:`, cur);
        if (!next || next.trim() === cur) return;
        try {
          await apiFetch('/api/files/rename', {
            method: 'POST',
            body: JSON.stringify({ path: renameBtn.dataset.renamePath, new_name: next.trim() }),
          });
          toast('Renamed successfully', 'ok');
          await Promise.all([loadFiles(state.currentRelPath), loadMedia(), loadOverview()]);
        } catch (err) {
          toast(`Rename failed: ${err.message}`, 'err');
        }
        return;
      }

      const tmdbPick = e.target.closest('[data-pick-tmdb]');
      if (tmdbPick) {
        try {
          const meta = await apiFetch(
            `/api/tmdb/resolve?tmdb_id=${tmdbPick.dataset.pickTmdb}&type=${tmdbPick.dataset.pickType}`
          );
          $('rematch-imdb').value = meta.imdb_id;
          toast(`Selected ${meta.title || meta.imdb_id} (${meta.imdb_id})`, 'ok');
        } catch (err) {
          toast(`Could not resolve IMDb ID: ${err.message}`, 'err');
        }
      }
    });

    $('btn-files-up').addEventListener('click', () => {
      loadFiles($('btn-files-up').dataset.parent || '');
    });
    $('btn-files-refresh').addEventListener('click', () => loadFiles(state.currentRelPath));

    $('btn-copy-stream').addEventListener('click', async () => {
      const ok = await copyToClipboard($('player-url-box').textContent);
      toast(ok ? 'Stream URL copied' : 'Could not copy automatically', ok ? 'ok' : 'warn');
    });

    $('rematch-type').addEventListener('change', syncRematchSeriesFields);
    $('btn-tmdb-search').addEventListener('click', runTmdbSearch);
    $('rematch-query').addEventListener('keydown', (e) => {
      if (e.key === 'Enter') {
        e.preventDefault();
        runTmdbSearch();
      }
    });

    $('btn-rematch-apply').addEventListener('click', async () => {
      const imdb_id = $('rematch-imdb').value.trim();
      if (!imdb_id) {
        toast('Select a TMDB match or enter an IMDb ID (e.g. tt0903747)', 'err');
        return;
      }
      const content_type = $('rematch-type').value;
      const seasonRaw = $('rematch-season').value.trim();
      const epRaw = $('rematch-episode').value.trim();
      try {
        await apiFetch('/api/media/rematch', {
          method: 'POST',
          body: JSON.stringify({
            path: state.rematchTargetPath,
            imdb_id,
            content_type,
            season: seasonRaw ? Number(seasonRaw) : null,
            episode: epRaw ? Number(epRaw) : null,
            write_imdb_tag: $('rematch-write-tag').checked,
          }),
        });
        closeModal('rematch-modal');
        toast(`Matched to ${imdb_id}`, 'ok');
        await Promise.all([loadMedia(), loadOverview()]);
        if (state.activeView === 'files') loadFiles(state.currentRelPath);
      } catch (err) {
        toast(`Rematch failed: ${err.message}`, 'err');
      }
    });

    $('poster-file-input').addEventListener('change', (e) => {
      const file = e.target.files[0];
      if (!file) return;
      const reader = new FileReader();
      reader.onload = () => {
        state.posterPayload = reader.result;
        $('poster-preview-img').src = reader.result;
        $('poster-preview-wrap').classList.remove('hidden');
        $('btn-poster-upload').disabled = false;
      };
      reader.readAsDataURL(file);
    });

    $('btn-poster-upload').addEventListener('click', async () => {
      if (!state.posterPayload) return;
      try {
        await apiFetch('/api/files/poster', {
          method: 'POST',
          body: JSON.stringify({
            video_path: state.posterTargetPath,
            data_base64: state.posterPayload,
          }),
        });
        closeModal('poster-modal');
        toast('Sidecar poster saved', 'ok');
        await loadMedia();
      } catch (err) {
        toast(`Poster upload failed: ${err.message}`, 'err');
      }
    });

    $('config-form').addEventListener('submit', async (e) => {
      e.preventDefault();
      const payload = {
        media_path: $('cfg-media-path').value.trim(),
        port: Number($('cfg-port').value),
        base_url: $('cfg-base-url').value.trim(),
        public_url: $('cfg-public-url').value.trim(),
        poster_url: $('cfg-poster-url').value.trim(),
        scan_cron: $('cfg-scan-cron').value.trim(),
        unmatched_catalog: $('cfg-unmatched').checked,
        clear_password: $('cfg-clear-pw').checked,
      };
      const apiKey = $('cfg-tmdb-key').value.trim();
      if (apiKey) payload.tmdb_api_key = apiKey;
      const pw = $('cfg-password').value.trim();
      if (pw && !payload.clear_password) payload.password = pw;

      try {
        const res = await apiFetch('/api/config', {
          method: 'PUT',
          body: JSON.stringify(payload),
        });
        toast(res.message || 'Configuration saved to .env', 'ok');
        if (!res.media_path_exists) {
          toast('Warning: configured MEDIA_PATH does not currently exist on disk', 'warn');
        }
        loadConfig();
      } catch (err) {
        toast(`Save failed: ${err.message}`, 'err');
      }
    });

    document.querySelectorAll('.preset-chip[data-cron]').forEach((btn) => {
      btn.addEventListener('click', () => {
        $('cfg-scan-cron').value = btn.dataset.cron;
      });
    });

    $('btn-srv-restart').addEventListener('click', async () => {
      if (!confirm('Restart the Lanio server now with the saved .env settings?')) return;
      try {
        await apiFetch('/api/server/restart', { method: 'POST' });
        toast('Server restarting… reconnecting in 3s', 'info');
        setTimeout(() => window.location.reload(), 2800);
      } catch (err) {
        toast(`Restart failed: ${err.message}`, 'err');
      }
    });

    $('btn-srv-shutdown').addEventListener('click', async () => {
      if (!confirm('Stop the Lanio server process? You will need to start it again from your terminal or service manager.')) return;
      try {
        await apiFetch('/api/server/shutdown', { method: 'POST' });
        toast('Server shutting down…', 'warn');
      } catch (err) {
        toast(`Shutdown failed: ${err.message}`, 'err');
      }
    });

    $('log-level-tabs').addEventListener('click', (e) => {
      const btn = e.target.closest('[data-level]');
      if (!btn) return;
      state.logLevel = btn.dataset.level;
      document.querySelectorAll('#log-level-tabs .tab-btn').forEach((b) => b.classList.remove('active'));
      btn.classList.add('active');
      loadLogs();
    });
    $('log-filter-text').addEventListener('input', loadLogs);
    $('btn-logs-clear').addEventListener('click', async () => {
      await apiFetch('/api/logs', { method: 'DELETE' });
      loadLogs();
      toast('Logs cleared', 'info');
    });
  }

  function startPollers() {
    setInterval(() => {
      if (AUTH_REQUIRED && !state.token) return;
      loadOverview();
      if (state.activeView === 'logs' && $('log-auto-refresh').checked) {
        loadLogs();
      }
    }, 3500);
  }

  wireEvents();
  initAuth();
  startPollers();
})();
