use boa_engine::{Context, Source};
use moka::future::Cache;
use regex::Regex;
use sha1::{Digest, Sha1};
use std::sync::LazyLock;
use std::time::Duration;

// Caches parsed JS functions from player URLs
// Key: SHA-1 hash of player_url
// Value: (decipher_function_body, n_transform_function_body)
static FUNCTIONS_CACHE: LazyLock<Cache<String, (String, String)>> = LazyLock::new(|| {
    Cache::builder()
        .max_capacity(128)
        .time_to_live(Duration::from_secs(24 * 3600)) // Cache for 24 hours
        .build()
});

// Caches solved n-throttle inputs
// Key: SHA-1 hash of player_url + "_" + n_input
// Value: n_output
static N_CACHE: LazyLock<Cache<String, String>> = LazyLock::new(|| {
    Cache::builder()
        .max_capacity(4096)
        .time_to_live(Duration::from_secs(12 * 3600)) // Cache for 12 hours
        .build()
});

/// Computes SHA-1 hex hash of a string
pub fn sha1_hash(input: &str) -> String {
    let mut hasher = Sha1::new();
    hasher.update(input.as_bytes());
    let result = hasher.finalize();
    format!("{:x}", result)
}

/// Helper function to extract a string between two bounds
fn between(str: &str, a: &str, b: &str) -> String {
    if let Some(ndx) = str.find(a) {
        let sub = &str[ndx + a.len()..];
        if let Some(end_ndx) = sub.find(b) {
            return sub[..end_ndx].to_string();
        }
    }
    String::new()
}

/// Helper to get balanced curly braces matching for JS block
fn cut_after_js(str: &str) -> Option<&str> {
    if !str.starts_with('{') {
        return None;
    }
    let mut open_braces = 0;
    for (i, c) in str.char_indices() {
        if c == '{' {
            open_braces += 1;
        } else if c == '}' {
            open_braces -= 1;
            if open_braces == 0 {
                return Some(&str[..=i]);
            }
        }
    }
    None
}

