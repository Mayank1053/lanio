use crate::auth::compute_token;
use crate::config::Config;
use crate::index::types::ContentType;
use crate::routes::poster::poster_url as build_poster_url;
use crate::routes::stream::authenticated_url;
use crate::scanner::{poster_sidecar, MediaScanner};
use crate::telemetry::global_log_store;
use axum::{
    extract::{Query, Request, State},
    http::{header, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use base64::{engine::general_purpose, Engine as _};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::{Component, Path as FsPath, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;

#[derive(Clone)]
pub struct ApiState {
    pub scanner: Arc<MediaScanner>,
    pub config: Arc<Config>,
}

pub async fn api_auth(
    State(config): State<Arc<Config>>,
    request: Request,
    next: Next,
) -> Response {
    let Some(ref expected) = config.auth_token else {
        return next.run(request).await;
    };

    let header_token = request
        .headers()
        .get("x-lanio-token")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .map(str::to_string);

    let bearer_token = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim)
        .map(str::to_string);

    let query_token = request.uri().query().and_then(|q| {
        q.split('&').find_map(|pair| {
            let (k, v) = pair.split_once('=')?;
            if k == "token" {
                Some(v.to_string())
            } else {
                None
            }
        })
    });

    let provided = header_token.or(bearer_token).or(query_token);
    match provided {
        Some(t) if t == expected.as_str() => next.run(request).await,
        _ => (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": "Unauthorized: invalid or missing token" })),
        )
            .into_response(),
    }
}

fn server_base_url(config: &Config) -> String {
    config
        .public_url
        .as_ref()
        .or(config.base_url.as_ref())
        .map(|s| s.trim_end_matches('/').to_string())
        .unwrap_or_else(|| format!("http://localhost:{}", config.port))
}

fn manifest_url_for(config: &Config) -> String {
    let base = server_base_url(config);
    match config.auth_token {
        Some(ref token) => format!("{}/{}/manifest.json", base, token),
        None => format!("{}/manifest.json", base),
    }
}

fn stream_url_for_path(config: &Config, file_path: &FsPath) -> String {
    let encoded_path =
        general_purpose::URL_SAFE_NO_PAD.encode(file_path.to_string_lossy().as_bytes());
    authenticated_url(config, &format!("video?path={}", encoded_path))
}

fn file_size_bytes(path: &FsPath) -> Option<u64> {
    std::fs::metadata(path).ok().map(|m| m.len())
}

fn file_extension(path: &FsPath) -> Option<String> {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
}

fn resolve_under_media_root(media_root: &FsPath, raw_path: &str) -> Result<PathBuf, (StatusCode, Json<serde_json::Value>)> {
    let root = media_root.canonicalize().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": format!("MEDIA_PATH unavailable: {}", e) })),
        )
    })?;

    let trimmed = raw_path.trim();
    let candidate = if trimmed.is_empty() {
        root.clone()
    } else {
        let p = FsPath::new(trimmed);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            let mut joined = root.clone();
            for comp in p.components() {
                match comp {
                    Component::Normal(seg) => joined.push(seg),
                    Component::CurDir => {}
                    _ => {
                        return Err((
                            StatusCode::BAD_REQUEST,
                            Json(serde_json::json!({ "error": "Invalid path traversal component" })),
                        ));
                    }
                }
            }
            joined
        }
    };

    let resolved = candidate.canonicalize().map_err(|e| {
        (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": format!("Path not found: {}", e) })),
        )
    })?;

    if !resolved.starts_with(&root) {
        return Err((
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": "Path escapes MEDIA_PATH" })),
        ));
    }

    Ok(resolved)
}

#[derive(Serialize)]
pub struct OverviewResponse {
    pub version: String,
    pub uptime_secs: i64,
    pub started_at: String,
    pub port: u16,
    pub base_url: Option<String>,
    pub public_url: Option<String>,
    pub manifest_url: String,
    pub install_url: String,
    pub media_path: String,
    pub media_path_exists: bool,
    pub auth_enabled: bool,
    pub unmatched_catalog: bool,
    pub poster_url_template: Option<String>,
    pub scan_cron: Option<String>,
    pub next_scheduled_scan: Option<String>,
    pub scanning: bool,
    pub pending_rescan: bool,
    pub watcher_active: bool,
    pub last_scan: Option<crate::scanner::ScanReport>,
    pub tmdb_cache_entries: u64,
    pub counts: crate::index::IndexCounts,
    pub total_size_bytes: u64,
}

