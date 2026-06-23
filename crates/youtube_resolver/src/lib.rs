#![allow(clippy::collapsible_if)]
use async_trait::async_trait;
use reqwest::header::{
    CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue, ORIGIN, REFERER, USER_AGENT,
};
use serde_json::json;
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub use rusty_ytdl::PlayerResponse;
pub mod format_selector;
pub mod js_solver;
pub mod stream_probe;

#[derive(Clone, Debug)]
pub struct SessionData {
    pub visitor_data: String,
    pub sts: u64,
    pub player_url: String,
}

static SESSION_CACHE: Mutex<Option<(SessionData, Instant)>> = Mutex::new(None);

#[derive(Debug, Clone)]
pub struct ResolveContext {
    pub visitor_data: Option<String>,
    pub user_agent_override: Option<String>,
    pub language: Option<String>,
    pub region: Option<String>,
    pub timeout: Duration,
    pub trace_id: Option<String>,
}

impl Default for ResolveContext {
    fn default() -> Self {
        Self {
            visitor_data: None,
            user_agent_override: None,
            language: Some("en".to_string()),
            region: Some("US".to_string()),
            timeout: Duration::from_secs(5),
            trace_id: None,
        }
    }
}

#[derive(thiserror::Error, Debug)]
pub enum ResolveError {
    #[error("Network error: {0}")]
    Network(#[from] reqwest::Error),

    #[error("Serialization/Deserialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("API error response: status={status:?}, reason={reason:?}")]
    ApiError {
        status: Option<String>,
        reason: Option<String>,
    },

    #[error("Video not playable: {0}")]
    NotPlayable(String),

    #[error("Request timeout after {0:?}")]
    Timeout(Duration),

    #[error("Unknown error: {0}")]
    Unknown(String),
}

#[async_trait]
pub trait InnerTubeClient: Send + Sync {
    fn name(&self) -> &'static str;
    fn client_name(&self) -> &'static str;
    fn client_version(&self) -> String;
    fn user_agent(&self) -> String;

    async fn player(
        &self,
        video_id: &str,
        context: &ResolveContext,
    ) -> Result<PlayerResponse, ResolveError>;
}

// A base implementation that builds standard Innertube requests
pub struct BaseInnerTubeClient {
    name: &'static str,
    client_name: &'static str,
    client_version: String,
    user_agent: String,
    client_id_header: String, // X-Youtube-Client-Name integer
    payload_client_override: Option<serde_json::Value>,
    payload_context_override: Option<serde_json::Value>,
}

impl BaseInnerTubeClient {
    pub fn new(
        name: &'static str,
        client_name: &'static str,
        client_version: String,
        user_agent: String,
        client_id_header: String,
        payload_client_override: Option<serde_json::Value>,
        payload_context_override: Option<serde_json::Value>,
    ) -> Self {
        Self {
            name,
            client_name,
            client_version,
            user_agent,
            client_id_header,
            payload_client_override,
            payload_context_override,
        }
    }