/// Parses the base.js body to extract decipher and n-transform functions.
/// Returns: (decipher_js, ncode_js)
fn extract_player_functions(body: &str) -> (String, String) {
    let mut decipher_js = String::new();
    let mut ncode_js = String::new();

    // 1. Extract decipher function
    let decipher_name = between(body, r#"a.set("alr","yes");c&&(c="#, "(decodeURIC");
    if !decipher_name.is_empty() {
        let function_start = format!("{decipher_name}=function(a)");
        if let Some(ndx) = body.find(function_start.as_str()) {
            let sub_body = &body[ndx + function_start.len()..];
            if let Some(cut_body) = cut_after_js(sub_body) {
                // Find helper object containing manipulations (reverse, slice, swap)
                let helper_obj_name = between(cut_body, "a=a.split(\"\");", ".");
                let mut full_body = format!("var {function_start}{cut_body}");

                if !helper_obj_name.is_empty() {
                    let helper_start = format!("var {helper_obj_name}={{");
                    if let Some(h_ndx) = body.find(helper_start.as_str()) {
                        let h_sub = &body[h_ndx + helper_start.len() - 1..];
                        if let Some(h_cut) = cut_after_js(h_sub) {
                            full_body = format!("var {helper_obj_name}={h_cut};{full_body}");
                        }
                    }
                }

                full_body.retain(|c| c != '\n');
                decipher_js = full_body;
            }
        }
    }

    // 2. Extract n-transform function
    let mut ncode_name = between(body, r#"c=a.get(b))&&(c="#, "(c)");
    if ncode_name.contains('[') {
        let left_name = format!(
            "var {splitted_function_name}=[",
            splitted_function_name = ncode_name.split('[').next().unwrap_or("")
        );
        ncode_name = between(body, left_name.as_str(), "]");
    }

    if ncode_name.is_empty() {
        // Fallback regex scan for ncode function
        if let Ok(re) = Regex::new(r";\s*([a-zA-Z0-9_$]+)\s*=\s*function\([a-zA-Z0-9_$]+\)\s*\{") {
            for caps in re.captures_iter(body) {
                if let Some(m) = caps.get(1) {
                    let name = m.as_str();
                    let Some(full_match) = caps.get(0) else {
                        continue;
                    };
                    let start_pos = full_match.end();
                    if let Some(end_pos) = body[start_pos..].find("};") {
                        let f_body = &body[start_pos..start_pos + end_pos];
                        if f_body.contains("enhanced_except_") {
                            ncode_name = name.to_string();
                            break;
                        }
                    }
                }
            }
        }
    }

    if !ncode_name.is_empty() {
        let function_start = format!("{ncode_name}=function(a)");
        if let Some(ndx) = body.find(function_start.as_str()) {
            let sub_body = &body[ndx + function_start.len()..];
            if let Some(cut_body) = cut_after_js(sub_body) {
                let mut full_body = format!("var {function_start}{cut_body};");
                full_body.retain(|c| c != '\n');
                ncode_js = full_body;
            }
        }
    }

    (decipher_js, ncode_js)
}

/// Fetches player JS from URL and parses its functions, caching them by player URL hash.
pub async fn get_or_fetch_player_functions(
    http_client: &reqwest::Client,
    player_url: &str,
) -> Result<(String, String), String> {
    let url_hash = sha1_hash(player_url);
    if let Some(funcs) = FUNCTIONS_CACHE.get(&url_hash).await {
        return Ok(funcs);
    }

    tracing::info!(
        player_url,
        "Fetching player JS to extract solver functions..."
    );
    let response = http_client
        .get(player_url)
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
        .send()
        .await
        .map_err(|e| format!("Failed to download player JS: {e}"))?
        .text()
        .await
        .map_err(|e| format!("Failed to read player JS text: {e}"))?;

    let funcs = extract_player_functions(&response);
    if funcs.0.is_empty() && funcs.1.is_empty() {
        tracing::warn!(
            player_url,
            "Could not extract decipher or n-transform functions from player JS"
        );
    }

    FUNCTIONS_CACHE.insert(url_hash, funcs.clone()).await;
    Ok(funcs)
}

/// Solves signature cipher deciphering.
pub fn solve_signature(decipher_js: &str, encrypted_sig: &str) -> Result<String, String> {
    if decipher_js.is_empty() {
        return Ok(encrypted_sig.to_string());
    }

    let mut context = Context::default();
    context
        .eval(Source::from_bytes(decipher_js.as_bytes()))
        .map_err(|e| format!("JS evaluation error during decipher setup: {e}"))?;

    // The decipher function name can be found in the JS script body: "var <name>=function(a)"
    let func_name = decipher_js
        .split('=')
        .next()
        .and_then(|s| s.split("var ").last())
        .map(|s| s.trim())
        .ok_or_else(|| "Could not determine decipher function name".to_string())?;

    let js_call = format!("{func_name}(\"{encrypted_sig}\")");
    let result = context
        .eval(Source::from_bytes(js_call.as_bytes()))
        .map_err(|e| format!("JS execution error during decipher call: {e}"))?;

    let decrypted = result
        .as_string()
        .and_then(|js_str| js_str.to_std_string().ok())
        .ok_or_else(|| "Decipher function returned non-string value".to_string())?;

    Ok(decrypted)
}

/// Solves n-throttle challenge.
pub async fn solve_n_throttle(
    http_client: &reqwest::Client,
    player_url: &str,
    n_input: &str,
) -> Result<String, String> {
    if n_input.is_empty() {
        return Ok(String::new());
    }

    let url_hash = sha1_hash(player_url);
    let cache_key = format!("{url_hash}_{n_input}");
    if let Some(cached_n) = N_CACHE.get(&cache_key).await {
        return Ok(cached_n);
    }

    let (_, ncode_js) = get_or_fetch_player_functions(http_client, player_url).await?;
    if ncode_js.is_empty() {
        return Ok(n_input.to_string());
    }

    let n_output = {
        let mut context = Context::default();
        context
            .eval(Source::from_bytes(ncode_js.as_bytes()))
            .map_err(|e| format!("JS evaluation error during ncode setup: {e}"))?;

        let func_name = ncode_js
            .split('=')
            .next()
            .and_then(|s| s.split("var ").last())
            .map(|s| s.trim())
            .ok_or_else(|| "Could not determine ncode function name".to_string())?;

        let js_call = format!("{func_name}(\"{n_input}\")");
        let result = context
            .eval(Source::from_bytes(js_call.as_bytes()))
            .map_err(|e| format!("JS execution error during ncode call: {e}"))?;

        result
            .as_string()
            .and_then(|js_str| js_str.to_std_string().ok())
            .ok_or_else(|| "ncode function returned non-string value".to_string())?
    };

    N_CACHE.insert(cache_key, n_output.clone()).await;
    Ok(n_output)
}

/// Deciphers signature and n-throttle on a format cipher/URL.
pub async fn decrypt_format_url(
    http_client: &reqwest::Client,
    player_url: &str,
    format_url: Option<&str>,
    sig_cipher: Option<&str>,
    cipher: Option<&str>,
) -> Result<String, String> {
    let mut final_url = if let Some(cipher_str) = sig_cipher.or(cipher) {
        let (decipher_js, _) = get_or_fetch_player_functions(http_client, player_url).await?;
        // Parse sig cipher query parameters
        let params = url::form_urlencoded::parse(cipher_str.as_bytes());
        let mut url_val = String::new();
        let mut s_val = String::new();
        let mut sp_val = "sig".to_string(); // default param name

        for (k, v) in params {
            match k.as_ref() {
                "url" => url_val = v.into_owned(),
                "s" => s_val = v.into_owned(),
                "sp" => sp_val = v.into_owned(),
                _ => {}
            }
        }

        if url_val.is_empty() {
            return Err("Signature cipher is missing URL parameter".to_string());
        }

        if !s_val.is_empty() && decipher_js.is_empty() {
            return Err(
                "Signature cipher is present but no decipher function was extracted".to_string(),
            );
        }
        let decrypted_sig = solve_signature(&decipher_js, &s_val)?;

        let mut parsed_url =
            url::Url::parse(&url_val).map_err(|e| format!("Invalid URL in cipher: {e}"))?;

        parsed_url
            .query_pairs_mut()
            .append_pair(&sp_val, &decrypted_sig);
        parsed_url.to_string()
    } else if let Some(url_str) = format_url {
        url_str.to_string()
    } else {
        return Err("No stream URL or signature cipher found in format".to_string());
    };

    // Now, apply n-throttle transformation if present
    if let Ok(mut parsed) = url::Url::parse(&final_url) {
        let mut n_val = String::new();
        for (k, v) in parsed.query_pairs() {
            if k == "n" {
                n_val = v.into_owned();
                break;
            }
        }

        if !n_val.is_empty() {
            if let Ok(solved_n) = solve_n_throttle(http_client, player_url, &n_val).await {
                // Rebuild query parameters replacing n
                let pairs: Vec<(String, String)> = parsed
                    .query_pairs()
                    .map(|(k, v)| {
                        if k == "n" {
                            (k.into_owned(), solved_n.clone())
                        } else {
                            (k.into_owned(), v.into_owned())
                        }
                    })
                    .collect();
                parsed.query_pairs_mut().clear().extend_pairs(pairs);
                final_url = parsed.to_string();
            }
        }
    }

    Ok(final_url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_player_function_extraction() -> Result<(), Box<dyn std::error::Error>> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()?;
        let session = crate::get_or_fetch_session(&client).await?;
        let (decipher_js, ncode_js) = get_or_fetch_player_functions(&client, &session.player_url)
            .await
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

        println!(
            "Resolved player URL: {}, decipher bytes: {}, ncode bytes: {}",
            session.player_url,
            decipher_js.len(),
            ncode_js.len()
        );

        assert!(session.player_url.contains("base.js"));

        let direct_url = "https://example.com/videoplayback?expire=1&n=plain";
        let decrypted =
            decrypt_format_url(&client, &session.player_url, Some(direct_url), None, None)
                .await
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        assert!(decrypted.starts_with("https://example.com/videoplayback"));
        Ok(())
    }
}
