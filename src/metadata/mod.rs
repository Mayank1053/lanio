pub mod rate_limiter;
pub mod types;

use moka::future::Cache;
use rate_limiter::RateLimiter;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use types::{
    MediaMetadata, TmdbCandidate, TmdbFindResponse, TmdbMovieDetails, TmdbSearchResponse,
    TmdbSearchResult, TmdbTvDetails, TmdbTvExternalIds,
};

const DISK_CACHE_TTL_SECS: i64 = 30 * 86400; // 30 days on disk

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DiskCacheEntry {
    cached_at: i64,
    metadata: MediaMetadata,
}

pub struct TmdbClient {
    client: reqwest::Client,
    api_key: String,
    cache: Cache<String, MediaMetadata>,
    cache_path: Option<PathBuf>,
    disk_entries: Arc<Mutex<HashMap<String, DiskCacheEntry>>>,
    write_lock: Arc<tokio::sync::Mutex<()>>,
    rate_limiter: RateLimiter,
    base_url: String,
    image_base_url: String,
}

impl TmdbClient {
    #[allow(dead_code)]
    pub fn new(api_key: String, base_url: String, image_base_url: String) -> Self {
        Self::with_cache_file(api_key, base_url, image_base_url, None)
    }

    pub fn with_cache_file(
        api_key: String,
        base_url: String,
        image_base_url: String,
        cache_path: Option<PathBuf>,
    ) -> Self {
        let cache = Cache::builder()
            .max_capacity(5000)
            .time_to_live(Duration::from_secs(86400)) // 24 hours in RAM
            .build();

        let disk_entries = Arc::new(Mutex::new(HashMap::new()));

        let client = Self {
            client: reqwest::Client::new(),
            api_key,
            cache,
            cache_path,
            disk_entries,
            write_lock: Arc::new(tokio::sync::Mutex::new(())),
            rate_limiter: RateLimiter::new(40), // 40 requests per 10 seconds
            base_url,
            image_base_url,
        };

        client.load_disk_cache_sync();
        client
    }

    fn load_disk_cache_sync(&self) {
        let Some(ref path) = self.cache_path else {
            return;
        };
        if !path.exists() {
            return;
        }
        let Ok(content) = std::fs::read_to_string(path) else {
            tracing::warn!("Could not read TMDB cache file {:?}", path);
            return;
        };
        let Ok(parsed) = serde_json::from_str::<HashMap<String, DiskCacheEntry>>(&content) else {
            tracing::warn!("TMDB cache file {:?} was malformed; starting fresh", path);
            return;
        };

        let now = chrono::Utc::now().timestamp();
        let mut loaded = 0usize;
        let mut valid: HashMap<String, DiskCacheEntry> = HashMap::new();
        for (key, entry) in parsed {
            if now - entry.cached_at <= DISK_CACHE_TTL_SECS {
                valid.insert(key, entry);
                loaded += 1;
            }
        }

        if let Ok(mut guard) = self.disk_entries.lock() {
            *guard = valid;
        }
        tracing::info!("Loaded {} cached TMDB entries from {:?}", loaded, path);
    }

    async fn get_cached(&self, key: &str) -> Option<MediaMetadata> {
        if let Some(cached) = self.cache.get(key).await {
            return Some(cached);
        }
        let disk_hit = {
            let guard = self.disk_entries.lock().ok()?;
            let entry = guard.get(key)?;
            if chrono::Utc::now().timestamp() - entry.cached_at <= DISK_CACHE_TTL_SECS {
                Some(entry.metadata.clone())
            } else {
                None
            }
        };
        if let Some(ref meta) = disk_hit {
            self.cache.insert(key.to_string(), meta.clone()).await;
        }
        disk_hit
    }

