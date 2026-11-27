use crate::config::Config;
use askama::Template;
use axum::{
    extract::State,
    http::{header, StatusCode},
    response::{Html, IntoResponse, Response},
};
use std::sync::Arc;

#[derive(Template)]
#[template(path = "console.html")]
struct ConsolePage {
    auth_required: bool,
    version: &'static str,
    port: u16,
    server_base: String,
    manifest_url: String,
    install_url: String,
}

pub async fn home_handler(State(config): State<Arc<Config>>) -> Html<String> {
    let auth_required = config.auth_token.is_some();
    let server_base = config
        .public_url
        .as_ref()
        .or(config.base_url.as_ref())
        .map(|s| s.trim_end_matches('/').to_string())
        .unwrap_or_default();

    // Never leak protected URLs in HTML when authentication is enabled
    let (manifest_url, install_url) = if auth_required {
        (String::new(), String::new())
    } else {
        let base = if server_base.is_empty() {
            format!("http://localhost:{}", config.port)
        } else {
            server_base.clone()
        };
        let manifest = format!("{}/manifest.json", base);
        let install = format!(
            "https://web.stremio.com/#/addons?addon={}",
            urlencoding::encode(&manifest)
        );
        (manifest, install)
    };

    Html(
        ConsolePage {
            auth_required,
            version: env!("CARGO_PKG_VERSION"),
            port: config.port,
            server_base,
            manifest_url,
            install_url,
        }
        .render()
        .unwrap(),
    )
}

pub async fn asset_css_handler() -> Response {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/css; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        include_str!("../templates/console.css"),
    )
        .into_response()
}

pub async fn asset_js_handler() -> Response {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/javascript; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        include_str!("../templates/console.js"),
    )
        .into_response()
}
