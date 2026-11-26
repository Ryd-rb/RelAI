//! Bounded HTTP and JSON-RPC transports for application-declared destinations.
use super::{LIMIT, text};
use anyhow::{Context, Result, bail, ensure};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::time::Duration;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Message, client::IntoClientRequest, http::HeaderValue},
};

pub async fn exchange(plan: &Value, config: &Value) -> Result<Value> {
    let dest = config["destinations"]
        .get(text(&plan["destination"])?)
        .context("Unregistered destination")?;
    let url = if let Some(var) = dest["url_env"].as_str() {
        std::env::var(var).with_context(|| format!("Set {var} for this provider"))?
    } else {
        text(&dest["url"])?.into()
    };
    let parsed = reqwest::Url::parse(&url)?;
    ensure!(
        parsed.username().is_empty()
            && parsed.password().is_none()
            && parsed.query().is_none()
            && parsed.fragment().is_none(),
        "Invalid service URL"
    );
    let local = parsed
        .host_str()
        .is_some_and(|h| h == "127.0.0.1" || h == "[::1]" || h == "localhost");
    ensure!(
        parsed.scheme() == "https" || (local && matches!(parsed.scheme(), "http" | "ws")),
        "Use HTTPS or a loopback endpoint"
    );
    tokio::time::timeout(Duration::from_secs(150), async {
        if dest["transport"] == "websocket" {
            ensure!(
                local && parsed.scheme() == "ws",
                "Expected a local WebSocket service"
            );
            let mut req = url.into_client_request()?;
            let token_path = std::env::var(text(&dest["token_file_env"])?)?;
            let token = tokio::fs::read_to_string(token_path).await?;
            req.headers_mut().insert(
                "Authorization",
                HeaderValue::from_str(&format!("Bearer {}", token.trim()))?,
            );
            let (mut ws, _) = connect_async(req).await?;
            let mut saved = json!({});
            let mut total = 0usize;
            for step in plan["steps"].as_array().context("Missing RPC steps")? {
                if let Some(send) = step.get("send") {
                    let send = bind(send, &saved)?;
                    ws.send(Message::Text(send.to_string().into())).await?;
                }
                if let Some(filter) = step.get("wait") {
                    loop {
                        let frame = ws.next().await.context("Provider disconnected")??;
                        let Message::Text(data) = frame else { continue };
                        total += data.len();
                        ensure!(total < 32 * 1024 * 1024, "Provider event budget exceeded");
                        let msg: Value = serde_json::from_str(&data)?;
                        if msg.get("method").is_some() && msg.get("id").is_some() {
                            bail!("Provider requested a tool or approval")
                        }
                        if let Some(stop) = step.get("until")
                            && msg.pointer(text(&stop["pointer"])?) == Some(&stop["equals"])
                        {
                            bail!("{}", text(&stop["error"])?);
                        }
                        if let Some(method) = filter["method"].as_str()
                            && msg["method"] != method
                        {
                            continue;
                        }
                        let field = text(&filter["pointer"])?;
                        if msg.pointer(field) != Some(&filter["equals"]) {
                            continue;
                        }
                        if let Some(error) = msg.get("error") {
                            bail!("RPC failure: {}", error["message"])
                        }
                        if let Some(check) = step.get("require") {
                            ensure!(
                                msg.pointer(text(&check["pointer"])?) == Some(&check["equals"]),
                                "{}",
                                text(&check["error"])?
                            )
                        }
                        if let Some(key) = step["save"].as_str() {
                            saved[key] = msg
                        }
                        break;
                    }
                }
            }
            ws.close(None).await.ok();
            Ok(saved)
        } else {
            let client = reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(140))
                .build()?;
            let mut req = client.post(parsed).json(&plan["body"]);
            if let Some(headers) = dest["headers"].as_object() {
                for (k, v) in headers {
                    req = req.header(k, text(v)?);
                }
            }
            if let Some(var) = dest["key_env"].as_str() {
                match std::env::var(var) {
                    Ok(key) => {
                        if dest["key_header"] == "Authorization" {
                            req = req.bearer_auth(key)
                        } else {
                            req = req.header(text(&dest["key_header"])?, key)
                        }
                    }
                    Err(_) if dest["optional_key"] == true => (),
                    Err(_) => bail!("Set {var} for this provider"),
                }
            }
            let mut res = req.send().await?;
            let status = res.status();
            let mut bytes = Vec::new();
            while let Some(c) = res.chunk().await? {
                ensure!(
                    bytes.len() + c.len() <= LIMIT,
                    "Response exceeds input limit"
                );
                bytes.extend_from_slice(&c);
            }
            let body: Value = serde_json::from_slice(&bytes)?;
            ensure!(
                status.is_success(),
                "Provider HTTP {status}: {}",
                body["error"]["message"]
            );
            Ok(body)
        }
    })
    .await
    .context("Provider deadline exceeded")?
}
fn bind(value: &Value, saved: &Value) -> Result<Value> {
    if let Some(pointer) = value.get("$saved").and_then(Value::as_str) {
        return Ok(saved
            .pointer(pointer)
            .context("Missing JSON-RPC binding")?
            .clone());
    }
    Ok(match value {
        Value::Array(a) => Value::Array(a.iter().map(|v| bind(v, saved)).collect::<Result<_>>()?),
        Value::Object(o) => Value::Object(
            o.iter()
                .map(|(k, v)| Ok((k.clone(), bind(v, saved)?)))
                .collect::<Result<_>>()?,
        ),
        _ => value.clone(),
    })
}