    async fn put_cached(&self, key: String, metadata: MediaMetadata) {
        self.cache.insert(key.clone(), metadata.clone()).await;

        let Some(ref path) = self.cache_path else {
            return;
        };
        let snapshot = {
            let Ok(mut guard) = self.disk_entries.lock() else {
                return;
            };
            guard.insert(
                key,
                DiskCacheEntry {
                    cached_at: chrono::Utc::now().timestamp(),
                    metadata,
                },
            );
            guard.clone()
        };

        let _lock = self.write_lock.lock().await;
        if let Ok(json) = serde_json::to_string_pretty(&snapshot) {
            if let Some(parent) = path.parent() {
                if !parent.as_os_str().is_empty() {
                    let _ = tokio::fs::create_dir_all(parent).await;
                }
            }
            if let Err(e) = write_atomic(path, json.as_bytes()).await {
                tracing::warn!("Failed to persist TMDB cache to {:?}: {}", path, e);
            }
        }
    }

    pub async fn search_movie(&self, title: &str, year: Option<u16>) -> Option<MediaMetadata> {
        let cache_key = format!("movie:{}:{}", title, year.unwrap_or(0));

        // Check cache first
        if let Some(cached) = self.get_cached(&cache_key).await {
            tracing::debug!("Cache hit for movie: {}", title);
            return Some(cached);
        }

        // Rate limit
        let _permit = self.rate_limiter.acquire().await;

        // Search for movie
        let mut url = format!("{}/search/movie", self.base_url);
        let mut params = vec![
            ("api_key", self.api_key.clone()),
            ("query", title.to_string()),
        ];
        if let Some(y) = year {
            params.push(("year", y.to_string()));
        }

        let response = self.client.get(&url).query(&params).send().await.ok()?;

        if !response.status().is_success() {
            tracing::error!("TMDB API error: {}", response.status());
            return None;
        }

        let search_response: TmdbSearchResponse = response.json().await.ok()?;

        if search_response.results.is_empty() {
            tracing::debug!("No TMDB results for movie: {}", title);
            return None;
        }

        let best = best_result_index(&search_response.results, year)?;
        let movie_id = search_response.results[best].id;

        // Get movie details to retrieve IMDb ID
        let _permit = self.rate_limiter.acquire().await;

        url = format!("{}/movie/{}", self.base_url, movie_id);
        let response = self
            .client
            .get(&url)
            .query(&[("api_key", &self.api_key)])
            .send()
            .await
            .ok()?;

        let details: TmdbMovieDetails = response.json().await.ok()?;

        let imdb_id = details.imdb_id.clone()?;
        let metadata = self.movie_metadata(
            imdb_id,
            &details,
            search_response.results[best].title.clone(),
        );

        tracing::info!("Found IMDb ID for {}: {}", title, metadata.imdb_id);

        // Cache the result
        self.put_cached(cache_key, metadata.clone()).await;
        Some(metadata)
    }

    pub async fn search_tv_show(&self, title: &str, year: Option<u16>) -> Option<MediaMetadata> {
        let cache_key = format!("tv:{}:{}", title, year.unwrap_or(0));

        // Check cache first
        if let Some(cached) = self.get_cached(&cache_key).await {
            tracing::debug!("Cache hit for TV show: {}", title);
            return Some(cached);
        }

        // Rate limit
        let _permit = self.rate_limiter.acquire().await;

        // Search for TV show
        let mut url = format!("{}/search/tv", self.base_url);
        let mut params = vec![
            ("api_key", self.api_key.clone()),
            ("query", title.to_string()),
        ];
        if let Some(y) = year {
            params.push(("first_air_date_year", y.to_string()));
        }

        let response = self.client.get(&url).query(&params).send().await.ok()?;

        if !response.status().is_success() {
            tracing::error!("TMDB API error: {}", response.status());
            return None;
        }

        let search_response: TmdbSearchResponse = response.json().await.ok()?;

        if search_response.results.is_empty() {
            tracing::debug!("No TMDB results for TV show: {}", title);
            return None;
        }

        let best = best_result_index(&search_response.results, year)?;
        let tv_id = search_response.results[best].id;

        // Get external IDs to retrieve IMDb ID
        let _permit = self.rate_limiter.acquire().await;

        url = format!("{}/tv/{}/external_ids", self.base_url, tv_id);
        let response = self
            .client
            .get(&url)
            .query(&[("api_key", &self.api_key)])
            .send()
            .await
            .ok()?;

        let external_ids: TmdbTvExternalIds = response.json().await.ok()?;

        let imdb_id = external_ids.imdb_id?;

        // Get TV show details to retrieve poster
        let _permit = self.rate_limiter.acquire().await;

        let details_url = format!("{}/tv/{}", self.base_url, tv_id);
        let details_response = self
            .client
            .get(&details_url)
            .query(&[("api_key", &self.api_key)])
            .send()
            .await
            .ok()?;

        let details: TmdbTvDetails = details_response.json().await.ok()?;

        let metadata = self.tv_metadata(
            imdb_id,
            &details,
            search_response.results[best].title.clone(),
        );

        tracing::info!("Found IMDb ID for {}: {}", title, metadata.imdb_id);

        // Cache the result
        self.put_cached(cache_key, metadata.clone()).await;
        Some(metadata)
    }

