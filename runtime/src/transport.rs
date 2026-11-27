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
                if step["collect_codex_turn"] == true {
                    let thread = saved
                        .pointer("/thread/result/thread/id")
                        .and_then(Value::as_str)
                        .context("Missing Codex thread ID")?;
                    let ack = step
                        .pointer("/send/id")
                        .context("Missing turn request ID")?;
                    let mut turn = None;
                    let mut pending = std::collections::HashMap::new();
                    loop {
                        let frame = ws.next().await.context("Provider disconnected")??;
                        let Message::Text(data) = frame else { continue };
                        let msg = rpc_event(data.as_str(), &mut total)?;
                        if msg.get("id") == Some(ack) {
                            if let Some(error) = msg.get("error") {
                                bail!("RPC failure: {}", error["message"]);
                            }
                            let id = text(&msg["result"]["turn"]["id"])?;
                            ensure!(turn.is_none(), "Duplicate Codex turn acknowledgement");
                            pending.retain(|key: &String, _| key == id);
                            turn = Some(id.to_owned());
                        } else if msg.pointer("/params/threadId").and_then(Value::as_str)
                            == Some(thread)
                            && let Some(id) = codex_event_turn(&msg)
                            && turn.as_deref().is_none_or(|expected| expected == id)
                        {
                            pending
                                .entry(id.to_owned())
                                .or_insert_with(CodexTurn::default)
                                .observe(msg);
                        }
                        if let Some(id) = &turn
                            && let Some(events) = pending.get_mut(id)
                            && let Some(completion) = events.completion.take()
                        {
                            ensure!(
                                completion["params"]["turn"]["status"] == "completed",
                                "Codex turn did not complete."
                            );
                            let message = events
                                .final_answer
                                .take()
                                .or_else(|| events.legacy.take())
                                .context("Codex completed the turn without an answer")?;
                            saved["message"] = message;
                            saved["completion"] = completion;
                            break;
                        }
                    }
                }
                if let Some(filter) = step.get("wait") {
                    loop {
                        let frame = ws.next().await.context("Provider disconnected")??;
                        let Message::Text(data) = frame else { continue };
                        let msg = rpc_event(data.as_str(), &mut total)?;
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
            if !status.is_success() {
                bail!("Provider HTTP {status}");
            }
            let mut bytes = Vec::new();
            while let Some(c) = res.chunk().await? {
                ensure!(
                    bytes.len() + c.len() <= LIMIT,
                    "Response exceeds input limit"
                );
                bytes.extend_from_slice(&c);
            }
            Ok(serde_json::from_slice(&bytes)?)
        }
    })
    .await
    .context("Provider deadline exceeded")?
}
fn rpc_event(data: &str, total: &mut usize) -> Result<Value> {
    *total += data.len();
    ensure!(*total < 32 * 1024 * 1024, "Provider event budget exceeded");
    let msg: Value = serde_json::from_str(data)?;
    if msg.get("method").is_some() && msg.get("id").is_some() {
        bail!("Provider requested a tool or approval");
    }
    Ok(msg)
}

fn codex_event_turn(msg: &Value) -> Option<&str> {
    match msg["method"].as_str()? {
        "item/completed" if msg["params"]["item"]["type"] == "agentMessage" => {
            msg["params"]["turnId"].as_str()
        }
        "turn/completed" => msg["params"]["turn"]["id"].as_str(),
        _ => None,
    }
}

#[derive(Default)]
struct CodexTurn {
    final_answer: Option<Value>,
    legacy: Option<Value>,
    completion: Option<Value>,
}

impl CodexTurn {
    fn observe(&mut self, msg: Value) {
        if self.completion.is_some() {
            return;
        }
        if msg["method"] == "turn/completed" {
            self.completion = Some(msg);
            return;
        }
        match msg.pointer("/params/item/phase") {
            Some(phase) if phase == "final_answer" => self.final_answer = Some(msg),
            None | Some(Value::Null) => self.legacy = Some(msg),
            _ => {}
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_turn_prefers_final_answer_and_ignores_commentary() {
        let message = |phase: Value, body: &str| {
            json!({"method":"item/completed","params":{"threadId":"thread","turnId":"turn",
                "item":{"type":"agentMessage","text":body,"phase":phase}}})
        };
        let mut turn = CodexTurn::default();
        let first = message(json!("commentary"), "Preparing a response.");
        assert_eq!(codex_event_turn(&first), Some("turn"));
        turn.observe(first);
        assert!(turn.legacy.is_none());
        turn.observe(message(Value::Null, "First older-protocol message"));
        let latest = message(Value::Null, "Last older-protocol message");
        turn.observe(latest.clone());
        assert_eq!(turn.legacy.as_ref(), Some(&latest));
        let final_answer = message(json!("final_answer"), "Final answer.");
        turn.observe(final_answer.clone());
        turn.observe(message(Value::Null, "Another phase-less message"));
        let completed = json!({"method":"turn/completed",
            "params":{"threadId":"thread","turn":{"id":"turn","status":"completed"}}});
        assert_eq!(codex_event_turn(&completed), Some("turn"));
        turn.observe(completed.clone());
        turn.observe(message(json!("final_answer"), "Out-of-order notification"));
        assert_eq!(turn.completion, Some(completed));
        assert_eq!(turn.final_answer.or(turn.legacy), Some(final_answer));
    }

    #[tokio::test]
    async fn provider_http_error_keeps_its_status_without_a_json_body() {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let router = axum::Router::new()
            .route(
                "/html",
                axum::routing::post(|| async {
                    (
                        axum::http::StatusCode::BAD_GATEWAY,
                        "<html>gateway error</html>",
                    )
                }),
            )
            .route(
                "/empty",
                axum::routing::post(|| async { axum::http::StatusCode::SERVICE_UNAVAILABLE }),
            );
        let service = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        for (path, status) in [("html", "502"), ("empty", "503")] {
            let url = format!("{origin}/{path}");
            let err = exchange(
                &json!({"destination":"mock","body":{}}),
                &json!({"destinations":{"mock":{"url":url}}}),
            )
            .await
            .unwrap_err()
            .to_string();
            assert!(err.contains(status), "{err}");
            assert!(!err.contains("<html>"));
        }
        service.abort();
    }
}