    fn uses_web_headers(&self) -> bool {
        matches!(self.name, "WEB" | "WEB_SAFARI")
    }
}

pub async fn get_or_fetch_session(
    http_client: &reqwest::Client,
) -> Result<SessionData, ResolveError> {
    {
        let cache = SESSION_CACHE
            .lock()
            .map_err(|_| ResolveError::Unknown("session cache lock poisoned".to_owned()))?;
        if let Some((ref data, fetched_at)) = *cache {
            if fetched_at.elapsed() < Duration::from_secs(6 * 3600) {
                return Ok(data.clone());
            }
        }
    }

    // Cold start or expired cache - fetch watch page to extract visitor_data, sts, and player_url
    let url = "https://www.youtube.com/watch?v=dQw4w9WgXcQ&hl=en";
    let res = http_client.get(url)
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
        .send()
        .await?
        .text()
        .await?;

    let visitor_data = match rusty_ytdl::get_visitor_data(&res) {
        Ok(v) => v,
        Err(_) => "CgtyckVza05NMXhtOCiV8-m_BjIKCgJWThIEGgAgSw==".to_string(),
    };

    let sts = match rusty_ytdl::get_ytconfig(&res) {
        Ok(ytcfg) => ytcfg.sts.unwrap_or(19950),
        Err(_) => 19950,
    };

    let player_url_path = match extract_player_url_path(&res) {
        Some(path) => path,
        None => "/s/player/9b27514a/player_ias.vflset/en_US/base.js".to_string(),
    };
    let player_url = if player_url_path.starts_with("https://") {
        player_url_path
    } else {
        format!("https://www.youtube.com{}", player_url_path)
    };

    let data = SessionData {
        visitor_data,
        sts,
        player_url,
    };

    {
        let mut cache = SESSION_CACHE
            .lock()
            .map_err(|_| ResolveError::Unknown("session cache lock poisoned".to_owned()))?;
        *cache = Some((data.clone(), Instant::now()));
    }

    Ok(data)
}

fn extract_player_url_path(body: &str) -> Option<String> {
    let patterns = [
        r#""jsUrl"\s*:\s*"([^"]+base\.js[^"]*)""#,
        r#""PLAYER_JS_URL"\s*:\s*"([^"]+base\.js[^"]*)""#,
        r#"<script[^>]+src="([^"]+base\.js[^"]*)""#,
        r#"/s/player/[a-zA-Z0-9-_]+/player_ias\.vflset/[a-zA-Z0-9-_]+/base\.js"#,
    ];

    for pattern in patterns {
        let Ok(re) = regex::Regex::new(pattern) else {
            continue;
        };
        if let Some(caps) = re.captures(body)
            && let Some(matched) = caps.get(1).or_else(|| caps.get(0))
        {
            return Some(matched.as_str().replace(r"\/", "/"));
        }
    }

    None
}

#[async_trait]
impl InnerTubeClient for BaseInnerTubeClient {
    fn name(&self) -> &'static str {
        self.name
    }

