mod common;

use base64::{engine::general_purpose, Engine as _};
use common::{mock_movie, start_server, token_for, wait_until};
use httpmock::prelude::*;
use serde_json::json;
use std::time::Duration;
use tempfile::tempdir;

#[tokio::test]
async fn gui_assets_and_management_endpoints_work() {
    let tmdb = MockServer::start();
    mock_movie(&tmdb, "Inception", 27205, "tt1375666", None);

    // Mock /find/tt1375666 for rematching
    tmdb.mock(|when, then| {
        when.method(GET)
            .path("/find/tt1375666")
            .query_param("external_source", "imdb_id");
        then.status(200).json_body(json!({
            "movie_results": [{
                "title": "Inception",
                "overview": "A thief who steals corporate secrets.",
                "release_date": "2010-07-16",
                "poster_path": "/inception.jpg",
                "vote_average": 8.4,
                "vote_count": 35000
            }],
            "tv_results": []
        }));
    });

    let temp_media = tempdir().unwrap();
    let movie_path = temp_media.path().join("Inception.2010.mkv");
    std::fs::write(&movie_path, "inception video").unwrap();
    let mystery_path = temp_media.path().join("mystery_clip.mp4");
    std::fs::write(&mystery_path, "mystery video").unwrap();

    let base_url = start_server(temp_media.path(), &tmdb.base_url(), None, None).await;
    let client = reqwest::Client::new();

    // Wait for initial scan to populate Inception
    let ready = wait_until(20, Duration::from_millis(250), || async {
        let resp = client
            .get(format!("{}/api/overview", base_url))
            .send()
            .await
            .unwrap();
        let ov: serde_json::Value = resp.json().await.unwrap();
        ov["counts"]["movies"] == 1
    })
    .await;
    assert!(ready, "overview should report 1 indexed movie");

    // Verify GUI HTML + CSS + JS serve cleanly
    let home = client.get(format!("{}/", base_url)).send().await.unwrap();
    assert_eq!(home.status(), 200);
    let home_html = home.text().await.unwrap();
    assert!(home_html.contains("Screening Room"));
    assert!(home_html.contains("/assets/console.css"));
    assert!(home_html.contains("/assets/console.js"));

    let css = client
        .get(format!("{}/assets/console.css", base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(css.status(), 200);
    let js = client
        .get(format!("{}/assets/console.js", base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(js.status(), 200);

    // Media API returns matched + skipped
    let media: serde_json::Value = client
        .get(format!("{}/api/media", base_url))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(media["movies"].as_array().unwrap().len(), 1);
    assert_eq!(media["skipped"].as_array().unwrap().len(), 1);

    // File explorer lists directory contents
    let files: serde_json::Value = client
        .get(format!("{}/api/files?path=", base_url))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(files["entries"].as_array().unwrap().len(), 2);

    // Path traversal blocked
    let escape = client
        .get(format!("{}/api/files?path=..", base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(escape.status(), 400);

    // TMDB search works
    let tmdb_res: serde_json::Value = client
        .get(format!(
            "{}/api/tmdb/search?q=Inception&type=movie",
            base_url
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(tmdb_res["results"][0]["tmdb_id"], 27205);

    // Sidecar poster upload works
    let png_b64 = general_purpose::STANDARD.encode(b"fake-jpg-data");
    let poster_resp = client
        .post(format!("{}/api/files/poster", base_url))
        .json(&json!({
            "video_path": mystery_path.to_string_lossy(),
            "extension": "jpg",
            "data_base64": png_b64,
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(poster_resp.status(), 200);
    assert!(temp_media.path().join("mystery_clip.jpg").is_file());

    // Rematch mystery_clip.mp4 to tt1375666 without renaming
    let rematch = client
        .post(format!("{}/api/media/rematch", base_url))
        .json(&json!({
            "path": mystery_path.to_string_lossy(),
            "imdb_id": "tt1375666",
            "content_type": "movie",
            "write_imdb_tag": false
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(rematch.status(), 200);

    // Clear cache works
    let clear = client
        .post(format!("{}/api/cache/clear", base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(clear.status(), 200);
}

#[tokio::test]
async fn management_api_requires_token_when_password_set() {
    let tmdb = MockServer::start();
    let temp_media = tempdir().unwrap();
    let base_url = start_server(temp_media.path(), &tmdb.base_url(), Some("hunter2"), None).await;
    let client = reqwest::Client::new();
    let token = token_for("hunter2");

    let unauth = client
        .get(format!("{}/api/overview", base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(unauth.status(), 401);

    let wrong = client
        .get(format!("{}/api/overview", base_url))
        .header("X-Lanio-Token", "bad-token")
        .send()
        .await
        .unwrap();
    assert_eq!(wrong.status(), 401);

    let ok = client
        .get(format!("{}/api/overview", base_url))
        .header("X-Lanio-Token", &token)
        .send()
        .await
        .unwrap();
    assert_eq!(ok.status(), 200);
    let body: serde_json::Value = ok.json().await.unwrap();
    assert_eq!(body["auth_enabled"], true);
}

#[tokio::test]
async fn watcher_reflects_added_changed_and_deleted_dotted_folders_without_duplicates() {
    let tmdb = MockServer::start();
    common::mock_series(&tmdb, "Breaking Bad", Some("2008"), 1396, "tt0903747");

    let temp_media = tempdir().unwrap();
    let base_url = start_server(temp_media.path(), &tmdb.base_url(), None, None).await;
    let client = reqwest::Client::new();

    // Add a dotted show folder + episode
    let show_dir = temp_media.path().join("Breaking.Bad.2008");
    std::fs::create_dir_all(&show_dir).unwrap();
    let ep_path = show_dir.join("Breaking.Bad.S01E01.mkv");
    std::fs::write(&ep_path, "ep1 v1").unwrap();

    let added = wait_until(25, Duration::from_millis(250), || async {
        let m: serde_json::Value = client
            .get(format!("{}/api/media", base_url))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        m["series"]
            .as_array()
            .is_some_and(|s| s.len() == 1 && s[0]["episode_count"] == 1)
    })
    .await;
    assert!(added, "added series episode should appear once");

    // Modify file in place — must stay 1 episode, not duplicate
    std::fs::write(&ep_path, "ep1 v2 updated content").unwrap();
    tokio::time::sleep(Duration::from_millis(400)).await;
    let media_after_mod: serde_json::Value = client
        .get(format!("{}/api/media", base_url))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        media_after_mod["series"][0]["episode_count"], 1,
        "modifying an episode file must not create duplicate entries"
    );

    // Delete the dotted folder — series must disappear cleanly
    std::fs::remove_dir_all(&show_dir).unwrap();
    let deleted = wait_until(25, Duration::from_millis(250), || async {
        let m: serde_json::Value = client
            .get(format!("{}/api/media", base_url))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        m["series"].as_array().is_some_and(|s| s.is_empty())
    })
    .await;
    assert!(deleted, "deleting a dotted show folder must purge its episodes");
}
