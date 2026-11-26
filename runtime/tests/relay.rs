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
