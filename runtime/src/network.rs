use crate::text;
use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::time::Duration;

fn destination(def: &Value) -> Result<reqwest::Url> {
    let raw = text(&def["url"])?;
    let url = reqwest::Url::parse(raw).context("Invalid connector URL")?;
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
    Ok(url)
}

fn optional_text<'a>(def: &'a Value, field: &str) -> Result<Option<&'a str>> {
    match def.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => Ok(Some(
            value
                .as_str()
                .with_context(|| format!("Invalid connector {field}"))?,
        )),
    }
}

/// Validate a definition without resolving DNS, reading credentials, or opening a socket.
pub(crate) fn validate_definition(def: &Value) -> Result<reqwest::Url> {
    let url = destination(def)?;
    ensure!(
        matches!(def["method"].as_str(), Some("GET" | "POST")),
        "Unsupported connector method"
    );
    if let Some(header) = optional_text(def, "idempotency_header")? {
        ensure!(
            header.eq_ignore_ascii_case("Idempotency-Key")
                || header.eq_ignore_ascii_case("X-Idempotency-Key"),
            "Unsupported idempotency header"
        );
    }
    if let Some(var) = optional_text(def, "credential")? {
        ensure!(
            var.starts_with("RELAI_CONNECTOR_")
                && var.ends_with("_TOKEN")
                && var
                    .bytes()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == b'_'),
            "Invalid connector credential reference"
        );
    }
    Ok(url)
}

fn prepare(plan: &Value) -> Result<(reqwest::Client, reqwest::Request)> {
    let def = &plan["definition"];
    let url = validate_definition(def)?;
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
    if let Some(var) = optional_text(def, "credential")? {
        req = req.bearer_auth(std::env::var(var).context("Missing connector credential")?);
    }
    if let Some(header) = optional_text(def, "idempotency_header")? {
        req = req.header(header, text(&plan["operation"])?);
    }
    Ok((client, req.build().context("Invalid connector request")?))
}

/// Return a completion payload. Only a fully prepared request reaches execute.
pub async fn exchange(plan: &Value) -> Value {
    let (client, request) = match prepare(plan) {
        Ok(prepared) => prepared,
        Err(error) => return json!({"failure":"not_sent","error":error.to_string()}),
    };
    let read = request.method() == reqwest::Method::GET;
    let response = match client.execute(request).await {
        Ok(response) => response,
        Err(_) => return json!({"error":"Connector transport failed after execution began"}),
    };
    let status = response.status();
    if !status.is_success() {
        let error = format!("Connector returned HTTP {}", status.as_u16());
        return if read {
            json!({"failure":"http_error","error":error,"status":status.as_u16()})
        } else {
            json!({"error":error})
        };
    }
    match capture(response).await {
        Ok(result) => json!({"result":result}),
        Err(error) => json!({"error":error.to_string()}),
    }
}

async fn capture(mut res: reqwest::Response) -> Result<Value> {
    let status = res.status();
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
    while let Some(c) = res
        .chunk()
        .await
        .context("Connector response read failed")?
    {
        ensure!(
            bytes.len() + c.len() <= 1024 * 1024,
            "Connector response exceeds 1 MiB"
        );
        bytes.extend_from_slice(&c)
    }
    Ok(json!({"status":status.as_u16(),"base64":STANDARD.encode(bytes),"media_type":mime}))
}