pub async fn overview_handler(State(state): State<ApiState>) -> Json<OverviewResponse> {
    let counts = state.scanner.index.counts();
    let movies = state.scanner.index.get_all_movies();
    let series = state.scanner.index.get_all_series_grouped();
    let unmatched = state.scanner.index.get_all_unmatched();
    let skipped = state.scanner.index.get_all_skipped();

    let mut total_size_bytes = 0u64;
    for (_, m) in &movies {
        total_size_bytes += file_size_bytes(&m.file_path).unwrap_or(0);
    }
    for (_, eps) in &series {
        for ep in eps {
            total_size_bytes += file_size_bytes(&ep.file_path).unwrap_or(0);
        }
    }
    for u in &unmatched {
        total_size_bytes += file_size_bytes(&u.file_path).unwrap_or(0);
    }
    for s in &skipped {
        total_size_bytes += file_size_bytes(&s.file_path).unwrap_or(0);
    }

    let manifest_url = manifest_url_for(&state.config);
    let install_url = format!(
        "https://web.stremio.com/#/addons?addon={}",
        urlencoding::encode(&manifest_url)
    );

    let next_scheduled_scan = state
        .config
        .parsed_scan_cron()
        .ok()
        .flatten()
        .and_then(|schedule| schedule.upcoming(chrono::Local).next())
        .map(|dt| dt.to_rfc3339());

    let uptime_secs = (chrono::Utc::now() - state.scanner.started_at).num_seconds().max(0);
    let last_scan = state.scanner.last_scan.read().await.clone();
    let tmdb_cache_entries = state.scanner.tmdb_client.cache_count().await;

    Json(OverviewResponse {
        version: env!("CARGO_PKG_VERSION").to_string(),
        uptime_secs,
        started_at: state
            .scanner
            .started_at
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        port: state.config.port,
        base_url: state.config.base_url.clone(),
        public_url: state.config.public_url.clone(),
        manifest_url,
        install_url,
        media_path: state.config.media_path.to_string_lossy().to_string(),
        media_path_exists: state.config.media_path.exists(),
        auth_enabled: state.config.auth_token.is_some(),
        unmatched_catalog: state.config.unmatched_catalog,
        poster_url_template: state.config.poster_url.clone(),
        scan_cron: state.config.scan_cron.clone(),
        next_scheduled_scan,
        scanning: state.scanner.scanning.load(Ordering::SeqCst),
        pending_rescan: state.scanner.pending_rescan.load(Ordering::SeqCst),
        watcher_active: state.scanner.watcher_active.load(Ordering::SeqCst),
        last_scan,
        tmdb_cache_entries,
        counts,
        total_size_bytes,
    })
}

#[derive(Serialize)]
pub struct MovieItem {
    pub imdb_id: String,
    pub title: String,
    pub year: Option<u16>,
    pub poster: Option<String>,
    pub description: Option<String>,
    pub tmdb_rating: Option<f64>,
    pub tmdb_votes: Option<u32>,
    pub file_path: String,
    pub file_name: String,
    pub size_bytes: Option<u64>,
    pub extension: Option<String>,
    pub stream_url: String,
}

#[derive(Serialize)]
pub struct EpisodeItem {
    pub season: Option<u16>,
    pub episode: Option<u16>,
    pub file_path: String,
    pub file_name: String,
    pub size_bytes: Option<u64>,
    pub extension: Option<String>,
    pub stream_url: String,
}

#[derive(Serialize)]
pub struct SeriesItem {
    pub imdb_id: String,
    pub title: String,
    pub year: Option<u16>,
    pub poster: Option<String>,
    pub description: Option<String>,
    pub tmdb_rating: Option<f64>,
    pub tmdb_votes: Option<u32>,
    pub episode_count: usize,
    pub total_size_bytes: u64,
    pub episodes: Vec<EpisodeItem>,
}

#[derive(Serialize)]
pub struct UnmatchedItem {
    pub id: String,
    pub title: String,
    pub file_path: String,
    pub size_bytes: Option<u64>,
    pub extension: Option<String>,
    pub created: Option<u64>,
    pub has_sidecar: bool,
    pub poster_url: Option<String>,
    pub stream_url: String,
}

