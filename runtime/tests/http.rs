use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader},
    path::Path,
    process::{Child, Command, Stdio},
};

struct Daemon(Child);
impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
#[tokio::test]
async fn loopback_api_checks_credentials_origin_and_host_before_project_access() {
    let root = tempfile::tempdir().unwrap();
    let working = root.path().join("project");
    std::fs::create_dir(&working).unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_fluxr-app-host"))
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../app"))
        .arg(root.path().join("db"))
        .arg("--headless")
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut daemon = Daemon(child);
    let mut line = String::new();
    BufReader::new(daemon.0.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let (origin, token) = line
        .trim()
        .strip_prefix("Open ")
        .unwrap()
        .split_once("/#")
        .unwrap();
    let c = reqwest::Client::new();
    let landing = c
        .get(origin)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let text = landing.text().await.unwrap();
    assert!(text.contains("POST /api/{operation}"));
    assert!(text.contains("Authorization: Bearer"));
    assert!(!text.contains(token));
    for path in ["/assets/client.js", "/assets/style.css"] {
        assert_eq!(
            c.get(format!("{origin}{path}"))
                .send()
                .await
                .unwrap()
                .status(),
            404
        );
    }
    let url = format!("{origin}/api/open_project");
    let request = || c.post(&url).json(&json!({"root":working}));
    assert_eq!(request().send().await.unwrap().status(), 403);
    assert_eq!(
        request()
            .bearer_auth(token)
            .header("origin", "https://example.invalid")
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    assert_eq!(
        request()
            .bearer_auth(token)
            .header("host", "invalid.local")
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    let response: Value = request()
        .bearer_auth(token)
        .header("origin", origin)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let poll = |after: i64| {
        c.post(format!("{origin}/api/events"))
            .bearer_auth(token)
            .json(&json!({"project":response["id"],"after":after}))
    };
    let first: Value = poll(0).send().await.unwrap().json().await.unwrap();
    assert_eq!(first["events"][0]["kind"], "Project opened");
    let second: Value = poll(first["cursor"].as_i64().unwrap())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(second["events"], json!([]));
    let private = c
        .post(format!("{origin}/api/poll"))
        .bearer_auth(token)
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(private.status(), 404);
}
