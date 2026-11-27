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
    time::Duration,
};
struct Daemon(Child);
impl Daemon {
    fn start(state: &Path, provider: Option<&str>) -> (Self, String, String) {
        let mut command = Command::new(env!("CARGO_BIN_EXE_fluxr-app-host"));
        command
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../app"))
            .arg(state)
            .arg("--headless")
            .env_remove("RELAI_CONNECTOR_OFFLINE_FIXTURE_TOKEN")
            .stdout(Stdio::piped());
        if let Some(provider) = provider {
            command
                .env("RELAI_QWEN_CHAT_URL", provider)
                .env_remove("RELAI_QWEN_KEY");
        }
        let mut daemon = Self(command.spawn().unwrap());
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
        (daemon, url.to_owned(), token.to_owned())
    }
}
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
            "/denied",
            get(|| async { (axum::http::StatusCode::NOT_FOUND, "private response") })
                .post(|| async { (axum::http::StatusCode::CONFLICT, "private response") }),
        )
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
    let (_daemon, url, token) = Daemon::start(&temp.path().join("state"), None);
    let http = reqwest::Client::new();
    let api = |name: &str, body: Value| {
        http.post(format!("{url}/api/{name}"))
            .bearer_auth(&token)
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
    for bad in [
        json!({"url":"http://example.invalid/read"}),
        json!({"url":"https://user@example.invalid/read"}),
        json!({"url":"https://example.invalid/read#fragment"}),
        json!({"idempotency_header":"Authorization"}),
    ] {
        let mut def = definition("GET", "read", json!({}));
        def.as_object_mut()
            .unwrap()
            .extend(bad.as_object().unwrap().clone());
        assert!(register(def).await.unwrap().status().is_client_error());
    }
    let get = register(definition("GET", "denied", json!({})))
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    let failure = run("web.fetch", &get["id"], json!({}), &Value::Null)
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(failure["state"], "http_error");
    assert_eq!(failure["status"], 404);
    assert!(failure.get("artifact").is_none() && failure.get("response").is_none());
    assert!(!failure.to_string().contains("private response"));

    for (missing_credential, expected) in [(true, "not_sent"), (false, "uncertain")] {
        let mut def = definition("POST", "denied", json!({}));
        if missing_credential {
            def["credential"] = json!("RELAI_CONNECTOR_OFFLINE_FIXTURE_TOKEN");
        } else {
            def.as_object_mut().unwrap().remove("idempotency_header");
        }
        let connector = register(def)
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json::<Value>()
            .await
            .unwrap();
        let id = &connector["id"];
        let grant = api(
            "connector",
            json!({"op":"authorize","project":p,"id":id,"arguments":{}}),
        )
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
        let operation = &grant["operation"];
        let first = run("connector.call", id, json!({}), operation)
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json::<Value>()
            .await
            .unwrap();
        assert_eq!(first["state"], expected);
        assert_eq!(&first["invocation"], operation);
        assert!(first.get("artifact").is_none() && first.get("response").is_none());
        assert!(!first.to_string().contains("private response"));
        let retry = run("connector.call", id, json!({}), operation)
            .await
            .unwrap();
        if missing_credential {
            let retry = retry
                .error_for_status()
                .unwrap()
                .json::<Value>()
                .await
                .unwrap();
            assert_eq!(retry["state"], "not_sent");
            assert_eq!(&retry["invocation"], operation);
        } else {
            assert!(retry.status().is_client_error());
        }
    }
    fixture.abort();
}