#[derive(Serialize)]
pub struct MediaLibraryResponse {
    pub movies: Vec<MovieItem>,
    pub series: Vec<SeriesItem>,
    pub unmatched: Vec<UnmatchedItem>,
    pub skipped: Vec<UnmatchedItem>,
}

pub async fn media_handler(State(state): State<ApiState>) -> Json<MediaLibraryResponse> {
    let mut movies: Vec<MovieItem> = state
        .scanner
        .index
        .get_all_movies()
        .into_iter()
        .map(|(imdb_id, info)| {
            let poster = state.config.poster_url_for(&imdb_id).or(info.poster.clone());
            let file_name = info
                .file_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_string();
            MovieItem {
                imdb_id,
                title: info.title,
                year: info.year,
                poster,
                description: info.description,
                tmdb_rating: info.tmdb_rating,
                tmdb_votes: info.tmdb_votes,
                size_bytes: file_size_bytes(&info.file_path),
                extension: file_extension(&info.file_path),
                stream_url: stream_url_for_path(&state.config, &info.file_path),
                file_path: info.file_path.to_string_lossy().to_string(),
                file_name,
            }
        })
        .collect();
    movies.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));

    let mut series: Vec<SeriesItem> = state
        .scanner
        .index
        .get_all_series_grouped()
        .into_iter()
        .filter_map(|(imdb_id, eps)| {
            let first = eps.first()?.clone();
            let poster = state.config.poster_url_for(&imdb_id).or(first.poster.clone());
            let mut total_size = 0u64;
            let episodes: Vec<EpisodeItem> = eps
                .into_iter()
                .map(|ep| {
                    let sz = file_size_bytes(&ep.file_path);
                    total_size += sz.unwrap_or(0);
                    let file_name = ep
                        .file_path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or_default()
                        .to_string();
                    EpisodeItem {
                        season: ep.parsed.season,
                        episode: ep.parsed.episode,
                        size_bytes: sz,
                        extension: file_extension(&ep.file_path),
                        stream_url: stream_url_for_path(&state.config, &ep.file_path),
                        file_path: ep.file_path.to_string_lossy().to_string(),
                        file_name,
                    }
                })
                .collect();
            Some(SeriesItem {
                imdb_id,
                title: first.title,
                year: first.year,
                poster,
                description: first.description,
                tmdb_rating: first.tmdb_rating,
                tmdb_votes: first.tmdb_votes,
                episode_count: episodes.len(),
                total_size_bytes: total_size,
                episodes,
            })
        })
        .collect();
    series.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));

    let map_unmatched = |u: crate::index::types::UnmatchedFile, allow_poster_route: bool| {
        let has_sidecar = poster_sidecar(&u.file_path).is_some();
        let poster_url = if allow_poster_route {
            build_poster_url(&state.config, &u)
        } else {
            None
        };
        UnmatchedItem {
            id: u.id,
            title: u.title,
            size_bytes: file_size_bytes(&u.file_path),
            extension: file_extension(&u.file_path),
            created: u.created,
            has_sidecar,
            poster_url,
            stream_url: stream_url_for_path(&state.config, &u.file_path),
            file_path: u.file_path.to_string_lossy().to_string(),
        }
    };

    let unmatched = state
        .scanner
        .index
        .get_all_unmatched()
        .into_iter()
        .map(|u| map_unmatched(u, state.config.unmatched_catalog))
        .collect();

    let skipped = state
        .scanner
        .index
        .get_all_skipped()
        .into_iter()
        .map(|u| map_unmatched(u, false))
        .collect();

    Json(MediaLibraryResponse {
        movies,
        series,
        unmatched,
        skipped,
    })
}

#[derive(Deserialize)]
pub struct FilesQuery {
    #[serde(default)]
    pub path: String,
}

#[derive(Serialize)]
pub struct FileEntryItem {
    pub name: String,
    pub relative_path: String,
    pub full_path: String,
    pub is_dir: bool,
    pub size_bytes: Option<u64>,
    pub extension: Option<String>,
    pub kind: String,
    pub index_status: String,
    pub imdb_id: Option<String>,
    pub matched_title: Option<String>,
    pub stream_url: Option<String>,
}

#[derive(Serialize)]
pub struct FilesResponse {
    pub root: String,
    pub current_relative: String,
    pub parent_relative: Option<String>,
    pub entries: Vec<FileEntryItem>,
}

