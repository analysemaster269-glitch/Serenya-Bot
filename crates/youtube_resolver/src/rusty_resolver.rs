use crate::{ResolveContext, ResolveError, ResolvedStream};

pub async fn resolve_best_audio_stream_rusty_ytdl(
    video_id: &str,
    context: &ResolveContext,
) -> Result<ResolvedStream, ResolveError> {
    use rusty_ytdl::{RequestOptions, Video, VideoOptions, VideoQuality, VideoSearchOptions};

    let opts = VideoOptions {
        quality: VideoQuality::HighestAudio,
        filter: VideoSearchOptions::Audio,
        request_options: RequestOptions {
            client: Some(context.http_client.clone()),
            ..Default::default()
        },
        ..Default::default()
    };
    let video = Video::new_with_options(video_id, opts)
        .map_err(|e| ResolveError::Unknown(e.to_string()))?;
    let info = video
        .get_info()
        .await
        .map_err(|e| ResolveError::Unknown(e.to_string()))?;
    let format = select_rusty_format(&info.formats)?;
    let (client_kind, user_agent) = infer_client_headers(&format.url);

    Ok(ResolvedStream {
        url: format.url.clone(),
        client_kind,
        user_agent,
        expires_at: None,
        mime_type: Some(format.mime_type.mime.to_string()),
        bitrate: Some(format.bitrate),
        resolve_source: "rusty_ytdl".to_string(),
    })
}

fn select_rusty_format(
    formats: &[rusty_ytdl::VideoFormat],
) -> Result<&rusty_ytdl::VideoFormat, ResolveError> {
    formats
        .iter()
        .filter(|format| {
            let is_audio = format.mime_type.mime.type_() == mime::AUDIO
                || format.has_audio && !format.has_video;
            is_audio && !format.url.is_empty()
        })
        .max_by_key(|format| itag_priority(format.itag))
        .ok_or_else(|| ResolveError::NotPlayable("No suitable audio streams found".to_string()))
}

fn itag_priority(itag: u64) -> i32 {
    match itag {
        251 => 10,
        140 => 9,
        250 => 8,
        249 => 7,
        139 => 6,
        _ => 1,
    }
}

fn infer_client_headers(url: &str) -> (String, String) {
    if url.contains("c=ANDROID") || url.contains("c=android") {
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
        (
            "TVHTML5".to_string(),
            "Mozilla/5.0 (Chromecast; Google TV) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/90.0.4430.225 Safari/537.36".to_string(),
        )
    } else {
        (
            "WEB".to_string(),
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0 Safari/537.36".to_string(),
        )
    }
}
