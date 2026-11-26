use anyhow::{Context, Result};
use axum::{
    Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderMap, Response, StatusCode},
    response::{Html, IntoResponse},
    routing::{get, post},
};
use fluxr_app_host::{App, transport};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
#[derive(Clone)]
struct Server {
    app: Arc<Mutex<App>>,
    token: String,
    origin: String,
}
type Reply = Result<Json<Value>, (StatusCode, String)>;
fn access(s: &Server, h: &HeaderMap) -> bool {
    let header = |k: &str| h.get(k).and_then(|h| h.to_str().ok());
    header("host") == s.origin.strip_prefix("http://")
        && header("origin").is_none_or(|o| o == s.origin)
        && header("authorization").is_some_and(|h| h.strip_prefix("Bearer ") == Some(&s.token))
}
async fn command(
    State(s): State<Server>,
    Path(name): Path<String>,
    h: HeaderMap,
    Json(input): Json<Value>,
) -> Reply {
    if !access(&s, &h) {
        return Err((StatusCode::FORBIDDEN, "Invalid local session".into()));
    }
    tokio::task::spawn_blocking(move || {
        let mut app = s.app.lock().unwrap();
        if !app.metadata()["public"]
            .as_array()
            .unwrap()
            .contains(&json!(name))
        {
            return Err((StatusCode::NOT_FOUND, "Unknown action".into()));
        }
        app.invoke(&name, input)
            .map(Json)
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("{e:#}")))
    })
    .await
    .map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Worker unavailable".into(),
        )
    })?
}
async fn asset(
    State(s): State<Server>,
    Path(name): Path<String>,
) -> Result<Response<Body>, StatusCode> {
    if name == "client.js" {
        return Ok(Response::builder()
            .header("content-type", "text/javascript; charset=utf-8")
            .body(Body::from(include_str!("client.js")))
            .unwrap());
    }
    let app = s.app.lock().unwrap();
    let content = app.metadata()["assets"]
        .get(&name)
        .ok_or(StatusCode::NOT_FOUND)?;
    let bytes = fluxr_app_host::read(&app.bundle.join(&name)).map_err(|_| StatusCode::NOT_FOUND)?;
    Ok(Response::builder()
        .header("content-type", content.as_str().unwrap())
        .header("x-content-type-options", "nosniff")
        .body(Body::from(bytes))
        .unwrap())
}
async fn work(s: Server) {
    let slots = Arc::new(tokio::sync::Semaphore::new(3));
    loop {
        let permit = slots.clone().acquire_owned().await.unwrap();
        let next = {
            let mut a = s.app.lock().unwrap();
            a.invoke("poll", Value::Null)
                .map(|v| (v, a.metadata().clone()))
        };
        match next {
            Ok((v, config)) if !v.is_null() => {
                let app = s.app.clone();
                tokio::spawn(async move {
                    let _slot = permit;
                    if let Some(export) = v["local"].as_str() {
                        let mut app = app.lock().unwrap();
                        let outcome = app.invoke(export, v.get("arguments").unwrap_or(&v).clone());
                        if let Some(then) = v["then"].as_str() {
                            let next = match outcome {
                                Ok(result) => json!({"job":v["job"],"result":result}),
                                Err(e) => json!({"job":v["job"],"error":format!("{e:#}")}),
                            };
                            if let Err(e) = app.invoke(then, next) {
                                eprintln!("Local completion: {e:#}")
                            }
                        } else if let Err(e) = outcome {
                            let _ = app
                                .invoke("hold", json!({"job":v["job"],"error":format!("{e:#}")}));
                        }
                        return;
                    }
                    let result = transport::exchange(&v["transport"], &config).await;
                    let payload = match result {
                        Ok(body) => json!({"job":v["job"],"attempt":v["attempt"],"response":body}),
                        Err(e) => {
                            json!({"job":v["job"],"attempt":v["attempt"],"error":format!("{e:#}")})
                        }
                    };
                    let mut app = app.lock().unwrap();
                    if let Err(e) = app.invoke("receive", payload.clone()) {
                        let _ = app.invoke(
                            "hold",
                            json!({"job":v["job"],"attempt":v["attempt"],"error":format!("{e:#}")}),
                        );
                    }
                });
            }
            result => {
                if let Err(e) = result {
                    eprintln!("Scheduler: {e:#}");
                }
                drop(permit);
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        }
    }
}
#[tokio::main]
async fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let bundle = PathBuf::from(
        args.next()
            .context("Usage: fluxr-app-host BUNDLE DATA [PORT]")?,
    );
    let data = PathBuf::from(args.next().context("Missing data directory")?);
    let port: u16 = args.next().as_deref().unwrap_or("0").parse()?;
    let app = App::open(&bundle, &data)?;
    let socket = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    let origin = format!("http://{}", socket.local_addr()?);
    let state = Server {
        app: Arc::new(Mutex::new(app)),
        token: uuid::Uuid::new_v4().simple().to_string(),
        origin: origin.clone(),
    };
    println!("Open {origin}/#{}", state.token);
    let app = Router::new()
        .route(
            "/",
            get(|| async { Html(include_str!("shell.html")).into_response() }),
        )
        .route("/assets/{name}", get(asset))
        .route("/api/{name}", post(command))
        .layer(DefaultBodyLimit::max(18 * 1024 * 1024))
        .with_state(state.clone());
    tokio::spawn(work(state));
    axum::serve(socket, app).await?;
    Ok(())
}