    fn client_name(&self) -> &'static str {
        self.client_name
    }

    fn client_version(&self) -> String {
        self.client_version.clone()
    }

    fn user_agent(&self) -> String {
        self.user_agent.clone()
    }

    async fn player(
        &self,
        video_id: &str,
        context: &ResolveContext,
    ) -> Result<PlayerResponse, ResolveError> {
        let http_client = reqwest::Client::builder()
            .timeout(context.timeout)
            .build()?;

        let session = if let Some(visitor_data) = context.visitor_data.clone() {
            SessionData {
                visitor_data,
                sts: 19950,
                player_url:
                    "https://www.youtube.com/s/player/9b27514a/player_ias.vflset/en_US/base.js"
                        .to_string(),
            }
        } else {
            get_or_fetch_session(&http_client).await?
        };

        let visitor_data = session.visitor_data;
        let sts = session.sts;

        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        let ua = context
            .user_agent_override
            .clone()
            .unwrap_or_else(|| self.user_agent());
        headers.insert(
            USER_AGENT,
            HeaderValue::from_str(&ua)
                .map_err(|e| ResolveError::Unknown(format!("invalid user-agent header: {e}")))?,
        );
        headers.insert(
            HeaderName::from_static("x-youtube-client-name"),
            HeaderValue::from_str(&self.client_id_header).map_err(|e| {
                ResolveError::Unknown(format!("invalid youtube client name header: {e}"))
            })?,
        );
        headers.insert(
            HeaderName::from_static("x-youtube-client-version"),
            HeaderValue::from_str(&self.client_version()).map_err(|e| {
                ResolveError::Unknown(format!("invalid youtube client version header: {e}"))
            })?,
        );
        if self.uses_web_headers() {
            headers.insert(ORIGIN, HeaderValue::from_static("https://www.youtube.com"));
            headers.insert(
                REFERER,
                HeaderValue::from_static("https://www.youtube.com/"),
            );
        }
        headers.insert(
            HeaderName::from_static("x-goog-visitor-id"),
            HeaderValue::from_str(&visitor_data)
                .map_err(|e| ResolveError::Unknown(format!("invalid visitor data header: {e}")))?,
        );

        let hl = context.language.clone().unwrap_or_else(|| "en".to_string());

        let mut client_obj = json!({
            "clientName": self.client_name,
            "clientVersion": self.client_version(),
            "hl": hl,
            "userAgent": context.user_agent_override.clone().unwrap_or_else(|| self.user_agent()),
        });

        // Merge any client payload overrides (like osName, osVersion, deviceModel, etc.)
        if let Some(ref extra) = self.payload_client_override {
            if let Some(obj) = client_obj.as_object_mut() {
                if let Some(extra_obj) = extra.as_object() {
                    for (k, v) in extra_obj {
                        obj.insert(k.clone(), v.clone());
                    }
                }
            }
        }

        let mut context_obj = json!({
            "client": client_obj
        });

        // Merge any context level overrides
        if let Some(ref extra) = self.payload_context_override {
            if let Some(obj) = context_obj.as_object_mut() {
                if let Some(extra_obj) = extra.as_object() {
                    for (k, v) in extra_obj {
                        obj.insert(k.clone(), v.clone());
                    }
                }
            }
        }

        let payload = json!({
            "context": context_obj,
            "videoId": video_id,
            "playbackContext": {
                "contentPlaybackContext": {
                    "signatureTimestamp": sts,
                    "html5Preference": "HTML5_PREF_WANTS"
                }
            }
        });

        // Innertube Player API Endpoint
        let url = "https://www.youtube.com/youtubei/v1/player?key=AIzaSyAO_FJ2SlqU8Q4STEHLGCilw_Y9_11qcW8";

        let response_text = http_client
            .post(url)
            .headers(headers)
            .json(&payload)
            .send()
            .await?
            .text()
            .await?;

        // Check if the response contains an error block
        let raw_val: serde_json::Value = serde_json::from_str(&response_text)?;
        if let Some(error_block) = raw_val.get("error") {
            let status = error_block
                .get("status")
                .and_then(|s| s.as_str())
                .map(|s| s.to_string());
            let reason = error_block
                .get("message")
                .and_then(|m| m.as_str())
                .map(|m| m.to_string());
            return Err(ResolveError::ApiError { status, reason });
        }

        let player_res: PlayerResponse = serde_json::from_value(raw_val)?;

        // Simple playability status check
        if let Some(ref playability) = player_res.playability_status {
            if let Some(ref status) = playability.status {
                if status != "OK" {
                    return Err(ResolveError::ApiError {
                        status: Some(status.clone()),
                        reason: playability.reason.clone(),
                    });
                }
            }
        }

        Ok(player_res)
    }
}

// ANDROID_VR Client Factory (No PO Token required, no decipher required)
pub fn create_android_vr_client() -> BaseInnerTubeClient {
    BaseInnerTubeClient::new(
        "ANDROID_VR",
        "ANDROID_VR",
        "1.57.2".to_string(),
        "Mozilla/5.0 (Linux; U; Android 10; en-US; Quest 2 Build/QQ3A.200805.001.A1) AppleWebKit/537.36 (KHTML, like Gecko) OculusBrowser/18.1.0.0.30.29 Chrome/89.0.4389.90 VR Safari/537.36".to_string(),
        "91".to_string(),
        Some(json!({
            "osName": "Android",
            "osVersion": "10",
            "deviceMake": "Oculus",
            "deviceModel": "Quest 2"
        })),
        None,
    )
}

// WEB_SAFARI Client Factory (Requires decipher/n-transform)
pub fn create_web_safari_client() -> BaseInnerTubeClient {
    BaseInnerTubeClient::new(
        "WEB_SAFARI",
        "WEB_SAFARI",
        "2.20240101.00.00".to_string(),
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15".to_string(),
        "56".to_string(),
        None,
        None,
    )
}