pub async fn files_handler(
    Query(query): Query<FilesQuery>,
    State(state): State<ApiState>,
) -> Result<Json<FilesResponse>, (StatusCode, Json<serde_json::Value>)> {
    let root = state.config.media_path.canonicalize().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": format!("MEDIA_PATH unavailable: {}", e) })),
        )
    })?;
    let current = resolve_under_media_root(&state.config.media_path, &query.path)?;
    if !current.is_dir() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Target is not a directory" })),
        ));
    }

    // Build quick path lookup maps for index status
    let mut matched_map: HashMap<PathBuf, (String, String)> = HashMap::new();
    for (imdb_id, m) in state.scanner.index.get_all_movies() {
        matched_map.insert(m.file_path, (imdb_id, m.title));
    }
    for (imdb_id, eps) in state.scanner.index.get_all_series_grouped() {
        for ep in eps {
            matched_map.insert(ep.file_path, (imdb_id.clone(), ep.title));
        }
    }
    let unmatched_set: HashMap<PathBuf, String> = state
        .scanner
        .index
        .get_all_unmatched()
        .into_iter()
        .map(|u| (u.file_path, u.id))
        .collect();
    let skipped_set: HashMap<PathBuf, String> = state
        .scanner
        .index
        .get_all_skipped()
        .into_iter()
        .map(|u| (u.file_path, u.id))
        .collect();

    let mut read_dir = tokio::fs::read_dir(&current).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Failed to read directory: {}", e) })),
        )
    })?;

    let video_exts = [
        "mkv", "mp4", "avi", "mov", "wmv", "flv", "webm", "m4v", "mpg", "mpeg", "m2ts", "ts",
        "vob",
    ];
    let image_exts = ["jpg", "jpeg", "png", "webp"];
    let sub_exts = ["srt", "vtt", "ass", "ssa", "sub", "idx"];

    let mut entries = Vec::new();
    while let Ok(Some(entry)) = read_dir.next_entry().await {
        let p = entry.path();
        let canonical = p.canonicalize().unwrap_or_else(|_| p.clone());
        let meta = entry.metadata().await.ok();
        let is_dir = meta.as_ref().map(|m| m.is_dir()).unwrap_or(false);
        let size_bytes = if is_dir {
            None
        } else {
            meta.as_ref().map(|m| m.len())
        };
        let name = entry.file_name().to_string_lossy().to_string();
        let rel = canonical
            .strip_prefix(&root)
            .unwrap_or(&canonical)
            .to_string_lossy()
            .replace('\\', "/");
        let ext = if is_dir {
            None
        } else {
            file_extension(&canonical)
        };
        let kind = if is_dir {
            "dir".to_string()
        } else if ext
            .as_deref()
            .is_some_and(|e| video_exts.contains(&e))
        {
            "video".to_string()
        } else if ext
            .as_deref()
            .is_some_and(|e| image_exts.contains(&e))
        {
            "image".to_string()
        } else if ext.as_deref().is_some_and(|e| sub_exts.contains(&e)) {
            "subtitle".to_string()
        } else {
            "other".to_string()
        };

        let (index_status, imdb_id, matched_title) = if is_dir {
            ("dir".to_string(), None, None)
        } else if let Some((id, title)) = matched_map.get(&canonical) {
            ("matched".to_string(), Some(id.clone()), Some(title.clone()))
        } else if unmatched_set.contains_key(&canonical) {
            ("unmatched".to_string(), None, None)
        } else if skipped_set.contains_key(&canonical) {
            ("skipped".to_string(), None, None)
        } else if kind == "video" {
            ("unindexed".to_string(), None, None)
        } else {
            ("asset".to_string(), None, None)
        };

        let stream_url = if kind == "video" {
            Some(stream_url_for_path(&state.config, &canonical))
        } else {
            None
        };

        entries.push(FileEntryItem {
            name,
            relative_path: rel,
            full_path: canonical.to_string_lossy().to_string(),
            is_dir,
            size_bytes,
            extension: ext,
            kind,
            index_status,
            imdb_id,
            matched_title,
            stream_url,
        });
    }

    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    let current_relative = current
        .strip_prefix(&root)
        .unwrap_or(FsPath::new(""))
        .to_string_lossy()
        .replace('\\', "/");

    let parent_relative = if current == root {
        None
    } else {
        current
            .parent()
            .and_then(|p| p.strip_prefix(&root).ok())
            .map(|p| p.to_string_lossy().replace('\\', "/"))
    };

    Ok(Json(FilesResponse {
        root: root.to_string_lossy().to_string(),
        current_relative,
        parent_relative,
        entries,
    }))
}

