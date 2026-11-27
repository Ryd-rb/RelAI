use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader},
    path::Path,
    process::{Child, Command, Stdio},
};
struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn first_line(mut child: Child) -> Result<(Process, String)> {
    let mut line = String::new();
    BufReader::new(child.stdout.take().context("Missing pipe")?).read_line(&mut line)?;
    ensure!(!line.is_empty(), "Fixture exited before ready");
    Ok((Process(child), line.trim().into()))
}
fn git(path: &Path, args: &[&str]) -> Result<String> {
    let o = Command::new("git")
        .current_dir(path)
        .args([
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "core.hooksPath=disabled-hooks",
        ])
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env(
            "GIT_CONFIG_GLOBAL",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
        )
        .output()?;
    ensure!(
        o.status.success(),
        "Git fixture error: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    Ok(String::from_utf8(o.stdout)?.trim().into())
}
struct Api {
    client: reqwest::Client,
    url: String,
    token: String,
}
impl Api {
    async fn call(&self, name: &str, body: Value) -> Result<Value> {
        let r = self
            .client
            .post(format!("{}/api/{name}", self.url))
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await?;
        let status = r.status();
        let content = r.text().await?;
        ensure!(status.is_success(), "{name} failed: {content}");
        Ok(serde_json::from_str(&content)?)
    }
    async fn operation(
        &self,
        p: &Value,
        id: &Value,
        name: &str,
        expected: Value,
        target: &str,
    ) -> Result<Value> {
        let grant=self.call("remote",json!({"op":"authorize","id":id,"project":p,"kind":name,"expected":expected,"target":target})).await?;
        self.call("tool_run",json!({"project":p,"tool":format!("git.{name}"),"arguments":{"operation":grant["operation"]}})).await
    }
}
#[tokio::test]
async fn authorized_https_push_and_fetch_interoperate_with_git_and_refuse_stale_or_divergent_tips()
-> Result<()> {
    let tmp = tempfile::tempdir()?;
    let server_dir = tmp.path().join("host");
    fs::create_dir(&server_dir)?;
    git(
        &server_dir,
        &["init", "--bare", "--initial-branch=main", "sample.git"],
    )?;
    git(
        &server_dir.join("sample.git"),
        &["config", "http.receivepack", "true"],
    )?;
    let producer = tmp.path().join("source");
    let consumer = tmp.path().join("consumer");
    for dir in [&producer, &consumer] {
        fs::create_dir(dir)?;
        git(dir, &["init", "--initial-branch=main"])?;
    }
    fs::write(producer.join("a"), "one")?;
    fs::create_dir(producer.join("skill"))?;
    fs::write(
        producer.join("skill/SKILL.md"),
        "---\nname: fixture\ndescription: Pinned test package\n---\nRead the receipt.",
    )?;
    git(&producer, &["add", "a", "skill"])?;
    git(&producer, &["commit", "-m", "one"])?;
    let one = git(&producer, &["rev-parse", "HEAD"])?;
    fs::write(producer.join("a"), "two")?;
    git(&producer, &["add", "a"])?;
    git(&producer, &["commit", "-m", "two"])?;
    let two = git(&producer, &["rev-parse", "HEAD"])?;
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()])?;
    let pem = tmp.path().join("cert.pem");
    let key = tmp.path().join("key.pem");
    fs::write(&pem, cert.cert.pem())?;
    fs::write(&key, cert.key_pair.serialize_pem())?;
    let (_server, port) = first_line(
        Command::new("python")
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/git_server.py"))
            .args([&server_dir, &pem, &key])
            .stdout(Stdio::piped())
            .spawn()?,
    )?;
    let (_daemon, line) = first_line(
        Command::new(env!("CARGO_BIN_EXE_fluxr-app-host"))
            .env("RELAI_GIT_CA_FILE", &pem)
            .env_remove("RELAI_CODEX_URL")
            .env_remove("RELAI_CODEX_TOKEN_FILE")
            .env_remove("RELAI_QWEN_CHAT_URL")
            .env_remove("ANTHROPIC_API_KEY")
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../app"))
            .arg(tmp.path().join("state"))
            .arg("--headless")
            .stdout(Stdio::piped())
            .spawn()?,
    )?;
    let (url, token) = line
        .strip_prefix("Open ")
        .context("Missing ready marker")?
        .split_once("/#")
        .context("Missing session")?;
    let api = Api {
        client: reqwest::Client::new(),
        url: url.into(),
        token: token.into(),
    };
    let register = |p: &Value| {
        api.call("remote",json!({"project":p,"op":"register","url":format!("https://localhost:{port}/sample.git"),"branch":"refs/heads/main"}))
    };
    let p = api.call("open_project", json!({"root":producer})).await?["id"].clone();
    let remote = register(&p).await?["id"].clone();
    let first = api
        .operation(&p, &remote, "push", Value::Null, &one)
        .await?;
    assert_eq!(first["state"], "pushed", "{first}");
    let attribution = api
        .call(
            "tool_run",
            json!({"project":p,"tool":"git.blame","arguments":{"object":two,"path":"a"}}),
        )
        .await?;
    assert_eq!(attribution["value"]["hunks"][0]["commit"], two);
    let forged = api
        .call(
            "tool_run",
            json!({"project":p,"tool":"git.push","arguments":{"operation":"unapproved"}}),
        )
        .await;
    assert!(forged.is_err());
    let stale = api
        .operation(&p, &remote, "push", Value::Null, &two)
        .await?;
    assert_eq!(stale["state"], "conflict", "{stale}");
    let next = api.operation(&p, &remote, "push", json!(one), &two).await?;
    assert_eq!(next["state"], "pushed", "{next}");
    let divergent = api.operation(&p, &remote, "push", json!(two), &one).await?;
    assert_eq!(divergent["state"], "rejected", "{divergent}");
    // The foreground scheduler starts a push directly from the checked current job.
    // Its durable transfer is already sending before the HTTPS write begins.
    let chat_remote = api.call("remote", json!({"project":p,"op":"register","url":format!("https://localhost:{port}/sample.git"),"branch":"refs/heads/chat"})).await?["id"].clone();
    let mut db = rusqlite::Connection::open(tmp.path().join("state/state.sqlite3"))?;
    db.busy_timeout(std::time::Duration::from_secs(5))?;
    for (job, expected, target, outcome) in [
        ("chat-first", Value::Null, one.as_str(), "pushed"),
        ("chat-next", json!(one), two.as_str(), "pushed"),
        ("chat-divergent", json!(two), one.as_str(), "rejected"),
    ] {
        let transaction = db.transaction()?;
        transaction.execute(
            "INSERT INTO requests(id,project,intent) VALUES(?,?,'Push this commit')",
            rusqlite::params![job, p.as_str().unwrap()],
        )?;
        transaction.execute(
            "INSERT INTO submissions VALUES(?,?,?,?,'hash','{}')",
            rusqlite::params![
                format!("submission-{job}"),
                p.as_str().unwrap(),
                job,
                format!("key-{job}")
            ],
        )?;
        transaction.execute("INSERT INTO jobs(id,project,request,revision,session,role,kind,input,state) SELECT ?,id,?,1,discriminator,'discriminator','tool',?,'queued' FROM projects WHERE id=?", rusqlite::params![job,job,json!({"operation":"tool_run","arguments":{"project":p,"tool":"git.push","arguments":{"remote":chat_remote,"expected":expected,"target":target}}}).to_string(),p.as_str().unwrap()])?;
        transaction.commit()?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        let receipt = loop {
            let (state, receipt): (String, Option<String>) =
                db.query_row("SELECT state,receipt FROM jobs WHERE id=?", [job], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })?;
            if state == "accepted" {
                break serde_json::from_str::<Value>(
                    &receipt.context("Missing foreground receipt")?,
                )?;
            }
            ensure!(
                std::time::Instant::now() < deadline,
                "Foreground push stalled in {state}"
            );
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        };
        assert_eq!(receipt["state"], outcome, "{receipt}");
        assert!(receipt["operation"].is_string());
        let (count, pending): (i64,i64) = db.query_row("SELECT count(*),sum(t.state IN ('authorized','sending')) FROM chat_git_transfers c JOIN git_transfers t ON t.id=c.transfer WHERE c.job=?", [job], |r| Ok((r.get(0)?,r.get(1)?)))?;
        assert_eq!((count, pending), (1, 0));
        db.execute("UPDATE requests SET state='completed' WHERE id=?", [job])?;
        db.execute(
            "UPDATE jobs SET state='cancelled' WHERE request=? AND kind!='tool'",
            [job],
        )?;
    }
    let uncertain: String = db.query_row(
        "SELECT transfer FROM chat_git_transfers WHERE job='chat-next'",
        [],
        |r| r.get(0),
    )?;
    db.execute(
        r#"UPDATE git_transfers SET state='uncertain',receipt='{"state":"uncertain"}' WHERE id=?"#,
        [&uncertain],
    )?;
    let confirmed = api
        .call(
            "tool_run",
            json!({"project":p,"tool":"git.push_status","arguments":{"operation":uncertain}}),
        )
        .await?;
    assert_eq!(confirmed["state"], "completed", "{confirmed}");
    assert_eq!(
        db.query_row(
            "SELECT state FROM git_transfers WHERE id=?",
            [&uncertain],
            |r| r.get::<_, String>(0)
        )?,
        "completed"
    );
    assert_eq!(
        git(
            &server_dir.join("sample.git"),
            &["rev-parse", "refs/heads/chat"]
        )?,
        two
    );
    assert_eq!(
        git(
            &server_dir.join("sample.git"),
            &["rev-parse", "refs/heads/main"]
        )?,
        two
    );
    let q = api.call("open_project", json!({"root":consumer})).await?["id"].clone();
    let receiver = register(&q).await?["id"].clone();
    let refs = api
        .call(
            "tool_run",
            json!({"project":q,"tool":"git.remote_refs","arguments":{"remote":receiver}}),
        )
        .await?;
    assert_eq!(refs["target"], two);
    let fetch = api
        .operation(&q, &receiver, "fetch", Value::Null, &two)
        .await?;
    assert_eq!(fetch["state"], "fetched", "{fetch}");
    let skill = api
        .call(
            "skill",
            json!({"project":q,"op":"install_git","transfer":fetch["operation"],"path":"skill"}),
        )
        .await?;
    let installed = api
        .call(
            "skill",
            json!({"project":q,"op":"inspect","id":skill["id"]}),
        )
        .await?;
    assert_eq!(installed["body"], "Read the receipt.");
    assert_eq!(installed["origins"][0]["commit"], two);
    let tracking = fetch["tracking"].as_str().unwrap();
    assert_eq!(git(&consumer, &["rev-parse", tracking])?, two);
    assert_eq!(git(&consumer, &["show", &format!("{tracking}:a")])?, "two");
    assert_eq!(
        git(
            &server_dir.join("sample.git"),
            &["rev-parse", "refs/heads/main"]
        )?,
        two
    );
    Ok(())
}
