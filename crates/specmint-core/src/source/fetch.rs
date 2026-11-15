use reqwest::StatusCode;
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue, IF_NONE_MATCH};
use tracing::debug;

use crate::error::{Error, Result};
use crate::source::{http_client, resolve_basic_auth_header};

pub enum FetchOutcome {
    NotModified,
    Body {
        bytes: Vec<u8>,
        etag: Option<String>,
    },
}

pub fn conditional_get(
    url: &str,
    basic_auth: Option<&str>,
    etag: Option<&str>,
) -> Result<FetchOutcome> {
    let client = http_client()?;
    let mut headers = HeaderMap::new();

    if let Some(cred) = basic_auth {
        let value = resolve_basic_auth_header(cred);
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&value)
                .map_err(|e| Error::source(format!("invalid Authorization header: {e}")))?,
        );
    }

    if let Some(tag) = etag {
        headers.insert(
            IF_NONE_MATCH,
            HeaderValue::from_str(tag).map_err(|e| Error::source(format!("invalid ETag: {e}")))?,
        );
    }

    debug!(%url, has_etag = etag.is_some(), has_auth = basic_auth.is_some(), "fetching OpenAPI schema");
    let response = client
        .get(url)
        .headers(headers)
        .send()
        .map_err(|e| classify_transport_error(url, basic_auth.is_some(), e))?;

    let status = response.status();
    if status == StatusCode::NOT_MODIFIED {
        return Ok(FetchOutcome::NotModified);
    }

    if !status.is_success() {
        let body_hint = response
            .text()
            .ok()
            .map(|t| {
                let t = t.trim();
                if t.is_empty() {
                    String::new()
                } else {
                    let snip: String = t.chars().take(200).collect();
                    format!(" — body: {snip}")
                }
            })
            .unwrap_or_default();
        return Err(Error::source(http_status_help(
            url,
            status,
            basic_auth.is_some(),
            &body_hint,
        )));
    }

    let etag = response
        .headers()
        .get(reqwest::header::ETAG)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);

    let bytes = response
        .bytes()
        .map_err(|e| Error::source(format!("GET {url}: failed reading response body: {e}")))?;
    Ok(FetchOutcome::Body {
        bytes: bytes.to_vec(),
        etag,
    })
}

fn classify_transport_error(url: &str, has_auth: bool, err: reqwest::Error) -> Error {
    let mut parts = vec![format!("GET {url} failed")];

    if err.is_timeout() {
        parts.push("request timed out (server slow, VPN, or firewall?)".into());
    } else if err.is_connect() {
        parts.push("could not connect (DNS, VPN, offline, or wrong host?)".into());
    } else if err.is_request() {
        parts.push("bad request setup".into());
    }

    parts.push(err.without_url().to_string());

    if !has_auth {
        parts.push(
            "hint: set SPECMINT_BASIC_AUTH='user:password' if the endpoint requires auth".into(),
        );
    }

    Error::source(parts.join(" — "))
}

fn http_status_help(url: &str, status: StatusCode, has_auth: bool, body_hint: &str) -> String {
    let code = status.as_u16();
    let mut msg = format!("GET {url} failed: HTTP {code}");

    match code {
        401 => {
            msg.push_str(" (unauthorized). Set SPECMINT_BASIC_AUTH='user:password' and retry.");
            if !has_auth {
                msg.push_str(" No auth header was sent.");
            }
        }
        403 => {
            msg.push_str(" (forbidden). Check credentials, environment URL, or VPN access.");
            if !has_auth {
                msg.push_str(" No auth header was sent — set SPECMINT_BASIC_AUTH.");
            }
        }
        404 => {
            msg.push_str(
                " (not found). Check [source].input — expected something like …/internal-docs/schema/.",
            );
        }
        407 => {
            msg.push_str(" (proxy auth required). Configure HTTPS proxy credentials.");
        }
        429 => {
            msg.push_str(" (rate limited). Wait and retry.");
        }
        500..=599 => {
            msg.push_str(" (server error). Backend may be down or schema endpoint broken.");
        }
        _ => {}
    }

    msg.push_str(body_hint);
    msg
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unauthorized_mentions_auth_env() {
        let msg = http_status_help(
            "https://example.test/schema/",
            StatusCode::UNAUTHORIZED,
            false,
            "",
        );
        assert!(msg.contains("SPECMINT_BASIC_AUTH"), "{msg}");
        assert!(msg.contains("No auth header"), "{msg}");
    }

    #[test]
    fn forbidden_with_auth_still_actionable() {
        let msg = http_status_help(
            "https://example.test/schema/",
            StatusCode::FORBIDDEN,
            true,
            "",
        );
        assert!(msg.contains("forbidden"), "{msg}");
        assert!(!msg.contains("No auth header"), "{msg}");
    }
}