    pub async fn get_metadata_by_imdb_id(&self, imdb_id: &str) -> Option<MediaMetadata> {
        let cache_key = format!("imdb:{}", imdb_id);

        // Check cache first
        if let Some(cached) = self.get_cached(&cache_key).await {
            tracing::debug!("Cache hit for IMDb ID: {}", imdb_id);
            return Some(cached);
        }

        // Rate limit
        let _permit = self.rate_limiter.acquire().await;

        // Use TMDB find endpoint with IMDb ID
        let url = format!("{}/find/{}", self.base_url, imdb_id);
        let response = self
            .client
            .get(&url)
            .query(&[
                ("api_key", self.api_key.as_str()),
                ("external_source", "imdb_id"),
            ])
            .send()
            .await
            .ok()?;

        if !response.status().is_success() {
            tracing::error!("TMDB API error: {}", response.status());
            return None;
        }

        let find_response: TmdbFindResponse = response.json().await.ok()?;

        // Check movie results first, then TV results
        let metadata = if !find_response.movie_results.is_empty() {
            let result = &find_response.movie_results[0];
            MediaMetadata {
                imdb_id: imdb_id.to_string(),
                title: result.title.clone(),
                year: parse_year(&result.release_date),
                overview: result.overview.clone(),
                poster_url: result
                    .poster_path
                    .clone()
                    .map(|path| format!("{}{}", self.image_base_url, path)),
                tmdb_rating: result.vote_average,
                tmdb_votes: result.vote_count,
            }
        } else if !find_response.tv_results.is_empty() {
            let result = &find_response.tv_results[0];
            MediaMetadata {
                imdb_id: imdb_id.to_string(),
                title: result.title.clone(),
                year: parse_year(&result.first_air_date),
                overview: result.overview.clone(),
                poster_url: result
                    .poster_path
                    .clone()
                    .map(|path| format!("{}{}", self.image_base_url, path)),
                tmdb_rating: result.vote_average,
                tmdb_votes: result.vote_count,
            }
        } else {
            tracing::debug!("No TMDB results for IMDb ID: {}", imdb_id);
            return None;
        };

        tracing::info!("Found metadata for IMDb ID: {}", imdb_id);

        // Cache the result
        self.put_cached(cache_key, metadata.clone()).await;
        Some(metadata)
    }

    pub async fn cache_count(&self) -> u64 {
        self.cache.run_pending_tasks().await;
        let disk_len = self.disk_entries.lock().map(|g| g.len() as u64).unwrap_or(0);
        self.cache.entry_count().max(disk_len)
    }

    pub fn clear_cache(&self) {
        self.cache.invalidate_all();
        if let Ok(mut guard) = self.disk_entries.lock() {
            guard.clear();
        }
        if let Some(ref path) = self.cache_path {
            let _ = std::fs::remove_file(path);
        }
    }

