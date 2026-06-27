use crate::{PlayerResponse, ResolveContext, ResolveError, SessionData, get_or_fetch_session};
use async_trait::async_trait;
use reqwest::header::{
    CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue, ORIGIN, REFERER, USER_AGENT,
};
use serde_json::json;

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

pub struct BaseInnerTubeClient {
    name: &'static str,
    client_name: &'static str,
    client_version: String,
    user_agent: String,
    client_id_header: String,
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
        let session = resolve_session(context, &context.http_client).await?;
        let headers = self.build_headers(context, &session)?;
        let payload = self.build_payload(video_id, context, &session);
        let response_text = context.http_client
            .post("https://www.youtube.com/youtubei/v1/player?key=AIzaSyAO_FJ2SlqU8Q4STEHLGCilw_Y9_11qcW8")
            .headers(headers)
            .json(&payload)
            .timeout(context.timeout)
            .send()
            .await?
            .text()
            .await?;
        parse_player_response(response_text)
    }
}

impl BaseInnerTubeClient {
    fn build_headers(
        &self,
        context: &ResolveContext,
        session: &SessionData,
    ) -> Result<HeaderMap, ResolveError> {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        let ua = context
            .user_agent_override
            .clone()
            .unwrap_or_else(|| self.user_agent());
        headers.insert(USER_AGENT, checked_header(&ua, "user-agent")?);
        headers.insert(
            HeaderName::from_static("x-youtube-client-name"),
            checked_header(&self.client_id_header, "youtube client name")?,
        );
        headers.insert(
            HeaderName::from_static("x-youtube-client-version"),
            checked_header(&self.client_version(), "youtube client version")?,
        );
        headers.insert(
            HeaderName::from_static("x-goog-visitor-id"),
            checked_header(&session.visitor_data, "visitor data")?,
        );
        if self.uses_web_headers() {
            headers.insert(ORIGIN, HeaderValue::from_static("https://www.youtube.com"));
            headers.insert(
                REFERER,
                HeaderValue::from_static("https://www.youtube.com/"),
            );
        }
        Ok(headers)
    }

    fn build_payload(
        &self,
        video_id: &str,
        context: &ResolveContext,
        session: &SessionData,
    ) -> serde_json::Value {
        let mut client_obj = json!({
            "clientName": self.client_name,
            "clientVersion": self.client_version(),
            "hl": context.language.clone().unwrap_or_else(|| "en".to_string()),
            "userAgent": context.user_agent_override.clone().unwrap_or_else(|| self.user_agent()),
        });
        merge_object(&mut client_obj, self.payload_client_override.as_ref());

        let mut context_obj = json!({ "client": client_obj });
        merge_object(&mut context_obj, self.payload_context_override.as_ref());

        json!({
            "context": context_obj,
            "videoId": video_id,
            "playbackContext": {
                "contentPlaybackContext": {
                    "signatureTimestamp": session.sts,
                    "html5Preference": "HTML5_PREF_WANTS"
                }
            }
        })
    }
}

async fn resolve_session(
    context: &ResolveContext,
    http_client: &reqwest::Client,
) -> Result<SessionData, ResolveError> {
    if let Some(visitor_data) = context.visitor_data.clone() {
        Ok(SessionData {
            visitor_data,
            sts: 19950,
            player_url: "https://www.youtube.com/s/player/9b27514a/player_ias.vflset/en_US/base.js"
                .to_string(),
        })
    } else {
        get_or_fetch_session(http_client).await
    }
}

fn checked_header(value: &str, label: &str) -> Result<HeaderValue, ResolveError> {
    HeaderValue::from_str(value)
        .map_err(|e| ResolveError::Unknown(format!("invalid {label} header: {e}")))
}

fn merge_object(target: &mut serde_json::Value, extra: Option<&serde_json::Value>) {
    if let Some(extra_obj) = extra.and_then(|value| value.as_object())
        && let Some(target_obj) = target.as_object_mut()
    {
        for (key, value) in extra_obj {
            target_obj.insert(key.clone(), value.clone());
        }
    }
}

fn parse_player_response(response_text: String) -> Result<PlayerResponse, ResolveError> {
    let raw_val: serde_json::Value = serde_json::from_str(&response_text)?;
    if let Some(error_block) = raw_val.get("error") {
        return Err(ResolveError::ApiError {
            status: error_block
                .get("status")
                .and_then(|s| s.as_str())
                .map(|s| s.to_string()),
            reason: error_block
                .get("message")
                .and_then(|m| m.as_str())
                .map(|m| m.to_string()),
        });
    }

    let player_res: PlayerResponse = serde_json::from_value(raw_val)?;
    if let Some(ref playability) = player_res.playability_status
        && let Some(ref status) = playability.status
        && status != "OK"
    {
        return Err(ResolveError::ApiError {
            status: Some(status.clone()),
            reason: playability.reason.clone(),
        });
    }
    Ok(player_res)
}