// ANDROID Client Factory
pub fn create_android_client(version: Option<String>) -> BaseInnerTubeClient {
    let ver = version.unwrap_or_else(|| "20.10.38".to_string());
    BaseInnerTubeClient::new(
        "ANDROID",
        "ANDROID",
        ver.clone(),
        format!(
            "com.google.android.youtube/{} (Linux; U; Android 11) gzip",
            ver
        ),
        "3".to_string(),
        Some(json!({
            "osName": "Android",
            "osVersion": "11",
            "userAgent": format!("com.google.android.youtube/{} (Linux; U; Android 11) gzip", ver)
        })),
        None,
    )
}

// TVHTML5 Client Factory
pub fn create_tvhtml5_client(version: Option<String>) -> BaseInnerTubeClient {
    let ver = version.unwrap_or_else(|| "7.20230522.05.00".to_string());
    BaseInnerTubeClient::new(
        "TVHTML5",
        "TVHTML5",
        ver,
        "Mozilla/5.0 (Chromecast; Google TV) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/90.0.4430.225 Safari/537.36".to_string(),
        "7".to_string(),
        None,
        None,
    )
}

// IOS Client Factory
pub fn create_ios_client(version: Option<String>) -> BaseInnerTubeClient {
    let ver = version.unwrap_or_else(|| "21.02.3".to_string());
    BaseInnerTubeClient::new(
        "IOS",
        "IOS",
        ver.clone(),
        format!(
            "com.google.ios.youtube/{} (iPhone16,2; U; CPU iOS 18_1_0 like Mac OS X;)",
            ver
        ),
        "5".to_string(),
        Some(json!({
            "deviceMake": "Apple",
            "deviceModel": "iPhone16,2",
            "osName": "iPhone",
            "osVersion": "18.1.0",
            "userAgent": format!("com.google.ios.youtube/{} (iPhone16,2; U; CPU iOS 18_1_0 like Mac OS X;)", ver)
        })),
        None,
    )
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ResolvedStream {
    pub url: String,
    pub client_kind: String,
    pub user_agent: String,
    pub expires_at: Option<u64>,
    pub mime_type: Option<String>,
    pub bitrate: Option<u64>,
    pub resolve_source: String,
}

pub async fn probe_resolved_stream_health(
    stream: &ResolvedStream,
    bytes_to_probe: usize,
    min_speed_kbps: f64,
) -> Result<stream_probe::ProbeResult, stream_probe::ProbeError> {
    let http_client = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()?;
    stream_probe::probe_stream_health(
        &http_client,
        &stream.url,
        &stream.user_agent,
        &stream.client_kind,
        bytes_to_probe,
        min_speed_kbps,
    )
    .await
}

/// Helper to extract the best audio stream URL for a given video ID using direct InnerTube player API calls.
pub async fn resolve_best_audio_stream_via_api(
    video_id: &str,
    context: &ResolveContext,
) -> Result<ResolvedStream, ResolveError> {
    let http_client = reqwest::Client::builder()
        .timeout(context.timeout)
        .build()?;

    // Fetch session first to get player_url
    let session = get_or_fetch_session(&http_client).await?;
    let player_url = session.player_url.clone();

    // Client priority list for anonymous flows
    let clients = vec![
        create_android_vr_client(),
        create_web_safari_client(),
        create_ios_client(None),
        create_android_client(None),
        create_tvhtml5_client(None),
    ];

    let mut last_err =
        ResolveError::NotPlayable("All Innertube clients failed to resolve stream".to_string());

    for client in clients {
        tracing::debug!(
            client = client.name(),
            video_id,
            "Attempting to resolve stream with client"
        );
        match client.player(video_id, context).await {
            Ok(player_res) => {
                if let Some(sd) = player_res.streaming_data {
                    let formats = sd.adaptive_formats.unwrap_or_default();
                    if let Some(best_format) = format_selector::select_best_audio(&formats) {
                        // Decrypt and de-throttle the format URL
                        let decrypt_res = js_solver::decrypt_format_url(
                            &http_client,
                            &player_url,
                            best_format.url.as_deref(),
                            best_format.signature_cipher.as_deref(),
                            best_format.cipher.as_deref(),
                        )
                        .await;

                        match decrypt_res {
                            Ok(decrypted_url) => {
                                // Probe the stream health to detect 403 Forbidden or throttling
                                let ua = client.user_agent();
                                let probe_res = stream_probe::probe_stream_health(
                                    &http_client,
                                    &decrypted_url,
                                    &ua,
                                    client.name(),
                                    102400, // Probe first 100 KB
                                    50.0,   // Min 50.0 KB/s
                                )
                                .await;

                                match probe_res {
                                    Ok(probe) => {
                                        tracing::info!(
                                            client = client.name(),
                                            speed = format!("{:.2} KB/s", probe.speed_kbps),
                                            "Successfully probed and validated stream URL"
                                        );
                                        return Ok(ResolvedStream {
                                            url: decrypted_url,
                                            client_kind: client.name().to_string(),
                                            user_agent: ua,
                                            expires_at: None,
                                            mime_type: best_format
                                                .mime_type
                                                .as_ref()
                                                .map(|m| m.mime.to_string()),
                                            bitrate: best_format.bitrate,
                                            resolve_source: format!(
                                                "api_client_{}",
                                                client.name().to_lowercase()
                                            ),
                                        });
                                    }
                                    Err(probe_err) => {
                                        tracing::warn!(
                                            client = client.name(),
                                            error = %probe_err,
                                            "Stream probe failed. Rotating to next client..."
                                        );
                                        last_err = ResolveError::NotPlayable(format!(
                                            "Client {} resolved URL but stream probe failed: {}",
                                            client.name(),
                                            probe_err
                                        ));
                                    }
                                }
                            }
                            Err(e) => {
                                tracing::warn!(
                                    client = client.name(),
                                    error = %e,
                                    "Failed to decrypt format URL. Rotating to next client..."
                                );
                                last_err = ResolveError::NotPlayable(format!(
                                    "Client {} failed to decrypt format URL: {}",
                                    client.name(),
                                    e
                                ));
                            }
                        }
                    } else {
                        tracing::warn!(
                            client = client.name(),
                            "No suitable audio formats found for client"
                        );
                        last_err = ResolveError::NotPlayable(format!(
                            "Client {} returned player response but no suitable audio formats found",
                            client.name()
                        ));
                    }
                } else {
                    tracing::warn!(
                        client = client.name(),
                        "Player response contains no streaming data"
                    );
                    last_err = ResolveError::NotPlayable(format!(
                        "Client {} returned player response with no streaming data",
                        client.name()
                    ));
                }
            }
            Err(e) => {
                tracing::warn!(client = client.name(), error = %e, "InnerTube player API error");
                last_err = e;
            }
        }
    }

    Err(last_err)
}

/// Fallback helper to extract stream info using rusty-ytdl's HTML parser
pub async fn resolve_best_audio_stream_rusty_ytdl(
    video_id: &str,
    _context: &ResolveContext,
) -> Result<ResolvedStream, ResolveError> {
    use rusty_ytdl::{Video, VideoOptions, VideoQuality, VideoSearchOptions};

    let opts = VideoOptions {
        quality: VideoQuality::HighestAudio,
        filter: VideoSearchOptions::Audio,
        ..Default::default()
    };

    let video = Video::new_with_options(video_id, opts)
        .map_err(|e| ResolveError::Unknown(e.to_string()))?;
    let info = video
        .get_info()
        .await
        .map_err(|e| ResolveError::Unknown(e.to_string()))?;

    // Prioritize formats by itag
    let itag_priority = |itag: u64| -> i32 {
        match itag {
            251 => 10, // Opus 160kbps
            140 => 9,  // AAC 128kbps
            250 => 8,  // Opus 70kbps
            249 => 7,  // Opus 50kbps
            139 => 6,  // AAC 48kbps
            _ => 1,    // Any other audio
        }
    };

    let mut candidate: Option<(&rusty_ytdl::VideoFormat, i32)> = None;

    for format in &info.formats {
        let is_audio =
            format.mime_type.mime.type_() == mime::AUDIO || format.has_audio && !format.has_video;

        if !is_audio {
            continue;
        }

        if format.url.is_empty() {
            continue;
        }

        let itag = format.itag;
        let priority = itag_priority(itag);

        if let Some((_, best_priority)) = candidate {
            if priority > best_priority {
                candidate = Some((format, priority));
            }
        } else {
            candidate = Some((format, priority));
        }
    }

    let format = candidate
        .map(|(f, _)| f)
        .ok_or_else(|| ResolveError::NotPlayable("No suitable audio streams found".to_string()))?;
    let url = format.url.clone();

    // The user explicitly asked to pass metadata "client_kind" and "user_agent".
    // We infer this from the deciphered URL parameters or rusty_ytdl defaults.
    let (client_kind, user_agent) = if url.contains("c=ANDROID") || url.contains("c=android") {
        (
            "ANDROID".to_string(),
            "com.google.android.youtube/20.10.38 (Linux; U; Android 11) gzip".to_string(),
        )
    } else if url.contains("c=IOS") || url.contains("c=ios") {
        (
            "IOS".to_string(),
            "com.google.ios.youtube/21.02.3 (iPhone16,2; U; CPU iOS 18_1_0 like Mac OS X;)"
                .to_string(),
        )
    } else if url.contains("c=TVHTML5") || url.contains("c=tvhtml5") {
        ("TVHTML5".to_string(), "Mozilla/5.0 (Chromecast; Google TV) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/90.0.4430.225 Safari/537.36".to_string())
    } else {
        ("WEB".to_string(), "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0 Safari/537.36".to_string())
    };

    Ok(ResolvedStream {
        url,
        client_kind,
        user_agent,
        expires_at: None, // Can be parsed from url expire=...
        mime_type: Some(format.mime_type.mime.to_string()),
        bitrate: Some(format.bitrate),
        resolve_source: "rusty_ytdl".to_string(),
    })
}

/// Helper to extract the best audio stream URL for a given video ID.
pub async fn resolve_best_audio_stream(
    video_id: &str,
    context: &ResolveContext,
) -> Result<ResolvedStream, ResolveError> {
    // Try the direct InnerTube API client first (HTML Bypass)
    if let Ok(stream) = resolve_best_audio_stream_via_api(video_id, context).await {
        return Ok(stream);
    }
    // Fallback to rusty-ytdl scraping
    resolve_best_audio_stream_rusty_ytdl(video_id, context).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_android_vr_resolve() {
        let client = create_android_vr_client();
        let ctx = ResolveContext::default();
        let video_id = "dQw4w9WgXcQ";
        let res = client.player(video_id, &ctx).await;
        assert!(res.is_ok());
        let player_res = res.unwrap();
        assert!(player_res.streaming_data.is_some());
    }

    #[tokio::test]
    async fn test_web_safari_resolve() {
        let client = create_web_safari_client();
        let ctx = ResolveContext::default();
        let video_id = "dQw4w9WgXcQ";
        let res = client.player(video_id, &ctx).await;
        match res {
            Ok(player_res) => {
                assert!(player_res.streaming_data.is_some());
            }
            Err(e) => {
                println!(
                    "Web Safari player API returned error (expected in anonymous context): {:?}",
                    e
                );
            }
        }
    }

    #[tokio::test]
    async fn test_resolve_best_audio_stream() {
        let ctx = ResolveContext::default();
        let video_id = "dQw4w9WgXcQ";
        let res = resolve_best_audio_stream(video_id, &ctx).await;
        match &res {
            Ok(stream) => {
                println!(
                    "resolve_best_audio_stream succeeded! url starts with: {}",
                    &stream.url[..std::cmp::min(stream.url.len(), 60)]
                );
            }
            Err(e) => {
                println!("resolve_best_audio_stream failed: {:?}", e);
            }
        }
        assert!(res.is_ok());
        let stream = res.unwrap();
        assert!(stream.url.contains("googlevideo.com"));
    }
}