async fn connector_chat_model(Json(request): Json<Value>) -> Json<Value> {
    let frame: Value = serde_json::from_str(
        request["messages"][1]["content"][0]["text"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    let envelope = if frame["role"] == "workflow_improver" {
        json!({"version":1,"next":"complete_when","actions":[{"type":"RESULT","summary":"Checked the workflow.","result_kind":"report","findings":"No reusable change.","evidence":[],"outcome":"no_change"}]})
    } else {
        let delivery = &frame["delivery"];
        let prior = &delivery["original_assignment"];
        let receipt = &delivery["tool_receipt"];
        if let Some(prompt) = delivery["text"].as_str().or_else(|| prior["text"].as_str()) {
            let words: Vec<_> = prompt.split_whitespace().collect();
            let amount: u32 = words[6].parse().unwrap();
            assert!(
                frame["project_tools"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|tool| tool["tool"] == "connector.write")
            );
            let args = json!({"connector":words[3],"arguments":{"amount":amount}});
            let mut write = json!({"version":1,"next":"wait_for","actions":[{"type":"RUN_TOOL","summary":"Send the requested amount.","tool":"connector.write","arguments_json":args.to_string()}]});
            if delivery["text"].is_string() {
                write["dispositions"] = json!([{"start":0,"end":prompt.len(),"attachment":null,"disposition":"Send the requested connector write."}]);
            } else {
                assert_eq!(
                    receipt["state"],
                    if amount == 1 {
                        "completed"
                    } else {
                        "uncertain"
                    }
                );
            }
            write
        } else {
            assert_eq!(receipt, &prior["tool_receipt"]);
            let findings = if receipt["state"] == "completed" {
                "Connector write sent."
            } else {
                "Connector write outcome could not be confirmed."
            };
            json!({"version":1,"next":"complete_when","actions":[{"type":"RESULT","summary":"Report the connector result.","result_kind":"final","findings":findings,"evidence":[],"outcome":null}]})
        }
    };
    Json(json!({"choices":[{"finish_reason":"stop","message":{"content":envelope.to_string()}}]}))
}

#[tokio::test]
async fn chat_writes_to_the_registered_connector_and_reuses_each_outcome() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    let state = temp.path().join("state");
    std::fs::create_dir(&project).unwrap();
    let successful_posts = Arc::new(AtomicUsize::new(0));
    let uncertain_posts = Arc::new(AtomicUsize::new(0));
    let posts = (successful_posts.clone(), uncertain_posts.clone());
    let fixture = Router::new()
        .route("/model", post(connector_chat_model))
        .route(
            "/write",
            post(move |headers: HeaderMap, Json(body): Json<Value>| {
                let posts = posts.clone();
                async move {
                    assert!(headers["idempotency-key"].to_str().unwrap().len() >= 32);
                    match body["amount"].as_u64() {
                        Some(1) => {
                            posts.0.fetch_add(1, Ordering::Relaxed);
                            axum::http::StatusCode::OK
                        }
                        Some(2) => {
                            posts.1.fetch_add(1, Ordering::Relaxed);
                            axum::http::StatusCode::CONFLICT
                        }
                        _ => panic!("Unexpected connector arguments"),
                    }
                }
            }),
        );
    let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = socket.local_addr().unwrap();
    let fixture = tokio::spawn(async move { axum::serve(socket, fixture).await.unwrap() });
    let (_daemon, url, token) = Daemon::start(&state, Some(&format!("http://{addr}/model")));
    let http = reqwest::Client::new();
    let api = |name: &str, body: Value| {
        let request = http
            .post(format!("{url}/api/{name}"))
            .bearer_auth(&token)
            .json(&body);
        async move {
            request
                .send()
                .await
                .unwrap()
                .error_for_status()
                .unwrap()
                .json::<Value>()
                .await
                .unwrap()
        }
    };
    let p = api("open_project", json!({"root":project})).await["id"].clone();
    for role in ["discriminator", "workflow_improver"] {
        api("save_model", json!({"project":p,"role":role,"provider":"qwen","model":"connector-fixture","effort":"low"})).await;
    }
    let connector = api("connector", json!({"project":p,"op":"register","definition":{
        "name":"fixture","method":"POST","url":format!("http://{addr}/write"),
        "idempotency_header":"Idempotency-Key",
        "schema":{"type":"object","required":["amount"],"additionalProperties":false,"properties":{"amount":{"type":"integer"}}}
    }})).await["id"].as_str().unwrap().to_owned();
    let resource = api(
        "tool_run",
        json!({"project":p,"tool":"connector.find","arguments":{}}),
    )
    .await;
    let discovered = &resource["value"][0];
    assert_eq!(discovered["id"], connector);
    assert_eq!(discovered["method"], "POST");
    assert!(discovered["url"].is_null() && discovered["credential"].is_null());
    let db = rusqlite::Connection::open(state.join("state.sqlite3")).unwrap();
    for (amount, expected) in [(1, "completed"), (2, "uncertain")] {
        let text = format!("Send through connector {connector} with amount {amount}");
        let request = api(
            "submit",
            json!({"project":p,"key":format!("write-{amount}"),"text":text}),
        )
        .await;
        let request = request["request"].as_str().unwrap();
        tokio::time::timeout(Duration::from_secs(15), async {
            while db
                .query_row(
                    "SELECT count(*) FROM finals WHERE request=? AND released=1",
                    [request],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap()
                == 0
            {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("Waiting for final connector result");
        let (count, stored): (i64, String) = db.query_row(
            "SELECT count(*),min(e.state) FROM chat_connector_writes w JOIN network_effects e ON e.id=w.invocation WHERE w.request=?",
            [request], |r| Ok((r.get(0)?,r.get(1)?)),
        ).unwrap();
        assert_eq!((count, stored.as_str()), (1, expected));
    }
    assert_eq!(successful_posts.load(Ordering::Relaxed), 1);
    assert_eq!(uncertain_posts.load(Ordering::Relaxed), 1);
    fixture.abort();
}
