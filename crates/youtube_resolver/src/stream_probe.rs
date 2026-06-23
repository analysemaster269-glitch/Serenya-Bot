use std::time::{Duration, Instant};

#[derive(thiserror::Error, Debug)]
pub enum ProbeError {
    #[error("HTTP 403 Forbidden: Access denied by YouTube")]
    Http403,

    #[error("HTTP Status Error: status code {0}")]
    HttpStatus(reqwest::StatusCode),

    #[error("Network error: {0}")]
    Network(#[from] reqwest::Error),

    #[error("Empty response body from stream probe")]
    EmptyBody,

    #[error("Throttled: speed {0:.2} KB/s is below threshold {1:.2} KB/s")]
    Throttled(f64, f64),
}

#[derive(Debug, Clone)]
pub struct ProbeResult {
    pub total_bytes: usize,
    pub elapsed: Duration,
    pub speed_kbps: f64,
}

/// Probes the first N bytes of a stream URL to detect HTTP 403 or throttling.
///
/// - `stream_url`: The stream direct URL to probe.
/// - `user_agent`: User agent corresponding to the client.
/// - `bytes_to_probe`: Number of bytes to request (e.g. 102400 for 100 KB).
/// - `min_speed_kbps`: Minimum acceptable speed in Kilobytes per second (e.g., 50.0 KB/s).
pub async fn probe_stream_health(
    http_client: &reqwest::Client,
    stream_url: &str,
    user_agent: &str,
    client_kind: &str,
    bytes_to_probe: usize,
    min_speed_kbps: f64,
) -> Result<ProbeResult, ProbeError> {
    let started = Instant::now();

    // Prepare headers matching how ffmpeg downloads it
    let mut req = http_client
        .get(stream_url)
        .header("User-Agent", user_agent)
        .header("Range", format!("bytes=0-{}", bytes_to_probe - 1))
        .timeout(Duration::from_secs(4));

    if client_kind == "WEB" || client_kind == "WEB_SAFARI" || client_kind.is_empty() {
        req = req
            .header("Referer", "https://www.youtube.com/")
            .header("Origin", "https://www.youtube.com");
    }

    let mut res = req.send().await?;

    if res.status() == reqwest::StatusCode::FORBIDDEN {
        return Err(ProbeError::Http403);
    }

    if !res.status().is_success() && res.status() != reqwest::StatusCode::PARTIAL_CONTENT {
        return Err(ProbeError::HttpStatus(res.status()));
    }

    let mut total_bytes = 0;
    while let Some(chunk) = res.chunk().await? {
        total_bytes += chunk.len();
        if total_bytes >= bytes_to_probe {
            break;
        }
    }

    if total_bytes == 0 {
        return Err(ProbeError::EmptyBody);
    }

    let elapsed = started.elapsed();
    let elapsed_secs = elapsed.as_secs_f64();
    let speed_kbps = if elapsed_secs > 0.0 {
        (total_bytes as f64 / 1024.0) / elapsed_secs
    } else {
        (total_bytes as f64) / 1024.0
    };

    tracing::debug!(
        total_bytes,
        elapsed_ms = elapsed.as_millis(),
        speed_kbps = format!("{:.2} KB/s", speed_kbps),
        "Probed stream health successfully"
    );

    if speed_kbps < min_speed_kbps && total_bytes > 0 {
        return Err(ProbeError::Throttled(speed_kbps, min_speed_kbps));
    }

    Ok(ProbeResult {
        total_bytes,
        elapsed,
        speed_kbps,
    })
}
