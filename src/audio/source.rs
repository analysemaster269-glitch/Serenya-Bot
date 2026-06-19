use poise::serenity_prelude as serenity;
use songbird::input::{Compose, YoutubeDl};

use crate::core::{SourceType, Track};
use crate::database::DatabaseManager;
use crate::utils::SerenyaError;

pub async fn resolve_input(
    query: &str,
    user_id: u64,
    db: &DatabaseManager,
    http_client: &reqwest::Client,
) -> Result<Vec<Track>, SerenyaError> {
    let query_trimmed = query.trim();

    // 1. Check user-owned playlist
    if let Some(playlist) = db.get_user_playlist(user_id, query_trimmed).await {
        let mut tracks = Vec::new();
        for t in playlist.tracks {
            tracks.push(Track {
                title: t.title,
                url: t.url,
                duration: t.duration_secs.map(std::time::Duration::from_secs),
                requester_id: serenity::UserId::new(user_id),
                requester_name: "".to_owned(),
                source_type: SourceType::Playlist,
            });
        }
        return Ok(tracks);
    }

    // 2. Check if it's a URL or search query
    let is_url = query_trimmed.starts_with("http://") || query_trimmed.starts_with("https://");
    let resolve_url = if is_url {
        query_trimmed.to_owned()
    } else {
        format!("ytsearch:{}", query_trimmed)
    };

    let mut ytdl = YoutubeDl::new(http_client.clone(), resolve_url.clone());
    let metadata = ytdl.aux_metadata().await.map_err(|e| {
        SerenyaError::Audio(format!(
            "failed to fetch metadata for {}: {}",
            resolve_url, e
        ))
    })?;

    let track = Track {
        title: metadata.title.unwrap_or_else(|| "Unknown Title".to_owned()),
        url: metadata
            .source_url
            .unwrap_or_else(|| query_trimmed.to_owned()),
        duration: metadata.duration,
        requester_id: serenity::UserId::new(user_id),
        requester_name: "".to_owned(),
        source_type: if is_url {
            SourceType::Url
        } else {
            SourceType::Search
        },
    };

    Ok(vec![track])
}