#[derive(Deserialize)]
pub struct RenameRequest {
    pub path: String,
    pub new_name: String,
}

pub async fn rename_file_handler(
    State(state): State<ApiState>,
    Json(payload): Json<RenameRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let new_name = payload.new_name.trim();
    if new_name.is_empty()
        || new_name.contains('/')
        || new_name.contains('\\')
        || new_name == "."
        || new_name == ".."
    {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Invalid filename" })),
        ));
    }

    let resolved = resolve_under_media_root(&state.config.media_path, &payload.path)?;
    let root = state.config.media_path.canonicalize().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
    })?;
    if resolved == root {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Cannot rename MEDIA_PATH root" })),
        ));
    }

    let target = resolved.with_file_name(new_name);
    if target.exists() {
        return Err((
            StatusCode::CONFLICT,
            Json(serde_json::json!({ "error": "A file or folder with that name already exists" })),
        ));
    }

    let is_dir = resolved.is_dir();
    if let Err(e) = tokio::fs::rename(&resolved, &target).await {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Rename failed: {}", e) })),
        ));
    }

    if is_dir {
        state.scanner.index.remove_by_dir(&resolved);
        let scanner = Arc::clone(&state.scanner);
        tokio::spawn(async move {
            let _ = scanner.scan().await;
        });
    } else {
        state.scanner.index.remove_by_path(&resolved);
        let _ = state.scanner.rescan_single_file(&target).await;
    }

    Ok(Json(serde_json::json!({
        "status": "ok",
        "new_path": target.to_string_lossy(),
    })))
}

#[derive(Deserialize)]
pub struct PosterUploadRequest {
    pub video_path: String,
    pub extension: Option<String>,
    pub data_base64: String,
}

pub async fn upload_poster_handler(
    State(state): State<ApiState>,
    Json(payload): Json<PosterUploadRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let video_path = resolve_under_media_root(&state.config.media_path, &payload.video_path)?;
    if !video_path.is_file() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Video file does not exist" })),
        ));
    }

    let (detected_ext, raw_b64) = if let Some((prefix, rest)) = payload.data_base64.split_once(',') {
        let ext = if prefix.contains("image/png") {
            "png"
        } else if prefix.contains("image/webp") {
            "webp"
        } else {
            "jpg"
        };
        (ext, rest)
    } else {
        ("jpg", payload.data_base64.as_str())
    };

    let ext = payload
        .extension
        .as_deref()
        .map(|e| e.trim().trim_start_matches('.').to_ascii_lowercase())
        .filter(|e| matches!(e.as_str(), "jpg" | "jpeg" | "png" | "webp"))
        .unwrap_or_else(|| detected_ext.to_string());

    let bytes = general_purpose::STANDARD
        .decode(raw_b64.trim())
        .map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "Invalid base64 image payload" })),
            )
        })?;

    if bytes.is_empty() || bytes.len() > 8 * 1024 * 1024 {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Image must be between 1 byte and 8 MB" })),
        ));
    }

    let poster_path = video_path.with_extension(&ext);
    tokio::fs::write(&poster_path, &bytes).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Failed to save poster: {}", e) })),
        )
    })?;

    Ok(Json(serde_json::json!({
        "status": "ok",
        "poster_path": poster_path.to_string_lossy(),
    })))
}

#[derive(Deserialize)]
pub struct PathRequest {
    pub path: String,
}

pub async fn rescan_file_handler(
    State(state): State<ApiState>,
    Json(payload): Json<PathRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let resolved = resolve_under_media_root(&state.config.media_path, &payload.path)?;
    match state.scanner.rescan_single_file(&resolved).await {
        Ok(matched) => Ok(Json(serde_json::json!({
            "status": "ok",
            "matched": matched,
        }))),
        Err(e) => Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        )),
    }
}

#[derive(Deserialize)]
pub struct RematchRequest {
    pub path: String,
    pub imdb_id: String,
    pub content_type: String,
    pub season: Option<u16>,
    pub episode: Option<u16>,
    #[serde(default)]
    pub write_imdb_tag: bool,
}

