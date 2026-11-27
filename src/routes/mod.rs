pub mod api;
pub mod catalog;
pub mod health;
pub mod home;
pub mod manifest;
pub mod meta;
pub mod poster;
pub mod rescan;
pub mod stream;

use crate::config::Config;
use crate::scanner::MediaScanner;
use crate::streamer::{video_handler, StreamerState};
use api::{
    api_auth, clear_cache_handler, clear_logs_handler, files_handler, get_config_handler,
    logs_handler, media_handler, overview_handler, rematch_handler, rename_file_handler,
    rescan_file_handler, server_restart_handler, server_shutdown_handler, tmdb_resolve_handler,
    tmdb_search_handler, update_config_handler, upload_poster_handler, ApiState,
};
use axum::{
    extract::{Path, Request, State},
    http::StatusCode,
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use catalog::{catalog_handler, CatalogState};
use health::health_handler;
use home::{asset_css_handler, asset_js_handler, home_handler};
use manifest::get_manifest;
use meta::meta_handler;
use poster::poster_handler;
use rescan::rescan_handler;
use std::collections::HashMap;
use std::sync::Arc;
use stream::{stream_handler, StreamState};
use tower_http::cors::CorsLayer;

async fn token_auth(
    Path(params): Path<HashMap<String, String>>,
    State(config): State<Arc<Config>>,
    request: Request,
    next: Next,
) -> Response {
    let token = params.get("token").map(|s| s.as_str()).unwrap_or("");
    if config.is_valid_token(token) {
        next.run(request).await
    } else {
        StatusCode::NOT_FOUND.into_response()
    }
}

fn build_stremio_subrouter(scanner: &Arc<MediaScanner>, config: &Arc<Config>) -> Router {
    let stream_state = StreamState {
        scanner: Arc::clone(scanner),
        config: Arc::clone(config),
    };
    let catalog_state = CatalogState {
        scanner: Arc::clone(scanner),
    };
    let streamer_state = StreamerState {
        config: Arc::clone(config),
    };

    let subrouter = Router::new()
        .route("/manifest.json", get(get_manifest))
        .with_state(Arc::clone(config))
        .route("/stream/:type/:id", get(stream_handler))
        .with_state(stream_state.clone())
        .route("/meta/:type/:id", get(meta_handler))
        .with_state(stream_state.clone())
        .route("/poster/:id", get(poster_handler))
        .with_state(stream_state)
        .route("/catalog/:type/:id", get(catalog_handler))
        .with_state(catalog_state)
        .route("/video", get(video_handler))
        .with_state(streamer_state)
        .route("/rescan", post(rescan_handler))
        .with_state(Arc::clone(scanner));

    if config.auth_token.is_some() {
        Router::new().nest(
            "/:token",
            subrouter.route_layer(middleware::from_fn_with_state(
                Arc::clone(config),
                token_auth,
            )),
        )
    } else {
        subrouter
    }
}

fn build_api_router(scanner: &Arc<MediaScanner>, config: &Arc<Config>) -> Router {
    let api_state = ApiState {
        scanner: Arc::clone(scanner),
        config: Arc::clone(config),
    };

    Router::new()
        .route("/overview", get(overview_handler))
        .route("/media", get(media_handler))
        .route("/media/rematch", post(rematch_handler))
        .route("/media/rescan-file", post(rescan_file_handler))
        .route("/files", get(files_handler))
        .route("/files/rename", post(rename_file_handler))
        .route("/files/poster", post(upload_poster_handler))
        .route("/tmdb/search", get(tmdb_search_handler))
        .route("/tmdb/resolve", get(tmdb_resolve_handler))
        .route("/rescan", post(rescan_handler).with_state(Arc::clone(scanner)))
        .route("/cache/clear", post(clear_cache_handler))
        .route("/config", get(get_config_handler).put(update_config_handler))
        .route("/server/restart", post(server_restart_handler))
        .route("/server/shutdown", post(server_shutdown_handler))
        .route("/logs", get(logs_handler).delete(clear_logs_handler))
        .with_state(api_state)
        .route_layer(middleware::from_fn_with_state(
            Arc::clone(config),
            api_auth,
        ))
}

pub fn create_router(scanner: Arc<MediaScanner>, config: Arc<Config>) -> Router {
    Router::new()
        .route("/", get(home_handler))
        .with_state(Arc::clone(&config))
        .route("/assets/console.css", get(asset_css_handler))
        .route("/assets/console.js", get(asset_js_handler))
        .nest("/api", build_api_router(&scanner, &config))
        .merge(build_stremio_subrouter(&scanner, &config))
        .merge(
            Router::new()
                .route("/health", get(health_handler))
                .with_state(scanner),
        )
        .layer(CorsLayer::permissive())
}
