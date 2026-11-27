# Lanio

A Stremio add-on that serves your local media files as streams over your Local Area Network (LAN). Point it at a directory of movies and TV shows, and they'll appear as stream options inside Stremio alongside any other sources you use.

Built with Rust to be hyper efficient with a low footprint on your server!

> **Looking for the native Windows desktop app?**
> Check out **[LanioWindows (`mayank1053/LanioWindows`)](https://github.com/mayank1053/LanioWindows)** — a companion Windows desktop & system-tray edition built with Rust, Axum, and Tauri v2. It adds a Windows setup wizard, multi-folder libraries, system tray & autostart, smart LAN IPv4 detection with offline QR pairing codes, `lanio.local` mDNS discovery, one-click Windows Firewall setup, active-stream Sleep Guard, Cloudflare DoH TMDB DNS fallback, and a phone-browser remote with file uploads, folder management, batch rename, and Recycle Bin delete.

## How it works

On startup the server scans your media directory, parses filenames to extract title/year/episode info, and looks each one up via the TMDB API to get its IMDb ID. Stremio identifies content by IMDb ID, so once a file is matched it shows up as a stream option whenever you open that movie or episode.

After this initial scan, a file watcher watches the directory for any changes and makes adjustments in real time.

## What Was Added in This Fork

### 1. Screening Room & Control Console (`/`)
Replaced the minimal landing pages with a full desktop- and mobile-responsive web console (`src/templates/console.html`, `console.css`, `console.js`):
- **Overview & Add-on**: Live library counts (movies, TV series/episodes, unmatched/skipped files), media footprint, scan duration, watcher/uptime status, and one-click Stremio Web / Stremio App manifest install links.
- **Media Library**: Filterable views for **Movies**, **TV Series**, **Unmatched**, and **Skipped** files, with manual **TMDB Rematching** (search TMDB or resolve by `tt...` IMDb ID, with optional `[tt...]` filename tagging) and per-file rescanning.
- **File Explorer**: Directory browser scoped safely under `MEDIA_PATH` (with path-traversal protection), inline file renaming, and sidecar poster uploads (`.jpg`, `.png`, `.webp`).
- **Server & Config**: View and save `.env` settings (`MEDIA_PATH`, `PORT`, `BASE_URL`, `PUBLIC_URL`, `POSTER_URL`, `SCAN_CRON` presets, `TMDB_API_KEY`, `PASSWORD`, `UNMATCHED_CATALOG`), trigger rescans, clear the TMDB cache, or restart/stop the server.
- **Live Logs**: Real-time log tailing backed by an in-memory 400-entry ring buffer (`src/telemetry.rs`), with level filters (`ALL`, `INFO`, `WARN`, `ERROR`, `DEBUG`) and text search.
- **Console Auth**: Password lock screen and API token verification (`X-Lanio-Token`, `Authorization: Bearer`, or `?token=`) when `PASSWORD` is configured.

### 2. Management REST API (`/api/*`)
Added authenticated JSON management endpoints in `src/routes/api.rs` (with end-to-end tests in `tests/management_api_e2e.rs`):
- `GET /api/overview`, `GET /api/media`, `POST /api/media/rematch`, `POST /api/media/rescan-file`
- `GET /api/files`, `POST /api/files/rename`, `POST /api/files/poster`
- `GET /api/tmdb/search`, `GET /api/tmdb/resolve`
- `POST /api/rescan`, `POST /api/cache/clear`
- `GET` / `PUT /api/config`, `POST /api/server/restart`, `POST /api/server/shutdown`
- `GET` / `DELETE /api/logs`

### 3. Persistent TMDB Disk Cache
- Layered a **30-day persistent JSON disk cache** (`.lanio-tmdb-cache.json` by default, or `TMDB_CACHE_PATH`) underneath the 24-hour in-memory Moka cache (`src/metadata/mod.rs`), avoiding redundant TMDB lookups across restarts and rescans.

### 4. Windows Launcher & Scanner/Index Fixes
- **`start-lanio.ps1`**: PowerShell launcher that reads `.env`, validates `MEDIA_PATH`, builds the release binary if missing, and launches `lanio.exe`.
- **Windows Path & Watcher Reliability**: Normalized Windows `\\?\` paths and canonical comparisons in `src/index/mod.rs` and `src/scanner/mod.rs` to prevent duplicate episode entries on in-place edits and cleanly purge deleted dotted show folders (e.g. `Breaking.Bad.2008`).
- **Scan Telemetry**: Added `ScanReport` timing/counts, `skipped` file tracking, and `watcher_active` status reporting.

## Quick Start

1. Create a `.env` file in the project root with at least `MEDIA_PATH` and `TMDB_API_KEY` (plus optional `PORT`, `BASE_URL`, `PUBLIC_URL`, `POSTER_URL`, `PASSWORD`, `SCAN_CRON`, `UNMATCHED_CATALOG`, `TMDB_CACHE_PATH`).
2. Run on Windows via PowerShell:
   ```powershell
   .\start-lanio.ps1
   ```
   Or build/run directly with Cargo:
   ```powershell
   cargo run --release
   ```
3. Open `http://localhost:8078` (or your configured `PORT` / LAN IP) to access the Screening Room & Control Console and install the manifest into Stremio.

Run the test suite with:
```powershell
cargo test
```

## Deployment and Installation

Please see [the wiki](https://github.com/Legume0704/lanio/wiki) for upstream Docker and deployment guides, or use **[LanioWindows](https://github.com/mayank1053/LanioWindows)** if you want a native Windows desktop & tray application.

## License

MIT
