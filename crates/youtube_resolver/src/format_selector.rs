use rusty_ytdl::StreamingDataFormat;

/// Filters formats to keep only audio-only streams.
pub fn filter_audio_only(formats: &[StreamingDataFormat]) -> Vec<StreamingDataFormat> {
    formats
        .iter()
        .filter(|f| {
            if let Some(ref mime_type) = f.mime_type {
                mime_type.mime.type_() == mime::AUDIO
            } else {
                false
            }
        })
        .cloned()
        .collect()
}

/// Rejects formats that require DRM decryption or contain indicators of DRM protection.
pub fn reject_drm(formats: &[StreamingDataFormat]) -> Vec<StreamingDataFormat> {
    formats
        .iter()
        .filter(|f| {
            // DRM formats often have mimeType with "enc" (e.g., audio/enc-isom)
            if let Some(ref mime_type) = f.mime_type {
                let mime_str = mime_type.mime.to_string();
                if mime_str.contains("enc-") || mime_str.contains("drm") {
                    return false;
                }
            }

            // If it lacks URL, signature cipher, and normal cipher altogether, it is unplayable or DRM
            if f.url.is_none() && f.signature_cipher.is_none() && f.cipher.is_none() {
                return false;
            }

            true
        })
        .cloned()
        .collect()
}

/// Prefers WebM container with Opus codec, prioritized by high-quality itag first.
/// - Itag 251: Opus 160kbps
/// - Itag 250: Opus 70kbps
/// - Itag 249: Opus 50kbps
pub fn prefer_opus_webm(formats: &[StreamingDataFormat]) -> Option<StreamingDataFormat> {
    let mut opus_formats: Vec<StreamingDataFormat> = formats
        .iter()
        .filter(|f| {
            if let Some(ref mime_type) = f.mime_type {
                let mime_str = mime_type.mime.to_string();
                mime_str.contains("webm") && mime_str.contains("opus")
            } else {
                false
            }
        })
        .cloned()
        .collect();

    // Sort by priority of itag (highest itag first: 251 -> 250 -> 249 -> others)
    opus_formats.sort_by_key(|f| match f.itag {
        Some(251) => 3,
        Some(250) => 2,
        Some(249) => 1,
        _ => 0,
    });

    opus_formats.last().cloned()
}

/// Falls back to M4A container with AAC codec, prioritized by high-quality itag first.
/// - Itag 140: AAC 128kbps
/// - Itag 139: AAC 48kbps
pub fn fallback_m4a(formats: &[StreamingDataFormat]) -> Option<StreamingDataFormat> {
    let mut m4a_formats: Vec<StreamingDataFormat> = formats
        .iter()
        .filter(|f| {
            if let Some(ref mime_type) = f.mime_type {
                let mime_str = mime_type.mime.to_string();
                mime_str.contains("mp4") || mime_str.contains("m4a") || mime_str.contains("aac")
            } else {
                false
            }
        })
        .cloned()
        .collect();

    // Sort by priority of itag (highest first: 140 -> 139 -> others)
    m4a_formats.sort_by_key(|f| match f.itag {
        Some(140) => 2,
        Some(139) => 1,
        _ => 0,
    });

    m4a_formats.last().cloned()
}

/// Detects if a format is damaged, incomplete, or corrupted.
pub fn detect_damaged_format(format: &StreamingDataFormat) -> bool {
    // 1. Missing all stream source URL/cipher attributes
    if format.url.is_none() && format.signature_cipher.is_none() && format.cipher.is_none() {
        return true;
    }

    // 2. Corrupted content length (explicitly set to "0")
    if let Some(ref content_len) = format.content_length {
        if content_len == "0" {
            return true;
        }
    }

    // 3. Bitrate is present but zero
    if let Some(bitrate) = format.bitrate {
        if bitrate == 0 {
            return true;
        }
    }

    false
}