pub async fn rematch_handler(
    State(state): State<ApiState>,
    Json(payload): Json<RematchRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let resolved = resolve_under_media_root(&state.config.media_path, &payload.path)?;
    let content_type = if payload.content_type.eq_ignore_ascii_case("series")
        || payload.content_type.eq_ignore_ascii_case("tv")
    {
        ContentType::Series
    } else {
        ContentType::Movie
    };

    match state
        .scanner
        .rematch_file(
            &resolved,
            &payload.imdb_id,
            content_type,
            payload.season,
            payload.episode,
            payload.write_imdb_tag,
        )
        .await
    {
        Ok(info) => Ok(Json(serde_json::json!({
            "status": "ok",
            "file_info": info,
        }))),
        Err(e) => Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        )),
    }
}

#[derive(Deserialize)]
pub struct TmdbSearchQuery {
    pub q: String,
    #[serde(rename = "type", default = "default_movie")]
    pub media_type: String,
    pub year: Option<u16>,
}

fn default_movie() -> String {
    "movie".to_string()
}

pub async fn tmdb_search_handler(
    Query(query): Query<TmdbSearchQuery>,
    State(state): State<ApiState>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let q = query.q.trim();
    if q.is_empty() {
        return Ok(Json(serde_json::json!({ "results": [] })));
    }
    match state
        .scanner
        .tmdb_client
        .search_candidates(q, &query.media_type, query.year)
        .await
    {
        Ok(results) => Ok(Json(serde_json::json!({ "results": results }))),
        Err(e) => Err((
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({ "error": e.to_string() })),
        )),
    }
}

#[derive(Deserialize)]
pub struct TmdbResolveQuery {
    pub tmdb_id: u64,
    #[serde(rename = "type", default = "default_movie")]
    pub media_type: String,
}

pub async fn tmdb_resolve_handler(
    Query(query): Query<TmdbResolveQuery>,
    State(state): State<ApiState>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    match state
        .scanner
        .tmdb_client
        .resolve_candidate(query.tmdb_id, &query.media_type)
        .await
    {
        Ok(Some(meta)) => Ok(Json(serde_json::json!({
            "imdb_id": meta.imdb_id,
            "title": meta.title,
            "year": meta.year,
            "overview": meta.overview,
            "poster_url": meta.poster_url,
            "tmdb_rating": meta.tmdb_rating,
            "tmdb_votes": meta.tmdb_votes,
        }))),
        Ok(None) => Err((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Selected TMDB entry has no IMDb ID" })),
        )),
        Err(e) => Err((
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({ "error": e.to_string() })),
        )),
    }
}

pub async fn clear_cache_handler(State(state): State<ApiState>) -> Json<serde_json::Value> {
    state.scanner.tmdb_client.clear_cache();
    tracing::info!("TMDB metadata cache cleared from GUI");
    Json(serde_json::json!({
        "status": "ok",
        "message": "TMDB metadata cache cleared"
    }))
}

fn mask_secret(value: &str) -> String {
    let len = value.chars().count();
    if len <= 6 {
        "*".repeat(len.max(4))
    } else {
        let prefix: String = value.chars().take(4).collect();
        let suffix: String = value.chars().skip(len - 4).collect();
        format!("{}...{}", prefix, suffix)
    }
}

fn env_file_path() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(".env")
}

#[derive(Serialize)]
pub struct ConfigResponse {
    pub media_path: String,
    pub port: u16,
    pub base_url: String,
    pub public_url: String,
    pub tmdb_api_key_masked: String,
    pub has_tmdb_api_key: bool,
    pub password_set: bool,
    pub poster_url: String,
    pub scan_cron: String,
    pub unmatched_catalog: bool,
    pub tmdb_base_url: String,
    pub tmdb_image_base_url: String,
    pub env_file_path: String,
    pub env_file_exists: bool,
}

