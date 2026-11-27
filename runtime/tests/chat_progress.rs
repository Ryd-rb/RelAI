use fluxr_app_host::App;
use serde_json::{Value, json};
use std::{fs, path::Path};

fn bundle() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../app")
}

fn status(app: &mut App, project: &Value) -> Option<String> {
    let view = app.invoke("dashboard", json!({"project":project})).unwrap();
    let frame = app
        .invoke("desktop_frame", json!({"view":view,"state":{}}))
        .unwrap();
    let statuses: Vec<_> = frame["controls"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["id"].as_str().is_some_and(|id| id.ends_with("/status")))
        .collect();
    assert!(statuses.len() <= 1);
    statuses.first().map(|c| {
        assert_eq!(c["kind"], "label");
        assert_eq!(c["role"], "muted");
        assert_eq!(c["label"], c["value"]);
        c["value"].as_str().unwrap().to_owned()
    })
}

#[test]
fn delivery_controls_enforce_model_budgets_without_blocking_local_tools() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    fs::create_dir(&root).unwrap();
    let data = temp.path().join("state");
    let mut app = App::open(&bundle(), &data).unwrap();
    let project = app.invoke("open_project", json!({"root":root})).unwrap()["id"].clone();
    app.invoke(
        "submit",
        json!({"project":project,"key":"budget","text":"Request","attachments":[]}),
    )
    .unwrap();
    let sql = rusqlite::Connection::open(data.join("state.sqlite3")).unwrap();
    let job: String = sql
        .query_row("SELECT id FROM jobs", [], |row| row.get(0))
        .unwrap();
    sql.execute("UPDATE jobs SET state='held' WHERE id=?", [&job])
        .unwrap();
    for index in 0..3 {
        sql.execute(
            "INSERT INTO attempts(id,job,state) VALUES(?,?,'rejected')",
            (format!("attempt-{index}"), &job),
        )
        .unwrap();
    }
    let control = |app: &mut App, op: &str, job: &str| {
        app.invoke("control", json!({"project":project,"op":op,"job":job}))
    };
    let controls = |app: &mut App| {
        let view = app.invoke("dashboard", json!({"project":project})).unwrap();
        app.invoke("desktop_frame", json!({"view":view,"state":{}}))
            .unwrap()["controls"]
            .as_array()
            .unwrap()
            .clone()
    };
    let current = controls(&mut app);
    assert!(!current.iter().any(|c| c["label"] == "Retry delivery"));
    assert!(
        current
            .iter()
            .any(|c| c["label"] == "Start a fresh attempt")
    );
    assert!(control(&mut app, "retry", &job).is_err());
    let replacement = control(&mut app, "replace", &job).unwrap();
    let replacement = replacement.as_str().unwrap();
    for index in 3..64 {
        sql.execute(
            "INSERT INTO attempts(id,job,state) VALUES(?,?,'rejected')",
            (format!("attempt-{index}"), &job),
        )
        .unwrap();
    }
    sql.execute("UPDATE requests SET revision=2", []).unwrap();
    sql.execute(
        "UPDATE jobs SET revision=2,state='held' WHERE id=?",
        [replacement],
    )
    .unwrap();
    assert!(control(&mut app, "retry", replacement).is_err());
    assert!(control(&mut app, "replace", replacement).is_err());
    let exhausted = controls(&mut app);
    assert!(
        !exhausted
            .iter()
            .any(|c| c["label"] == "Retry delivery" || c["label"] == "Start a fresh attempt")
    );
    assert!(
        exhausted
            .iter()
            .any(|c| c["label"] == "New topic" && c["event"]["patch"]["fresh"] == true)
    );
    let fresh = app
        .invoke(
            "submit",
            json!({"project":project,"key":"new-budget","text":"Another topic","attachments":[]}),
        )
        .unwrap();
    let next = app.invoke("poll", Value::Null).unwrap();
    assert_eq!(next["local"], "seal");
    assert_ne!(fresh["request"], Value::Null);
    assert_ne!(next["job"], replacement);
    sql.execute("UPDATE jobs SET kind='tool' WHERE id=?", [replacement])
        .unwrap();
    control(&mut app, "retry", replacement).unwrap();
}

