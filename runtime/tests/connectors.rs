use axum::{
    Json, Router,
    http::HeaderMap,
    routing::{get, post},
};
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader},
    path::Path,
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
struct Daemon(Child);
impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
#[tokio::test]
async fn named_connector_captures_bytes_and_consumes_an_exact_write_grant_once() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let writes = Arc::new(AtomicUsize::new(0));
    let counter = writes.clone();
    let fixture = Router::new()
        .route("/read", get(|| async { "from fixture" }))
        .route(
            "/write",
            post(move |headers: HeaderMap, Json(body): Json<Value>| {
                let counter = counter.clone();
                async move {
                    assert!(headers["idempotency-key"].to_str().unwrap().len() >= 32);
                    assert_eq!(body, json!({"amount":1}));
                    counter.fetch_add(1, Ordering::Relaxed);
                    "accepted"
                }
            }),
        );
    let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = socket.local_addr().unwrap();
    let fixture = tokio::spawn(async move { axum::serve(socket, fixture).await.unwrap() });
    let mut daemon = Daemon(
        Command::new(env!("CARGO_BIN_EXE_fluxr-app-host"))
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../app"))
            .arg(temp.path().join("state"))
            .arg("--headless")
            .stdout(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let mut line = String::new();
    BufReader::new(daemon.0.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let (url, token) = line
        .trim()
        .strip_prefix("Open ")
        .unwrap()
        .split_once("/#")
        .unwrap();
    let http = reqwest::Client::new();
    let api = |name: &str, body: Value| {
        http.post(format!("{url}/api/{name}"))
            .bearer_auth(token)
            .json(&body)
            .send()
    };
    let p = api("open_project", json!({"root":project}))
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json::<Value>()
        .await
        .unwrap()["id"]
        .clone();
    let definition = |method: &str, path: &str, properties: Value| json!({"name":path,"url":format!("http://{addr}/{path}"),"method":method,"tool":"web.fetch","idempotency_header":"Idempotency-Key","schema":{"type":"object","properties":properties,"additionalProperties":false}});
    let register = |def| {
        api(
            "connector",
            json!({"op":"register","project":p,"definition":def}),
        )
    };
    let id = register(definition("GET", "read", json!({})))
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap()["id"]
        .clone();
    let run = |tool: &str, id: &Value, args: Value, grant: &Value| {
        api(
            "tool_run",
            json!({"project":p,"tool":tool,"arguments":{"connector":id,"arguments":args,"operation":grant}}),
        )
    };
    let read = run("web.fetch", &id, json!({}), &Value::Null)
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(read["status"], 200);
    assert_eq!(read["artifact"]["byte_length"], 12);
    let result = api(
        "tool_run",
        json!({"project":p,"tool":"artifact.read","arguments":{"artifact":read["artifact"]["id"]}}),
    )
    .await
    .unwrap()
    .error_for_status()
    .unwrap()
    .json::<Value>()
    .await
    .unwrap();
    assert!(result.to_string().contains("ZnJvbSBmaXh0dXJl"));
    let mut constrained = definition(
        "GET",
        "read",
        json!({"query":{"type":"string","maxLength":3},"filter":{"type":"string"}}),
    );
    constrained["schema"]["required"] = json!(["query"]);
    let scoped = register(constrained)
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json::<Value>()
        .await
        .unwrap()["id"]
        .clone();
    assert!(
        run("web.fetch", &scoped, json!({"query":"ok"}), &Value::Null)
            .await
            .unwrap()
            .status()
            .is_success()
    );
    for invalid in [
        json!({}),
        json!({"query":"length"}),
        json!({"query":"ok","unknown":1}),
    ] {
        assert!(
            run("web.fetch", &scoped, invalid, &Value::Null)
                .await
                .unwrap()
                .status()
                .is_client_error()
        );
    }
    let write = register(definition(
        "POST",
        "write",
        json!({"amount":{"type":"integer"}}),
    ))
    .await
    .unwrap()
    .json::<Value>()
    .await
    .unwrap()["id"]
        .clone();
    let args = json!({"amount":1});
    assert!(
        run("connector.call", &write, args.clone(), &Value::Null)
            .await
            .unwrap()
            .status()
            .is_client_error()
    );
    let grant = api(
        "connector",
        json!({"op":"authorize","project":p,"id":write,"arguments":args}),
    )
    .await
    .unwrap()
    .error_for_status()
    .unwrap()
    .json::<Value>()
    .await
    .unwrap()["operation"]
        .clone();
    assert!(
        run("connector.call", &write, json!({"amount":2}), &grant)
            .await
            .unwrap()
            .status()
            .is_client_error()
    );
    let receipt = run("connector.call", &write, args.clone(), &grant)
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(receipt["invocation"], grant);
    assert!(
        run("connector.call", &write, args, &grant)
            .await
            .unwrap()
            .status()
            .is_client_error()
    );
    assert_eq!(writes.load(Ordering::Relaxed), 1);
    api("connector", json!({"op":"revoke","project":p,"id":id}))
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    assert!(
        run("web.fetch", &id, json!({}), &Value::Null)
            .await
            .unwrap()
            .status()
            .is_client_error()
    );
    fixture.abort();
}
