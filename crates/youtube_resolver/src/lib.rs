use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::json;
use std::str::FromStr;
use std::time::Duration;
pub use rusty_ytdl::PlayerResponse;

#[derive(Debug, Clone)]
pub struct ResolveContext {
    pub po_token: Option<String>,
    pub visitor_data: Option<String>,
    pub cookies: Option<String>,
    pub user_agent_override: Option<String>,
    pub language: Option<String>,
    pub region: Option<String>,
    pub timeout: Duration,
    pub trace_id: Option<String>,
}

impl Default for ResolveContext {
    fn default() -> Self {
        Self {
            po_token: None,
            visitor_data: None,
            cookies: None,
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

        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_str("content-type").unwrap(),
            HeaderValue::from_str("application/json").unwrap(),
        );
        headers.insert(
            HeaderName::from_str("User-Agent").unwrap(),
            HeaderValue::from_str(&context.user_agent_override.clone().unwrap_or_else(|| self.user_agent())).unwrap(),
        );
        headers.insert(
            HeaderName::from_str("X-Youtube-Client-Name").unwrap(),
            HeaderValue::from_str(&self.client_id_header).unwrap(),
        );
        headers.insert(
            HeaderName::from_str("X-Youtube-Client-Version").unwrap(),
            HeaderValue::from_str(&self.client_version()).unwrap(),
        );
        headers.insert(
            HeaderName::from_str("Origin").unwrap(),
            HeaderValue::from_str("https://www.youtube.com").unwrap(),
        );
        headers.insert(
            HeaderName::from_str("Referer").unwrap(),
            HeaderValue::from_str("https://www.youtube.com/").unwrap(),
        );

        if let Some(ref visitor_data) = context.visitor_data {
            headers.insert(
                HeaderName::from_str("X-Goog-Visitor-Id").unwrap(),
                HeaderValue::from_str(visitor_data).unwrap(),
            );
        }

        if let Some(ref cookies) = context.cookies {
            headers.insert(
                HeaderName::from_str("Cookie").unwrap(),
                HeaderValue::from_str(cookies).unwrap(),
            );
        }

        let hl = context.language.clone().unwrap_or_else(|| "en".to_string());
        let gl = context.region.clone().unwrap_or_else(|| "US".to_string());

        let mut client_obj = json!({
            "clientName": self.client_name,
            "clientVersion": self.client_version(),
            "hl": hl,
            "gl": gl,
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

        let mut payload = json!({
            "context": context_obj,
            "videoId": video_id,
            "playbackContext": {
                "contentPlaybackContext": {
                    "signatureTimestamp": 19950, // recent signature timestamp fallback
                    "html5Preference": "HTML5_PREF_WANTS"
                }
            }
        });

        // Add serviceIntegrityDimensions if po_token is supplied
        if let Some(ref po_token) = context.po_token {
            if let Some(payload_obj) = payload.as_object_mut() {
                payload_obj.insert(
                    "serviceIntegrityDimensions".to_string(),
                    json!({
                        "poToken": po_token
                    }),
                );
            }
        }

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

        let player_res: PlayerResponse = serde_json::from_str(&response_text)?;

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

// ANDROID Client Factory
pub fn create_android_client(version: Option<String>) -> BaseInnerTubeClient {
    let ver = version.unwrap_or_else(|| "19.30.36".to_string());
    BaseInnerTubeClient::new(
        "ANDROID",
        "ANDROID",
        ver.clone(),
        format!("com.google.android.youtube/{} (Linux; U; Android 11) gzip", ver),
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
    let ver = version.unwrap_or_else(|| "19.29.1".to_string());
    BaseInnerTubeClient::new(
        "IOS",
        "IOS",
        ver.clone(),
        format!("com.google.ios.youtube/{} (iPhone16,2; U; CPU iOS 17_5_1 like Mac OS X;)", ver),
        "5".to_string(),
        Some(json!({
            "deviceMake": "Apple",
            "deviceModel": "iPhone16,2",
            "osName": "iPhone",
            "osVersion": "17.5.1.21F90",
            "userAgent": format!("com.google.ios.youtube/{} (iPhone16,2; U; CPU iOS 17_5_1 like Mac OS X;)", ver)
        })),
        None,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_android_resolve() {
        let client = create_android_client(None);
        let ctx = ResolveContext::default();
        // A standard valid video ID, e.g. "dQw4w9WgXcQ"
        let video_id = "dQw4w9WgXcQ";
        let res = client.player(video_id, &ctx).await;
        match &res {
            Ok(player_res) => {
                println!("Android resolve succeeded! Playability status: {:?}", player_res.playability_status);
            }
            Err(e) => {
                println!("Android resolve failed: {:?}", e);
            }
        }
        assert!(res.is_ok());
    }

    #[tokio::test]
    async fn test_ios_resolve() {
        let client = create_ios_client(None);
        let ctx = ResolveContext::default();
        let video_id = "dQw4w9WgXcQ";
        let res = client.player(video_id, &ctx).await;
        match &res {
            Ok(player_res) => {
                println!("IOS resolve succeeded! Playability status: {:?}", player_res.playability_status);
            }
            Err(e) => {
                println!("IOS resolve failed: {:?}", e);
            }
        }
        assert!(res.is_ok());
    }
}