#[test]
fn chat_describes_real_model_preparation_and_a_pending_file_read() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("readme.txt"), "Local project evidence").unwrap();
    let data = temp.path().join("state");
    let mut app = App::open(&bundle(), &data).unwrap();
    let project = app.invoke("open_project", json!({"root":root})).unwrap()["id"].clone();
    app.invoke(
        "submit",
        json!({"project":project,"key":"read","text":"Read the project","attachments":[]}),
    )
    .unwrap();
    assert_eq!(
        status(&mut app, &project).as_deref(),
        Some("Preparing the model request…")
    );
    let seal = app.invoke("poll", Value::Null).unwrap();
    assert_eq!(seal["local"], "seal");
    assert_eq!(
        status(&mut app, &project).as_deref(),
        Some("Preparing the model request…")
    );
    app.invoke("seal", seal).unwrap();
    assert_eq!(
        status(&mut app, &project).as_deref(),
        Some("Preparing the model request…")
    );
    let model = app.invoke("poll", Value::Null).unwrap();
    assert!(!model["attempt"].is_null());
    assert_eq!(
        status(&mut app, &project).as_deref(),
        Some("Waiting for the model response…")
    );
    let reply = json!({"version":1,"actions":[{"type":"RUN_TOOL","summary":"Read the selected project file.","tool":"fs.read","arguments_json":json!({"path":"readme.txt"}).to_string()}],"next":"wait_for","dispositions":[{"start":0,"end":16,"attachment":null,"disposition":"Read project evidence."}]});
    app.invoke("receive", json!({"job":model["job"],"attempt":model["attempt"],"response":{"message":{"params":{"item":{"text":reply.to_string()}}}}})).unwrap();
    assert_eq!(
        status(&mut app, &project).as_deref(),
        Some("Preparing to read files…")
    );
    let local = app.invoke("poll", Value::Null).unwrap();
    assert_eq!(local["local"], "tool_run");
    assert_eq!(
        status(&mut app, &project).as_deref(),
        Some("Reading the selected files…")
    );
    let receipt = app.invoke("tool_run", local["arguments"].clone()).unwrap();
    assert_eq!(receipt["value"]["text"], "Local project evidence");
    app.invoke("tool_done", json!({"job":local["job"],"result":receipt}))
        .unwrap();
    assert_eq!(
        status(&mut app, &project).as_deref(),
        Some("Preparing the model request…")
    );
}

#[test]
fn chat_uses_safe_operation_and_failure_copy_without_echoing_private_details() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    fs::create_dir(&root).unwrap();
    let data = temp.path().join("state");
    let mut app = App::open(&bundle(), &data).unwrap();
    let project = app.invoke("open_project", json!({"root":root})).unwrap()["id"].clone();
    app.invoke("submit", json!({"project":project,"key":"fixtures","text":"Help with this project","attachments":[]})).unwrap();
    let sql = rusqlite::Connection::open(data.join("state.sqlite3")).unwrap();
    let private = "C:\\private\\07fddca1-1593-4300-86f8-bd39d22cf1d2\\secret.txt API_CODE_X";
    for (tool, expected) in [
        ("project.lookup", "Looking up project information…"),
        ("fs.search", "Searching the project…"),
        ("fs.list", "Inspecting the project…"),
        ("web.fetch", "Looking up online information…"),
        ("verify.test", "Checking the project…"),
        (private, "Running the next project step…"),
    ] {
        let input = json!({"operation":"tool_run","arguments":{"tool":tool,"arguments":{"path":private}},"summary":private});
        sql.execute(
            "UPDATE jobs SET kind='tool',state='executing',input=?",
            [input.to_string()],
        )
        .unwrap();
        assert_eq!(status(&mut app, &project).as_deref(), Some(expected));
    }
    for (state, reason, expected) in [
        (
            "held",
            private,
            "A project operation stopped. Retry delivery to continue.",
        ),
        (
            "held",
            "Selected source access changed",
            "File access changed. Select the file again to continue.",
        ),
        (
            "held",
            "Interrupted deterministic job",
            "A project operation was interrupted. Retry delivery to continue.",
        ),
        (
            "uncertain",
            private,
            "This request was interrupted. Review before retrying.",
        ),
    ] {
        sql.execute("UPDATE jobs SET state=?,reason=?", [state, reason])
            .unwrap();
        assert_eq!(status(&mut app, &project).as_deref(), Some(expected));
    }
    sql.execute(
        "UPDATE jobs SET kind='intake',state='held',reason=?",
        [private],
    )
    .unwrap();
    assert_eq!(
        status(&mut app, &project).as_deref(),
        Some(
            "The model request could not be prepared. No model request was sent for this job. Retry delivery to continue."
        )
    );
    sql.execute(
        "INSERT INTO attempts(id,job,state) SELECT 'interrupted-attempt',id,'uncertain' FROM jobs",
        [],
    )
    .unwrap();
    sql.execute("UPDATE jobs SET attempt='interrupted-attempt'", [])
        .unwrap();
    assert_eq!(
        status(&mut app, &project).as_deref(),
        Some("This request was interrupted. Review before retrying.")
    );
    let interrupted: String = sql
        .query_row("SELECT id FROM jobs", [], |r| r.get(0))
        .unwrap();
    app.invoke(
        "control",
        json!({"project":project,"op":"replace","job":interrupted}),
    )
    .unwrap();
    assert_eq!(
        status(&mut app, &project).as_deref(),
        Some("Preparing the model request…")
    );
}