    pub async fn search_candidates(
        &self,
        query: &str,
        media_type: &str,
        year: Option<u16>,
    ) -> anyhow::Result<Vec<TmdbCandidate>> {
        let _permit = self.rate_limiter.acquire().await;

        let is_tv = media_type.eq_ignore_ascii_case("series")
            || media_type.eq_ignore_ascii_case("tv");
        let endpoint = if is_tv { "search/tv" } else { "search/movie" };
        let url = format!("{}/{}", self.base_url, endpoint);

        let mut params = vec![
            ("api_key", self.api_key.clone()),
            ("query", query.to_string()),
        ];
        if let Some(y) = year {
            params.push((
                if is_tv { "first_air_date_year" } else { "year" },
                y.to_string(),
            ));
        }

        let response = self.client.get(&url).query(&params).send().await?;
        if !response.status().is_success() {
            anyhow::bail!("TMDB API returned HTTP {}", response.status());
        }

        let search_response: TmdbSearchResponse = response.json().await?;
        let kind = if is_tv { "series" } else { "movie" };
        Ok(search_response
            .results
            .into_iter()
            .take(12)
            .map(|r| TmdbCandidate {
                tmdb_id: r.id,
                media_type: kind.to_string(),
                title: r.title.unwrap_or_else(|| format!("TMDB #{}", r.id)),
                year: parse_year(&r.release_date),
                overview: r.overview,
                poster_url: r
                    .poster_path
                    .map(|path| format!("{}{}", self.image_base_url, path)),
                vote_average: r.vote_average,
                vote_count: r.vote_count,
            })
            .collect())
    }

    pub async fn resolve_candidate(
        &self,
        tmdb_id: u64,
        media_type: &str,
    ) -> anyhow::Result<Option<MediaMetadata>> {
        let is_tv = media_type.eq_ignore_ascii_case("series")
            || media_type.eq_ignore_ascii_case("tv");

        if is_tv {
            let _permit = self.rate_limiter.acquire().await;
            let ext_url = format!("{}/tv/{}/external_ids", self.base_url, tmdb_id);
            let ext_resp = self
                .client
                .get(&ext_url)
                .query(&[("api_key", &self.api_key)])
                .send()
                .await?;
            if !ext_resp.status().is_success() {
                anyhow::bail!("TMDB API returned HTTP {}", ext_resp.status());
            }
            let external_ids: TmdbTvExternalIds = ext_resp.json().await?;
            let Some(imdb_id) = external_ids.imdb_id else {
                return Ok(None);
            };

            let _permit = self.rate_limiter.acquire().await;
            let details_url = format!("{}/tv/{}", self.base_url, tmdb_id);
            let details_resp = self
                .client
                .get(&details_url)
                .query(&[("api_key", &self.api_key)])
                .send()
                .await?;
            if !details_resp.status().is_success() {
                anyhow::bail!("TMDB API returned HTTP {}", details_resp.status());
            }
            let details: TmdbTvDetails = details_resp.json().await?;
            let meta = self.tv_metadata(imdb_id.clone(), &details, None);
            self.put_cached(format!("imdb:{}", imdb_id), meta.clone()).await;
            Ok(Some(meta))
        } else {
            let _permit = self.rate_limiter.acquire().await;
            let url = format!("{}/movie/{}", self.base_url, tmdb_id);
            let resp = self
                .client
                .get(&url)
                .query(&[("api_key", &self.api_key)])
                .send()
                .await?;
            if !resp.status().is_success() {
                anyhow::bail!("TMDB API returned HTTP {}", resp.status());
            }
            let details: TmdbMovieDetails = resp.json().await?;
            let Some(imdb_id) = details.imdb_id.clone() else {
                return Ok(None);
            };
            let meta = self.movie_metadata(imdb_id.clone(), &details, None);
            self.put_cached(format!("imdb:{}", imdb_id), meta.clone()).await;
            Ok(Some(meta))
        }
    }

    fn movie_metadata(
        &self,
        imdb_id: String,
        details: &TmdbMovieDetails,
        fallback_title: Option<String>,
    ) -> MediaMetadata {
        MediaMetadata {
            imdb_id,
            title: details.title.clone().or(fallback_title),
            year: parse_year(&details.release_date),
            overview: details.overview.clone(),
            poster_url: details
                .poster_path
                .clone()
                .map(|path| format!("{}{}", self.image_base_url, path)),
            tmdb_rating: details.vote_average,
            tmdb_votes: details.vote_count,
        }
    }