pub async fn get_config_handler(State(state): State<ApiState>) -> Json<ConfigResponse> {
    let env_path = env_file_path();
    Json(ConfigResponse {
        media_path: state.config.media_path.to_string_lossy().to_string(),
        port: state.config.port,
        base_url: state.config.base_url.clone().unwrap_or_default(),
        public_url: state.config.public_url.clone().unwrap_or_default(),
        tmdb_api_key_masked: mask_secret(&state.config.tmdb_api_key),
        has_tmdb_api_key: !state.config.tmdb_api_key.trim().is_empty(),
        password_set: state.config.auth_token.is_some(),
        poster_url: state.config.poster_url.clone().unwrap_or_default(),
        scan_cron: state.config.scan_cron.clone().unwrap_or_default(),
        unmatched_catalog: state.config.unmatched_catalog,
        tmdb_base_url: state.config.tmdb_base_url.clone(),
        tmdb_image_base_url: state.config.tmdb_image_base_url.clone(),
        env_file_path: env_path.to_string_lossy().to_string(),
        env_file_exists: env_path.exists(),
    })
}

#[derive(Deserialize)]
pub struct UpdateConfigRequest {
    pub media_path: Option<String>,
    pub port: Option<u16>,
    pub base_url: Option<String>,
    pub public_url: Option<String>,
    pub tmdb_api_key: Option<String>,
    pub password: Option<String>,
    #[serde(default)]
    pub clear_password: bool,
    pub poster_url: Option<String>,
    pub scan_cron: Option<String>,
    pub unmatched_catalog: Option<bool>,
}

fn upsert_env_content(existing: &str, updates: &BTreeMap<String, Option<String>>) -> String {
    let mut seen = std::collections::HashSet::new();
    let mut lines: Vec<String> = Vec::new();

    for raw_line in existing.lines() {
        let trimmed = raw_line.trim();
        let active_key = if !trimmed.is_empty() && !trimmed.starts_with('#') {
            trimmed.split_once('=').map(|(k, _)| k.trim().to_string())
        } else {
            None
        };

        if let Some(key) = active_key {
            if let Some(val_opt) = updates.get(&key) {
                seen.insert(key.clone());
                match val_opt {
                    Some(v) => lines.push(format!("{}={}", key, v)),
                    None => lines.push(format!("# {}=", key)),
                }
                continue;
            }
        }
        lines.push(raw_line.to_string());
    }

    for (key, val_opt) in updates {
        if !seen.contains(key) {
            if let Some(v) = val_opt {
                lines.push(format!("{}={}", key, v));
            }
        }
    }

    let mut out = lines.join("\n");
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

pub async fn update_config_handler(
    State(state): State<ApiState>,
    Json(payload): Json<UpdateConfigRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    if let Some(port) = payload.port {
        if port == 0 {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "PORT must be between 1 and 65535" })),
            ));
        }
    }

    if let Some(ref cron_str) = payload.scan_cron {
        let trimmed = cron_str.trim();
        if !trimmed.is_empty() {
            let test_cfg = Config {
                media_path: state.config.media_path.clone(),
                port: state.config.port,
                base_url: None,
                public_url: None,
                tmdb_api_key: String::new(),
                tmdb_base_url: state.config.tmdb_base_url.clone(),
                tmdb_image_base_url: state.config.tmdb_image_base_url.clone(),
                password: None,
                auth_token: None,
                poster_url: None,
                scan_cron: Some(trimmed.to_string()),
                unmatched_catalog: false,
            };
            if let Err(e) = test_cfg.parsed_scan_cron() {
                return Err((
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({ "error": e.to_string() })),
                ));
            }
        }
    }

    let mut updates: BTreeMap<String, Option<String>> = BTreeMap::new();
    let mut media_path_exists = state.config.media_path.exists();

    if let Some(mp) = payload.media_path {
        let trimmed = mp.trim().to_string();
        if trimmed.is_empty() {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "MEDIA_PATH cannot be empty" })),
            ));
        }
        media_path_exists = FsPath::new(&trimmed).exists();
        updates.insert("MEDIA_PATH".to_string(), Some(trimmed));
    }
    if let Some(port) = payload.port {
        updates.insert("PORT".to_string(), Some(port.to_string()));
    }
    if let Some(base_url) = payload.base_url {
        let t = base_url.trim().to_string();
        updates.insert("BASE_URL".to_string(), if t.is_empty() { None } else { Some(t) });
    }
    if let Some(public_url) = payload.public_url {
        let t = public_url.trim().to_string();
        updates.insert("PUBLIC_URL".to_string(), if t.is_empty() { None } else { Some(t) });
    }
    if let Some(api_key) = payload.tmdb_api_key {
        let t = api_key.trim().to_string();
        if !t.is_empty() {
            updates.insert("TMDB_API_KEY".to_string(), Some(t));
        }
    }
    if payload.clear_password {
        updates.insert("PASSWORD".to_string(), None);
    } else if let Some(pw) = payload.password {
        let t = pw.trim().to_string();
        if !t.is_empty() {
            updates.insert("PASSWORD".to_string(), Some(t));
        }
    }
    if let Some(poster_url) = payload.poster_url {
        let t = poster_url.trim().to_string();
        updates.insert("POSTER_URL".to_string(), if t.is_empty() { None } else { Some(t) });
    }
    if let Some(scan_cron) = payload.scan_cron {
        let t = scan_cron.trim().to_string();
        updates.insert("SCAN_CRON".to_string(), if t.is_empty() { None } else { Some(t) });
    }
    if let Some(unmatched) = payload.unmatched_catalog {
        updates.insert("UNMATCHED_CATALOG".to_string(), Some(unmatched.to_string()));
    }

    let env_path = env_file_path();
    let existing = tokio::fs::read_to_string(&env_path).await.unwrap_or_default();
    let updated = upsert_env_content(&existing, &updates);
    tokio::fs::write(&env_path, updated).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Failed to write .env: {}", e) })),
        )
    })?;

    tracing::info!("Updated configuration in {:?}", env_path);

    let new_token = updates
        .get("PASSWORD")
        .and_then(|v| v.as_ref())
        .map(|p| compute_token(p));

    Ok(Json(serde_json::json!({
        "status": "ok",
        "env_path": env_path.to_string_lossy(),
        "media_path_exists": media_path_exists,
        "restart_required": true,
        "new_token_preview": new_token.map(|t| format!("{}...", &t[..12])),
        "message": "Saved settings to .env. Restart the server to apply changes.",
    })))
}

