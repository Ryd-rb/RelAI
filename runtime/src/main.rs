#[cfg(windows)]
mod desktop;
use anyhow::{Context, Result};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderMap, StatusCode},
    routing::post,
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
    #[cfg(windows)]
    if name == "choose_folder" {
        let folder = tokio::task::spawn_blocking(|| rfd::FileDialog::new().pick_folder())
            .await
            .map_err(|_| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Folder selection failed".into(),
                )
            })?;
        return Ok(Json(json!({"path":folder})));
    }
    if !s.app.lock().unwrap().metadata()["public"]
        .as_array()
        .unwrap()
        .contains(&json!(name))
    {
        return Err((StatusCode::NOT_FOUND, "Unknown action".into()));
    }
    invoke(s.app, &name, input)
        .await
        .map(Json)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("{e:#}")))
}
async fn invoke(app: Arc<Mutex<App>>, name: &str, input: Value) -> Result<Value> {
    let worker = app.clone();
    let export = name.to_owned();
    let v = tokio::task::spawn_blocking(move || worker.lock().unwrap().invoke(&export, input))
        .await??;
    if name != "tool_run" && name != "remote" {
        return Ok(v);
    }
    if let Some(plan) = v.get("$smart") {
        let private = app.lock().unwrap().data.clone();
        let result = fluxr_app_host::smart_http::exchange(plan, &private).await;
        let receipt = match result {
            Ok(result) => json!({"state":v["state"],"result":result}),
            Err(error) => json!({"state":v["state"],"error":format!("{error:#}")}),
        };
        return app.lock().unwrap().invoke("guest_transfer_done", receipt);
    }
    if let Some(plan) = v.get("$http") {
        let result = fluxr_app_host::network::exchange(plan).await;
        let receipt = match result {
            Ok(result) => json!({"state":v["state"],"result":result}),
            Err(e) => json!({"state":v["state"],"error":format!("{e:#}")}),
        };
        return app.lock().unwrap().invoke("guest_net_done", receipt);
    }
    Ok(v)
}
async fn work(app: Arc<Mutex<App>>, changed: Option<std::sync::mpsc::Sender<Value>>) {
    let slots = Arc::new(tokio::sync::Semaphore::new(3));
    loop {
        let permit = slots.clone().acquire_owned().await.unwrap();
        let next = {
            let mut a = app.lock().unwrap();
            let before = a.change_count();
            let job = a.invoke("poll", Value::Null);
            if before != a.change_count() {
                notify(&changed);
            }
            job.map(|v| (v, a.metadata().clone()))
        };
        match next {
            Ok((v, config)) if !v.is_null() => {
                let app = app.clone();
                let changed = changed.clone();
                tokio::spawn(async move {
                    let _slot = permit;
                    if let Some(export) = v["local"].as_str() {
                        let outcome = invoke(
                            app.clone(),
                            export,
                            v.get("arguments").unwrap_or(&v).clone(),
                        )
                        .await;
                        let mut app = app.lock().unwrap();
                        if let Some(then) = v["then"].as_str() {
                            let next = match outcome {
                                Ok(result) => {
                                    json!({"job":v["job"],"state":v["state"],"result":result})
                                }
                                Err(e) => {
                                    json!({"job":v["job"],"state":v["state"],"error":format!("{e:#}")})
                                }
                            };
                            if let Err(e) = app.invoke(then, next) {
                                eprintln!("Local completion: {e:#}")
                            }
                        } else if let Err(e) = outcome {
                            let _ = app.invoke(
                                "hold",
                                json!({"job":v["job"],"state":v["state"],"error":format!("{e:#}")}),
                            );
                        }
                        notify(&changed);
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
                    notify(&changed);
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
fn notify(channel: &Option<std::sync::mpsc::Sender<Value>>) {
    if let Some(tx) = channel {
        let _ = tx.send(json!({"type":"refresh"}));
    }
}
#[tokio::main]
async fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let bundle = PathBuf::from(
        args.next()
            .context("Usage: fluxr-app-host BUNDLE DATA [--headless [PORT]]")?,
    );
    let data = PathBuf::from(args.next().context("Missing data directory")?);
    let mode = args.next();
    let app = Arc::new(Mutex::new(App::open(&bundle, &data)?));
    let port: u16 = match mode.as_deref() {
        None => {
            #[cfg(windows)]
            return desktop::run(app);
            #[cfg(not(windows))]
            anyhow::bail!("Native desktop requires Windows. Use --headless for the local API.");
        }
        Some("--headless") => args.next().as_deref().unwrap_or("0").parse()?,
        Some(port) => port.parse()?,
    };
    anyhow::ensure!(args.next().is_none(), "Unexpected argument");
    let socket = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    let origin = format!("http://{}", socket.local_addr()?);
    let state = Server {
        app: app.clone(),
        token: uuid::Uuid::new_v4().simple().to_string(),
        origin: origin.clone(),
    };
    println!("Open {origin}/#{}", state.token);
    tokio::spawn(work(app, None));
    let app = Router::new()
        .route("/api/{name}", post(command))
        .layer(DefaultBodyLimit::max(18 * 1024 * 1024))
        .with_state(state);
    axum::serve(socket, app).await?;
    Ok(())
}