/// Selects the best audio format from a slice of formats, applying standard selection policy.
pub fn select_best_audio(formats: &[StreamingDataFormat]) -> Option<StreamingDataFormat> {
    let audio_only = filter_audio_only(formats);
    let non_drm = reject_drm(&audio_only);
    let playable: Vec<StreamingDataFormat> = non_drm
        .into_iter()
        .filter(|f| !detect_damaged_format(f))
        .collect();

    if playable.is_empty() {
        return None;
    }

    // 1. Try Opus WebM first
    if let Some(best_opus) = prefer_opus_webm(&playable) {
        return Some(best_opus);
    }

    // 2. Fallback to M4A AAC next
    if let Some(best_m4a) = fallback_m4a(&playable) {
        return Some(best_m4a);
    }

    // 3. Otherwise, pick any playable format with highest bitrate
    playable.into_iter().max_by_key(|f| f.bitrate.unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_format(
        itag: u64,
        mime_str: &str,
        url: Option<&str>,
        content_length: Option<&str>,
    ) -> StreamingDataFormat {
        StreamingDataFormat {
            itag: Some(itag),
            mime_type: Some(
                serde_json::from_str(&serde_json::to_string(mime_str).unwrap()).unwrap(),
            ),
            bitrate: Some(128000),
            width: None,
            height: None,
            init_range: None,
            index_range: None,
            last_modified: None,
            content_length: content_length.map(|s| s.to_string()),
            quality: None,
            fps: None,
            quality_label: None,
            projection_type: None,
            average_bitrate: None,
            high_replication: None,
            audio_quality: None,
            color_info: None,
            approx_duration_ms: None,
            audio_sample_rate: None,
            audio_channels: None,
            audio_bitrate: None,
            loudness_db: None,
            url: url.map(|s| s.to_string()),
            signature_cipher: None,
            cipher: None,
        }
    }

    #[test]
    fn test_filter_audio_only() {
        let f1 = make_test_format(251, "audio/webm; codecs=\"opus\"", Some("http://url"), None);
        let f2 = make_test_format(
            137,
            "video/mp4; codecs=\"avc1.640028\"",
            Some("http://url"),
            None,
        );
        let formats = vec![f1.clone(), f2];
        let audio = filter_audio_only(&formats);
        assert_eq!(audio.len(), 1);
        assert_eq!(audio[0].itag, Some(251));
    }

    #[test]
    fn test_reject_drm() {
        let f1 = make_test_format(251, "audio/webm; codecs=\"opus\"", Some("http://url"), None);
        let f2 = make_test_format(251, "audio/enc-isom", Some("http://url"), None);
        let f3 = StreamingDataFormat {
            itag: Some(251),
            mime_type: Some(
                serde_json::from_str(
                    &serde_json::to_string("audio/webm; codecs=\"opus\"").unwrap(),
                )
                .unwrap(),
            ),
            url: None,
            signature_cipher: None,
            cipher: None,
            ..Default::default()
        };
        let formats = vec![f1, f2, f3];
        let non_drm = reject_drm(&formats);
        assert_eq!(non_drm.len(), 1);
        assert_eq!(non_drm[0].url, Some("http://url".to_string()));
    }

    #[test]
    fn test_prefer_opus() {
        let f1 = make_test_format(249, "audio/webm; codecs=\"opus\"", Some("http://url"), None);
        let f2 = make_test_format(251, "audio/webm; codecs=\"opus\"", Some("http://url"), None);
        let f3 = make_test_format(
            140,
            "audio/mp4; codecs=\"mp4a.40.2\"",
            Some("http://url"),
            None,
        );
        let formats = vec![f1, f2, f3];
        let opus = prefer_opus_webm(&formats);
        assert!(opus.is_some());
        assert_eq!(opus.unwrap().itag, Some(251));
    }

    #[test]
    fn test_damaged_format() {
        let f1 = make_test_format(
            251,
            "audio/webm; codecs=\"opus\"",
            Some("http://url"),
            Some("0"),
        );
        let f2 = make_test_format(
            251,
            "audio/webm; codecs=\"opus\"",
            Some("http://url"),
            Some("12345"),
        );
        assert!(detect_damaged_format(&f1));
        assert!(!detect_damaged_format(&f2));
    }
}