    fn tv_metadata(
        &self,
        imdb_id: String,
        details: &TmdbTvDetails,
        fallback_title: Option<String>,
    ) -> MediaMetadata {
        MediaMetadata {
            imdb_id,
            title: details.name.clone().or(fallback_title),
            year: parse_year(&details.first_air_date),
            overview: details.overview.clone(),
            poster_url: details
                .poster_path
                .clone()
                .map(|path| format!("{}{}", self.image_base_url, path)),
            tmdb_rating: details.vote_average,
            tmdb_votes: details.vote_count,
        }
    }
}

async fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp_path = path.with_extension("json.tmp");
    tokio::fs::write(&tmp_path, bytes).await?;
    tokio::fs::rename(&tmp_path, path).await
}

fn parse_year(date: &Option<String>) -> Option<u16> {
    date.as_deref()?.get(..4)?.parse().ok()
}

/// Pick the search hit that best matches the year on disk.
///
/// TMDB search results are ranked by relevance/popularity, not by year. The
/// `year` / `first_air_date_year` parameter is a soft filter, so a remake, a
/// newer entry or a similarly-named title can still come back first — trusting
/// `results[0]` is how "Avatar: The Last Airbender" from 2005 gets mapped onto a
/// different 2024 series. Prefer an exact year match, otherwise the nearest one,
/// and fall back to relevance order when no candidate carries a date.
fn best_result_index(results: &[TmdbSearchResult], year: Option<u16>) -> Option<usize> {
    if results.is_empty() {
        return None;
    }

    let Some(wanted) = year else {
        return Some(0);
    };

    let mut nearest: Option<(u16, usize)> = None;
    for (index, result) in results.iter().enumerate() {
        let Some(candidate) = parse_year(&result.release_date) else {
            continue;
        };
        let distance = candidate.abs_diff(wanted);
        match nearest {
            // Strictly-less keeps the earlier (more relevant) result on ties.
            Some((best, _)) if best <= distance => {}
            _ => nearest = Some((distance, index)),
        }
    }

    Some(nearest.map_or(0, |(_, index)| index))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(id: u64, release_date: Option<&str>) -> TmdbSearchResult {
        TmdbSearchResult {
            id,
            title: Some(format!("title {}", id)),
            overview: None,
            poster_path: None,
            release_date: release_date.map(str::to_string),
            vote_average: None,
            vote_count: None,
        }
    }

    #[test]
    fn best_result_is_none_when_there_are_no_results() {
        assert_eq!(best_result_index(&[], Some(2005)), None);
    }

    #[test]
    fn best_result_falls_back_to_relevance_order_without_a_year() {
        let results = vec![result(1, Some("2024-02-07")), result(2, Some("2005-02-08"))];
        assert_eq!(best_result_index(&results, None), Some(0));
    }

    #[test]
    fn best_result_prefers_exact_year_over_higher_ranked_mismatch() {
        let results = vec![result(1, Some("2024-02-07")), result(2, Some("2005-02-08"))];
        assert_eq!(best_result_index(&results, Some(2005)), Some(1));
    }

    #[test]
    fn best_result_picks_nearest_year_when_nothing_matches_exactly() {
        let results = vec![result(1, Some("2024-02-07")), result(2, Some("2007-02-08"))];
        assert_eq!(best_result_index(&results, Some(2005)), Some(1));
    }

    #[test]
    fn best_result_keeps_relevance_order_on_equal_distance() {
        let results = vec![result(1, Some("2003-01-01")), result(2, Some("2007-01-01"))];
        assert_eq!(best_result_index(&results, Some(2005)), Some(0));
    }

    #[test]
    fn best_result_falls_back_when_no_candidate_has_a_date() {
        let results = vec![result(1, None), result(2, None)];
        assert_eq!(best_result_index(&results, Some(2005)), Some(0));
    }

    #[test]
    fn best_result_skips_undated_candidates() {
        let results = vec![result(1, None), result(2, Some("2005-02-08"))];
        assert_eq!(best_result_index(&results, Some(2005)), Some(1));
    }

    #[tokio::test]
    async fn search_movie_prefers_the_result_matching_the_file_year() {
        let server = httpmock::MockServer::start();
        server.mock(|when, then| {
            when.method(httpmock::Method::GET)
                .path("/search/movie")
                .query_param("query", "Avatar The Last Airbender")
                .query_param("year", "2005");
            then.status(200).json_body(serde_json::json!({
                "results": [
                    { "id": 100, "title": "Avatar: The Last Airbender", "release_date": "2024-02-07" },
                    { "id": 200, "title": "Avatar: The Last Airbender", "release_date": "2005-02-08" }
                ]
            }));
        });
        server.mock(|when, then| {
            when.method(httpmock::Method::GET).path("/movie/200");
            then.status(200).json_body(serde_json::json!({
                "id": 200,
                "imdb_id": "tt0417299",
                "title": "Avatar: The Last Airbender",
                "release_date": "2005-02-08"
            }));
        });

        let client = TmdbClient::new("fake".into(), server.base_url(), "http://img".into());
        let metadata = client
            .search_movie("Avatar The Last Airbender", Some(2005))
            .await;

        assert_eq!(metadata.unwrap().imdb_id, "tt0417299");
    }

    #[tokio::test]
    async fn search_tv_show_prefers_the_result_matching_the_file_year() {
        let server = httpmock::MockServer::start();
        server.mock(|when, then| {
            when.method(httpmock::Method::GET)
                .path("/search/tv")
                .query_param("query", "Avatar The Last Airbender")
                .query_param("first_air_date_year", "2005");
            then.status(200).json_body(serde_json::json!({
                "results": [
                    { "id": 100, "name": "Avatar: The Last Airbender", "first_air_date": "2024-02-07" },
                    { "id": 200, "name": "Avatar: The Last Airbender", "first_air_date": "2005-02-08" }
                ]
            }));
        });
        server.mock(|when, then| {
            when.method(httpmock::Method::GET)
                .path("/tv/200/external_ids");
            then.status(200)
                .json_body(serde_json::json!({ "imdb_id": "tt0417299" }));
        });
        server.mock(|when, then| {
            when.method(httpmock::Method::GET).path("/tv/200");
            then.status(200).json_body(serde_json::json!({
                "name": "Avatar: The Last Airbender",
                "first_air_date": "2005-02-08"
            }));
        });

        let client = TmdbClient::new("fake".into(), server.base_url(), "http://img".into());
        let metadata = client
            .search_tv_show("Avatar The Last Airbender", Some(2005))
            .await;

        assert_eq!(metadata.unwrap().imdb_id, "tt0417299");
    }

    #[tokio::test]
    async fn disk_cache_persists_across_client_restarts_and_clears() {
        let server = httpmock::MockServer::start();
        let search_mock = server.mock(|when, then| {
            when.method(httpmock::Method::GET)
                .path("/search/movie")
                .query_param("query", "Dune")
                .query_param("year", "2021");
            then.status(200).json_body(serde_json::json!({
                "results": [{ "id": 438631, "title": "Dune", "release_date": "2021-09-15" }]
            }));
        });
        let details_mock = server.mock(|when, then| {
            when.method(httpmock::Method::GET).path("/movie/438631");
            then.status(200).json_body(serde_json::json!({
                "id": 438631,
                "imdb_id": "tt1160419",
                "title": "Dune",
                "release_date": "2021-09-15"
            }));
        });

        let dir = tempfile::tempdir().unwrap();
        let cache_file = dir.path().join("tmdb-cache.json");

        let client1 = TmdbClient::with_cache_file(
            "fake".into(),
            server.base_url(),
            "http://img".into(),
            Some(cache_file.clone()),
        );
        let first = client1.search_movie("Dune", Some(2021)).await.unwrap();
        assert_eq!(first.imdb_id, "tt1160419");
        assert!(cache_file.exists());
        search_mock.assert_hits(1);
        details_mock.assert_hits(1);

        // Fresh client pointing at an unreachable TMDB server must still serve from disk cache
        let client2 = TmdbClient::with_cache_file(
            "fake".into(),
            "http://127.0.0.1:9".into(),
            "http://img".into(),
            Some(cache_file.clone()),
        );
        let second = client2.search_movie("Dune", Some(2021)).await.unwrap();
        assert_eq!(second.imdb_id, "tt1160419");
        assert_eq!(client2.cache_count().await, 1);

        client2.clear_cache();
        assert!(!cache_file.exists());
        assert_eq!(client2.cache_count().await, 0);
    }
}
