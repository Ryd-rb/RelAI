use fluxr_app_host::App;
use serde_json::{Value, json};
use std::{fs, path::Path};
fn bundle() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../app")
}
fn next(a: &mut App) -> Value {
    for _ in 0..20 {
        let job = a.invoke("poll", Value::Null).unwrap();
        if job.is_null() {
            continue;
        }
        if job["local"] == "seal" {
            a.invoke("seal", job).unwrap();
            continue;
        }
        return job;
    }
    panic!("Expected a provider job")
}
fn deliver(a: &mut App, j: &Value, e: Value) -> anyhow::Result<Value> {
    a.invoke("receive",json!({"job":j["job"],"attempt":j["attempt"],"response":{"message":{"params":{"item":{"text":e.to_string()}}}}}))
}
fn envelope(actions: Value) -> Value {
    json!({"version":1,"actions":actions,"next":"complete_when"})
}
fn result(kind: &str, msg: &str) -> Value {
    json!({"type":"RESULT","summary":"Read the result.","result_kind":kind,"findings":msg,"evidence":[],"outcome":null})
}
fn intake(actions: Value, end: usize) -> Value {
    let mut e = envelope(actions);
    e["dispositions"] =
        json!([{"start":0,"end":end,"attachment":null,"disposition":"Answer the request."}]);
    e
}
#[test]
fn submission_recovery_retries_and_single_improvement() {
    let t = tempfile::tempdir().unwrap();
    let work = t.path().join("project");
    let db = t.path().join("db");
    fs::create_dir(&work).unwrap();
    fs::write(work.join("Agents.md"), "Root instructions").unwrap();
    fs::write(work.join("CLAUDE.md"), "Second file").unwrap();
    let mut app = App::open(&bundle(), &db).unwrap();
    let p = app.invoke("open_project", json!({"root":work})).unwrap()["id"].clone();
    let file = app
        .invoke(
            "capture",
            json!({"project":p,"name":"note.txt","mime":"text/plain","base64":"Qmx1ZQ=="}),
        )
        .unwrap();
    let arg =
        json!({"project":p,"text":"Read note.","key":"submission-1","attachments":[file["id"]]});
    let sub = app.invoke("submit", arg.clone()).unwrap();
    let retry = app.invoke("submit", arg.clone()).unwrap();
    assert_eq!(sub["id"], retry["id"]);
    let mut conflict = arg;
    conflict["text"] = json!("Different");
    assert!(app.invoke("submit", conflict).is_err());
    let item = next(&mut app);
    let turn = db
        .join("turns")
        .join(item["job"].as_str().unwrap())
        .join("input.json");
    let sealed: Value = serde_json::from_slice(&fs::read(turn).unwrap()).unwrap();
    assert_eq!(
        sealed.as_object().unwrap().keys().next().unwrap(),
        "project_instructions"
    );
    assert_eq!(
        sealed["project_instructions"][0]["text"],
        "Root instructions"
    );
    assert_eq!(sealed["project_instructions"][1]["text"], "Second file");
    assert!(item["transport"].to_string().contains("Blue"));
    assert_eq!(item["transport"]["destination"], "codex");
    drop(app);
    fs::write(work.join("Agents.md"), "Edited").unwrap();
    let prompt = db
        .join("submissions")
        .join(sub["id"].as_str().unwrap())
        .join("PROMPT.txt");
    fs::write(&prompt, "damage").unwrap();
    let mut app = App::open(&bundle(), &db).unwrap();
    assert!(app.invoke("poll", Value::Null).unwrap().is_null());
    assert_eq!(fs::read(prompt).unwrap(), b"Read note.");
    app.invoke(
        "control",
        json!({"project":p,"op":"retry","job":item["job"]}),
    )
    .unwrap();
    let retried = next(&mut app);
    assert_ne!(retried["attempt"], item["attempt"]);
    assert_eq!(retried["transport"], item["transport"]);
    deliver(
        &mut app,
        &retried,
        intake(json!([result("final", "Blue")]), 10),
    )
    .unwrap();
    let sql = rusqlite::Connection::open(db.join("state.sqlite3")).unwrap();
    let action: String = sql
        .query_row(
            "SELECT id FROM actions WHERE state='awaiting_acceptance'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    app.invoke(
        "control",
        json!({"project":p,"op":"accept","action":action}),
    )
    .unwrap();
    let improvement = next(&mut app);
    let mut report = result("report", "No reusable improvement.");
    report["outcome"] = json!("no_change");
    deliver(&mut app, &improvement, envelope(json!([report]))).unwrap();
    assert!(app.invoke("poll", Value::Null).unwrap().is_null());
    assert_eq!(
        1,
        sql.query_row("SELECT count(*) FROM improvements", [], |r| r
            .get::<_, i64>(0))
            .unwrap()
    );
    assert_eq!(
        1,
        sql.query_row(
            "SELECT count(*) FROM events WHERE kind='Final result'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap()
    );
}

#[test]
fn a_chat_follow_up_uses_only_the_selected_exchange_and_preserves_the_new_prompt() {
    let temp = tempfile::tempdir().unwrap();
    let folder = temp.path().join("first");
    let other = temp.path().join("second");
    fs::create_dir(&folder).unwrap();
    fs::create_dir(&other).unwrap();
    let data = temp.path().join("state");
    let mut app = App::open(&bundle(), &data).unwrap();
    let project = app.invoke("open_project", json!({"root":folder})).unwrap()["id"].clone();
    let second = app.invoke("open_project", json!({"root":other})).unwrap()["id"].clone();
    app.invoke(
        "submit",
        json!({"project":project,"text":"Colour?","key":"first","attachments":[]}),
    )
    .unwrap();
    let job = next(&mut app);
    deliver(&mut app, &job, intake(json!([result("final", "Blue.")]), 7)).unwrap();
    let render = |app: &mut App| {
        let view = app.invoke("dashboard", json!({"project":project})).unwrap();
        app.invoke("desktop_frame", json!({"view":view,"state":{}}))
            .unwrap()
    };
    let frame = render(&mut app);
    let accept = frame["controls"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["label"] == "Accept answer")
        .unwrap();
    let mut args = accept["event"]["spec"]["fixed"].clone();
    let action = args["action"].clone();
    args["project"] = project.clone();
    app.invoke("control", args).unwrap();
    let improvement = next(&mut app);
    let mut report = result("report", "internal workflow note");
    report["outcome"] = json!("no_change");
    deliver(&mut app, &improvement, envelope(json!([report]))).unwrap();
    drop(app);
    let mut app = App::open(&bundle(), &data).unwrap();
    let frame = render(&mut app);
    let controls = frame["controls"].as_array().unwrap();
    assert_eq!(
        controls
            .iter()
            .filter(|c| c["role"] == "incoming" && c["value"] == "Blue.")
            .count(),
        1
    );
    assert!(!frame.to_string().contains("internal workflow note"));
    assert!(!controls.iter().any(|c| c["label"] == "Accept answer"));
    let send = controls.iter().find(|c| c["label"] == "Send").unwrap();
    assert_eq!(
        send["event"]["spec"]["destination"]["bind"]["follow"],
        action
    );
    let text = "Why that one?\n🙂";
    let mut args =
        json!({"project":second,"text":text,"follow":action,"key":"follow","attachments":[]});
    assert!(app.invoke("submit", args.clone()).is_err());
    args["project"] = project;
    let submitted = app.invoke("submit", args).unwrap();
    let job = next(&mut app);
    let sealed = fs::read_to_string(
        data.join("turns")
            .join(job["job"].as_str().unwrap())
            .join("input.json"),
    )
    .unwrap();
    assert!(
        sealed.contains("previous_exchange")
            && sealed.contains("Colour?")
            && sealed.contains("Blue.")
    );
    assert!(!sealed.contains("internal workflow note"));
    assert_eq!(
        fs::read_to_string(
            data.join("submissions")
                .join(submitted["id"].as_str().unwrap())
                .join("PROMPT.txt")
        )
        .unwrap(),
        text
    );
}

#[test]
fn two_workers_require_separate_receipts_before_review() {
    let t = tempfile::tempdir().unwrap();
    let work = t.path().join("files");
    fs::create_dir(&work).unwrap();
    let db = t.path().join("db");
    let mut a = App::open(&bundle(), &db).unwrap();
    let p = a.invoke("open_project", json!({"root":work})).unwrap()["id"].clone();
    a.invoke(
        "submit",
        json!({"project":p,"key":"pair","text":"A and B","attachments":[]}),
    )
    .unwrap();
    let j = next(&mut a);
    let task = |name: &str| json!({"type":"TASK","summary":"Prepare the answer.","role":"planner","assignment":name,"deliverable":name,"criterion":name,"constraints":[],"skills":[],"targets":[],"attachments":[],"prerequisites":[],"sources":[]});
    deliver(&mut a, &j, intake(json!([task("A"), task("B")]), 7)).unwrap();
    let sql = rusqlite::Connection::open(db.join("state.sqlite3")).unwrap();
    for msg in ["First", "Second"] {
        let worker = next(&mut a);
        deliver(&mut a, &worker, envelope(json!([result("report", msg)]))).unwrap();
        let action: String = sql
            .query_row("SELECT id FROM actions WHERE state='evidence'", [], |r| {
                r.get(0)
            })
            .unwrap();
        a.invoke(
            "control",
            json!({"project":p,"op":"accept","action":action}),
        )
        .unwrap();
    }
    let review = next(&mut a);
    let rid: String = sql
        .query_row("SELECT id FROM reviews", [], |r| r.get(0))
        .unwrap();
    let mut e = envelope(json!([result("final", "Both reports accepted.")]));
    e["review_id"] = json!(rid);
    e["decision"] = json!("finish");
    let branch_ids = sql
        .prepare("SELECT id FROM branches ORDER BY id")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let reports = sql
        .prepare("SELECT report FROM branches ORDER BY id")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    e["achieved"] = json!(branch_ids);
    e["actions"][0]["evidence"] = json!(reports);
    e["remaining"] = json!([]);
    deliver(&mut a, &review, e).unwrap();
    assert_eq!(
        1,
        sql.query_row("SELECT count(*) FROM finals", [], |r| r.get::<_, i64>(0))
            .unwrap()
    );
}
fn task(name: &str, prerequisites: Value) -> Value {
    json!({"type":"TASK","summary":name,"role":"planner","assignment":name,"deliverable":name,"criterion":name,"constraints":[],"skills":[],"targets":[],"attachments":[],"prerequisites":prerequisites,"sources":[]})
}

#[test]
fn nested_review_settles_before_the_parent_review_is_released() {
    let temp = tempfile::tempdir().unwrap();
    let files = temp.path().join("repo");
    fs::create_dir(&files).unwrap();
    let data = temp.path().join("db");
    let mut app = App::open(&bundle(), &data).unwrap();
    let p = app.invoke("open_project", json!({"root":files})).unwrap()["id"].clone();
    app.invoke(
        "submit",
        json!({"project":p,"key":"nested","text":"Build","attachments":[]}),
    )
    .unwrap();
    let sql = rusqlite::Connection::open(data.join("state.sqlite3")).unwrap();
    let first = next(&mut app);
    deliver(
        &mut app,
        &first,
        intake(json!([task("Parent", json!([]))]), 5),
    )
    .unwrap();
    let parent = next(&mut app);
    deliver(
        &mut app,
        &parent,
        envelope(json!([
            task("Child", json!([])),
            result("report", "Parent evidence")
        ])),
    )
    .unwrap();
    let accept = |app: &mut App| {
        let a: String = sql
            .query_row("SELECT id FROM actions WHERE state='evidence'", [], |r| {
                r.get(0)
            })
            .unwrap();
        app.invoke("control", json!({"project":p,"op":"accept","action":a}))
            .unwrap();
    };
    accept(&mut app);
    assert_eq!(
        sql.query_row("SELECT count(*) FROM reviews", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    let child = next(&mut app);
    deliver(
        &mut app,
        &child,
        envelope(json!([result("report", "Child evidence")])),
    )
    .unwrap();
    accept(&mut app);
    let review = next(&mut app);
    let (review_id, root): (String, Option<String>) = sql
        .query_row(
            "SELECT rv.id,b.parent FROM reviews rv JOIN branches b USING(fan) WHERE rv.job=?",
            [review["job"].as_str().unwrap()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert!(root.is_some());
    let mut body = envelope(json!([]));
    body["decision"] = json!("settle");
    body["review_id"] = json!(review_id);
    body["achieved"] = json!([]);
    body["remaining"] = json!([]);
    deliver(&mut app, &review, body).unwrap();
    let ancestor = next(&mut app);
    let root: Option<String> = sql
        .query_row(
            "SELECT b.parent FROM reviews rv JOIN branches b USING(fan) WHERE rv.job=?",
            [ancestor["job"].as_str().unwrap()],
            |r| r.get(0),
        )
        .unwrap();
    assert!(root.is_none());
}

#[test]
fn a_named_question_survives_restart_and_can_be_reviewed_as_waiting() {
    let temp = tempfile::tempdir().unwrap();
    let files = temp.path().join("repo");
    fs::create_dir(&files).unwrap();
    let data = temp.path().join("db");
    let mut app = App::open(&bundle(), &data).unwrap();
    let p = app.invoke("open_project", json!({"root":files})).unwrap()["id"].clone();
    app.invoke(
        "submit",
        json!({"project":p,"key":"ask","text":"Check","attachments":[]}),
    )
    .unwrap();
    let first = next(&mut app);
    deliver(
        &mut app,
        &first,
        intake(json!([task("Check files", json!([]))]), 5),
    )
    .unwrap();
    let worker = next(&mut app);
    let mut ask = envelope(
        json!([{"type":"ASK","summary":"Choose scope.","question":"Include examples?","work":"Check links","choices":["Yes","No"]}]),
    );
    ask["next"] = json!("wait_for");
    deliver(&mut app, &worker, ask).unwrap();
    drop(app);
    let mut app = App::open(&bundle(), &data).unwrap();
    let review = next(&mut app);
    let sql = rusqlite::Connection::open(data.join("state.sqlite3")).unwrap();
    let (rid,q):(String,String)=sql.query_row("SELECT rv.id,q.id FROM reviews rv JOIN questions q ON rv.request=q.request WHERE rv.job=?",[review["job"].as_str().unwrap()],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    let view = app.invoke("dashboard", json!({"project":p})).unwrap();
    let frame = app
        .invoke("desktop_frame", json!({"view":view,"state":{}}))
        .unwrap();
    let controls = frame["controls"].as_array().unwrap();
    assert!(
        controls
            .iter()
            .any(|c| c["role"] == "incoming" && c["value"] == "Include examples?\n\nYes · No")
    );
    let send = controls.iter().find(|c| c["label"] == "Send").unwrap();
    assert_eq!(send["event"]["spec"]["destination"]["bind"]["reply"], q);
    let mut response = envelope(json!([]));
    response["decision"] = json!("wait");
    response["review_id"] = json!(rid);
    response["achieved"] = json!([]);
    response["remaining"] = json!(["missing-condition"]);
    assert!(deliver(&mut app, &review, response.clone()).is_err());
    response["remaining"] = json!([q]);
    deliver(&mut app, &review, response).unwrap();
    assert!(app.invoke("poll", Value::Null).unwrap().is_null());
}
#[test]
fn queued_external_read_rechecks_the_exact_human_source_selection() {
    let t = tempfile::tempdir().unwrap();
    let folder = t.path().join("project");
    fs::create_dir(&folder).unwrap();
    let external = t.path().join("outside.txt");
    fs::write(&external, "selected content").unwrap();
    let other = t.path().join("other.txt");
    fs::write(&other, "unselected content").unwrap();
    let mut app = App::open(&bundle(), &t.path().join("db")).unwrap();
    let p = app.invoke("open_project", json!({"root":folder})).unwrap()["id"].clone();
    let sub = app
        .invoke(
            "submit",
            json!({"project":p,"key":"live-source","text":"Read","attachments":[]}),
        )
        .unwrap();
    let selection = json!({"op":"allow","project":p,"request":sub["request"],"revision":1,"host_path":external});
    app.invoke("source_access", selection.clone()).unwrap();
    let job = next(&mut app);
    let read = |path: &std::path::Path| json!({"type":"RUN_TOOL","summary":"Read selected source.","tool":"fs.read","arguments_json":json!({"host_path":path}).to_string()});
    deliver(
        &mut app,
        &job,
        intake(json!([read(&external), read(&other)]), 4),
    )
    .unwrap();
    let db = rusqlite::Connection::open(t.path().join("db/state.sqlite3")).unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM actions WHERE state='held'", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    let mut revoke = selection.clone();
    revoke["op"] = json!("revoke");
    app.invoke("source_access", revoke).unwrap();
    assert!(app.invoke("poll", Value::Null).unwrap().is_null());
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM jobs WHERE kind='tool' AND state='held'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    let sub2 = app
        .invoke(
            "submit",
            json!({"project":p,"key":"read-permitted","text":"Read","attachments":[]}),
        )
        .unwrap();
    let mut permit = selection;
    permit["request"] = sub2["request"].clone();
    app.invoke("source_access", permit.clone()).unwrap();
    let job = next(&mut app);
    deliver(&mut app, &job, intake(json!([read(&external)]), 4)).unwrap();
    let local = app.invoke("poll", Value::Null).unwrap();
    assert_eq!(local["local"], "read_selected_file");
    let receipt = app
        .invoke("read_selected_file", local["arguments"].clone())
        .unwrap();
    assert_eq!(receipt["value"]["text"], "selected content");
    permit["op"] = json!("revoke");
    app.invoke("source_access", permit).unwrap();
    assert!(
        app.invoke("read_selected_file", local["arguments"].clone())
            .is_err()
    );
    app.invoke("tool_done", json!({"job":local["job"],"result":receipt}))
        .unwrap();
    assert_eq!(
        db.query_row(
            "SELECT state FROM jobs WHERE id=?",
            [local["job"].as_str().unwrap()],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "held"
    );
}
#[test]
fn a_direct_checked_tool_result_needs_no_followup_model_turn() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("db");
    let root = temp.path().join("repo");
    fs::create_dir(&root).unwrap();
    let mut app = App::open(&bundle(), &data).unwrap();
    let p = app.invoke("open_project", json!({"root":root})).unwrap()["id"].clone();
    app.invoke(
        "submit",
        json!({"project":p,"key":"direct-tool","text":"Count","attachments":[]}),
    )
    .unwrap();
    let first = next(&mut app);
    deliver(
        &mut app,
        &first,
        intake(json!([task("Count values", json!([]))]), 5),
    )
    .unwrap();
    let worker = next(&mut app);
    let action = json!({"type":"RUN_TOOL","summary":"Summarize values.","tool":"data.summarize","arguments_json":json!({"values":[2,4,6]}).to_string()});
    deliver(&mut app, &worker, envelope(json!([action]))).unwrap();
    let local = app.invoke("poll", Value::Null).unwrap();
    assert_eq!(local["local"], "tool_run");
    let receipt = app.invoke("tool_run", local["arguments"].clone()).unwrap();
    app.invoke("tool_done", json!({"job":local["job"],"result":receipt}))
        .unwrap();
    assert!(app.invoke("poll", Value::Null).unwrap().is_null());
    let db = rusqlite::Connection::open(data.join("state.sqlite3")).unwrap();
    assert_eq!(
        2,
        db.query_row("SELECT count(*) FROM attempts", [], |r| r.get::<_, i64>(0))
            .unwrap()
    );
    assert_eq!(
        1,
        db.query_row(
            "SELECT count(*) FROM branches WHERE report IS NOT NULL AND accepted IS NULL",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap()
    );
}