#[test]
fn a_received_response_stays_distinct_from_an_interrupted_provider_call_after_reopening() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    fs::create_dir(&root).unwrap();
    let data = temp.path().join("state");
    let mut app = App::open(&bundle(), &data).unwrap();
    let project = app.invoke("open_project", json!({"root":root})).unwrap()["id"].clone();
    app.invoke(
        "submit",
        json!({"project":project,"key":"oversize","text":"Read this.","attachments":[]}),
    )
    .unwrap();
    let seal = app.invoke("poll", Value::Null).unwrap();
    app.invoke("seal", seal).unwrap();
    let model = app.invoke("poll", Value::Null).unwrap();
    app.invoke("hold",json!({"job":model["job"],"attempt":model["attempt"],"body_received":true,"error":"require: Action limit exceeded"})).unwrap();
    let sql = rusqlite::Connection::open(data.join("state.sqlite3")).unwrap();
    let attempt = model["attempt"].as_str().unwrap();
    let attempt_state = || {
        sql.query_row("SELECT state FROM attempts WHERE id=?", [attempt], |r| {
            r.get::<_, String>(0)
        })
        .unwrap()
    };
    let check_rejected = |app: &mut App| {
        assert!(status(app, &project).unwrap().contains("received"));
        let view = app.invoke("dashboard", json!({"project":project})).unwrap();
        let frame = app
            .invoke("desktop_frame", json!({"view":view,"state":{}}))
            .unwrap();
        assert!(frame["controls"].as_array().unwrap().iter().all(|c| {
            !c["value"]
                .as_str()
                .unwrap_or("")
                .contains("Delivery could not be confirmed")
        }));
    };
    assert_eq!(attempt_state(), "rejected");
    check_rejected(&mut app);
    drop(app);
    sql.execute(
        "UPDATE attempts SET state='uncertain' WHERE id=?",
        [attempt],
    )
    .unwrap();
    let mut reopened = App::open(&bundle(), &data).unwrap();
    assert_eq!(attempt_state(), "rejected");
    check_rejected(&mut reopened);
    sql.execute(
        "UPDATE jobs SET reason='Connection lost' WHERE id=?",
        [model["job"].as_str().unwrap()],
    )
    .unwrap();
    sql.execute(
        "UPDATE attempts SET state='uncertain' WHERE id=?",
        [attempt],
    )
    .unwrap();
    drop(reopened);
    let mut reopened = App::open(&bundle(), &data).unwrap();
    assert_eq!(attempt_state(), "uncertain");
    assert!(
        status(&mut reopened, &project)
            .unwrap()
            .contains("interrupted")
    );
}

#[test]
fn chat_prioritizes_actionable_questions_and_approval_and_scopes_progress() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    let other = temp.path().join("other");
    fs::create_dir(&root).unwrap();
    fs::create_dir(&other).unwrap();
    let data = temp.path().join("state");
    let mut app = App::open(&bundle(), &data).unwrap();
    let project = app.invoke("open_project", json!({"root":root})).unwrap()["id"].clone();
    let foreign = app.invoke("open_project", json!({"root":other})).unwrap()["id"].clone();
    let sub = app
        .invoke(
            "submit",
            json!({"project":project,"key":"scope","text":"Review this project","attachments":[]}),
        )
        .unwrap();
    let sql = rusqlite::Connection::open(data.join("state.sqlite3")).unwrap();
    sql.execute("UPDATE jobs SET state='held',reason='private failure'", [])
        .unwrap();
    sql.execute("INSERT INTO questions(id,request,revision,session,role,body) SELECT 'question',request,revision,session,role,'{}' FROM jobs", []).unwrap();
    assert_eq!(status(&mut app, &project), None);
    sql.execute("UPDATE questions SET state='answered'", [])
        .unwrap();
    sql.execute("INSERT INTO actions(id,job,kind,payload,state) SELECT 'answer',id,'RESULT','{}','awaiting_acceptance' FROM jobs", []).unwrap();
    assert_eq!(status(&mut app, &project), None);
    sql.execute("UPDATE actions SET state='accepted'", [])
        .unwrap();
    sql.execute("UPDATE jobs SET state='accepted'", []).unwrap();
    sql.execute("INSERT INTO jobs(id,project,request,revision,session,role,kind,input,state) SELECT 'stale',project,request,0,'old','planner','tool','{}','held' FROM jobs LIMIT 1", []).unwrap();
    sql.execute("INSERT INTO jobs(id,project,request,revision,session,role,kind,input,state) VALUES('foreign',?,?,1,'foreign','planner','tool','{}','held')", [foreign.as_str().unwrap(), sub["request"].as_str().unwrap()]).unwrap();
    assert_eq!(
        status(&mut app, &project).as_deref(),
        Some("Waiting for a required step to finish.")
    );
    assert_eq!(status(&mut app, &foreign), None);
    sql.execute(
        "UPDATE requests SET state='completed' WHERE id=?",
        [sub["request"].as_str().unwrap()],
    )
    .unwrap();
    assert_eq!(status(&mut app, &project), None);
}
