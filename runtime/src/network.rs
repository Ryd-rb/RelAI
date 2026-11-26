use crate::text;
use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::time::Duration;

/// A single named operation on an application-approved destination.
pub async fn exchange(plan: &Value) -> Result<Value> {
    let def = &plan["definition"];
    let url = reqwest::Url::parse(text(&def["url"])?)?;
    let local = url
        .host_str()
        .is_some_and(|h| matches!(h, "127.0.0.1" | "[::1]" | "localhost"));
    ensure!(
        (url.scheme() == "https" || (url.scheme() == "http" && local))
            && url.username().is_empty()
            && url.password().is_none()
            && url.fragment().is_none(),
        "Invalid connector destination"
    );
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let mut req = if def["method"] == "GET" {
        client.get(url).query(&plan["arguments"])
    } else {
        ensure!(def["method"] == "POST", "Unsupported connector method");
        client.post(url).json(&plan["arguments"])
    };
    if let Some(var) = def["credential"].as_str() {
        ensure!(
            var.starts_with("RELAI_CONNECTOR_")
                && var.ends_with("_TOKEN")
                && var
                    .bytes()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == b'_'),
            "Invalid connector credential reference"
        );
        req = req.bearer_auth(std::env::var(var).context("Missing connector credential")?);
    }
    if let Some(header) = def["idempotency_header"].as_str() {
        ensure!(
            header.eq_ignore_ascii_case("Idempotency-Key")
                || header.eq_ignore_ascii_case("X-Idempotency-Key"),
            "Unsupported idempotency header"
        );
        req = req.header(header, text(&plan["operation"])?);
    }
    let mut res = req.send().await?;
    let status = res.status();
    ensure!(status.is_success(), "Connector returned HTTP {status}");
    let mime = res
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    let mut bytes = Vec::new();
    while let Some(c) = res.chunk().await? {
        ensure!(
            bytes.len() + c.len() <= 1024 * 1024,
            "Connector response exceeds 1 MiB"
        );
        bytes.extend_from_slice(&c)
    }
    Ok(json!({"status":status.as_u16(),"base64":STANDARD.encode(bytes),"media_type":mime}))
}