fn read_env_map(path: &FsPath) -> Vec<(String, String)> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    content
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                return None;
            }
            let (k, v) = trimmed.split_once('=')?;
            Some((k.trim().to_string(), v.trim().to_string()))
        })
        .collect()
}

pub async fn server_shutdown_handler() -> Json<serde_json::Value> {
    tracing::warn!("Server shutdown requested from GUI");
    tokio::spawn(async {
        tokio::time::sleep(std::time::Duration::from_millis(350)).await;
        std::process::exit(0);
    });
    Json(serde_json::json!({
        "status": "ok",
        "message": "Server is shutting down..."
    }))
}

pub async fn server_restart_handler() -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let exe = std::env::current_exe().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Cannot locate executable: {}", e) })),
        )
    })?;

    let env_path = env_file_path();
    let env_pairs = read_env_map(&env_path);

    tracing::warn!("Server restart requested from GUI ({:?})", exe);

    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(350)).await;

        #[cfg(windows)]
        {
            let mut cmd = std::process::Command::new("cmd");
            let exe_str = exe.to_string_lossy().to_string();
            cmd.args(["/C", &format!("timeout /T 1 /NOBREAK >nul && \"{}\"", exe_str)]);
            for (k, v) in env_pairs {
                cmd.env(k, v);
            }
            let _ = cmd.spawn();
        }

        #[cfg(not(windows))]
        {
            let mut cmd = std::process::Command::new("sh");
            let exe_str = exe.to_string_lossy().to_string();
            cmd.args(["-c", &format!("sleep 1 && exec \"{}\"", exe_str)]);
            for (k, v) in env_pairs {
                cmd.env(k, v);
            }
            let _ = cmd.spawn();
        }

        std::process::exit(0);
    });

    Ok(Json(serde_json::json!({
        "status": "ok",
        "message": "Server restarting with .env settings..."
    })))
}

#[derive(Deserialize)]
pub struct LogsQuery {
    pub limit: Option<usize>,
    pub level: Option<String>,
}

pub async fn logs_handler(Query(query): Query<LogsQuery>) -> Json<serde_json::Value> {
    let limit = query.limit.unwrap_or(150).min(400);
    let entries = global_log_store().recent(limit, query.level.as_deref());
    Json(serde_json::json!({
        "entries": entries,
    }))
}

pub async fn clear_logs_handler() -> Json<serde_json::Value> {
    global_log_store().clear();
    Json(serde_json::json!({ "status": "ok" }))
}
