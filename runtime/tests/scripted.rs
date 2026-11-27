//! Complete offline relay acceptance script. All state transitions use checked public
//! exports; the read-only database connection only audits durable invariants.
use base64::{Engine, engine::general_purpose::STANDARD};
use fluxr_app_host::App;
use rusqlite::{Connection, OpenFlags};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
};

fn bundle() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../app")
}
fn next_model(app: &mut App) -> Value {
    for _ in 0..20 {
        let job = app.invoke("poll", Value::Null).unwrap();
        assert!(!job.is_null(), "Expected the next scripted model turn");
        if job["local"] == "seal" {
            app.invoke("seal", job).unwrap();
        } else {
            assert!(
                job["local"].is_null(),
                "Unexpected local operation: {job:#}"
            );
            assert!(job["attempt"].is_string());
            return job;
        }
    }
    panic!("Turn did not become dispatchable within the local bound")
}
fn frame(data: &Path, job: &Value) -> Value {
    serde_json::from_slice(
        &fs::read(
            data.join("turns")
                .join(job["job"].as_str().unwrap())
                .join("input.json"),
        )
        .unwrap(),
    )
    .unwrap()
}
fn details(app: &mut App, project: &Value, job: &Value) -> Value {
    app.invoke("job_details", json!({"project":project,"id":job["job"]}))
        .unwrap()
}
fn reply(app: &mut App, job: &Value, body: &Value) -> anyhow::Result<Value> {
    // This is the same provider response envelope used by the Codex adapter.
    app.invoke("receive",json!({"job":job["job"],"attempt":job["attempt"],"response":{"message":{"params":{"item":{"text":body.to_string()}}}}}))
}
fn envelope(actions: Value) -> Value {
    json!({"version":1,"actions":actions,"next":"complete_when"})
}
fn result(kind: &str, findings: &str, evidence: Value) -> Value {
    json!({"type":"RESULT","summary":"Read the verified result.","result_kind":kind,"findings":findings,"evidence":evidence,"outcome":null})
}
fn task(name: &str, role: &str, attachment: &Value) -> Value {
    json!({"type":"TASK","summary":name,"role":role,"assignment":name,"deliverable":"One evidence report about the submitted text","criterion":"The report agrees with the exact submitted fixture","constraints":["Read only the supplied fixture; do not change files"],"skills":[],"targets":[],"attachments":[attachment],"prerequisites":[],"sources":[]})
}
fn events(app: &mut App, project: &Value) -> Vec<Value> {
    let page = app
        .invoke("events", json!({"project":project,"after":0}))
        .unwrap();
    let items = page["events"].as_array().unwrap();
    assert!(
        items.len() < 100,
        "The acceptance fixture must fit one bounded event page"
    );
    items.clone()
}
fn evidence_action(app: &mut App, project: &Value, findings: &str) -> Value {
    let found = events(app, project)
        .into_iter()
        .filter(|e| e["kind"] == "RESULT" && e["payload"]["payload"]["findings"] == findings)
        .collect::<Vec<_>>();
    assert_eq!(
        found.len(),
        1,
        "Duplicate or missing evidence delivery: {found:#?}"
    );
    found[0]["payload"]["action"].clone()
}
fn count(db: &Connection, query: &str) -> i64 {
    db.query_row(query, [], |row| row.get(0)).unwrap()
}

#[test]
fn permanent_relay_script_finishes_once_and_recovers_the_same_improver() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    let data = temp.path().join("private");
    fs::create_dir(&root).unwrap();
    let unselected = "UNSELECTED_PROJECT_FILE_CONTENT";
    fs::write(root.join("README.md"), unselected).unwrap();
    fs::write(
        root.join("Agents.md"),
        "Inspect the supplied text. Keep project files unchanged.\n",
    )
    .unwrap();
    let fixture = b"The release color is blue. The release count is two.\n";
    let prompt = "Check the color and count independently, then combine the accepted reports.";
    let mut app = App::open(&bundle(), &data).unwrap();
    let opened = app.invoke("open_project", json!({"root":root})).unwrap();
    let project = opened["id"].clone();
    let discriminator = opened["discriminator"].clone();
    let attachment=app.invoke("capture",json!({"project":project,"name":"release.txt","mime":"text/plain","base64":STANDARD.encode(fixture)})).unwrap();
    let submission = json!({"project":project,"key":"scripted-permanent-relay","text":prompt,"attachments":[attachment["id"]]});
    let submitted = app.invoke("submit", submission.clone()).unwrap();
    assert_eq!(
        app.invoke("submit", submission.clone()).unwrap()["id"],
        submitted["id"]
    );
    let mut conflicting = submission.clone();
    conflicting["text"] = json!("Changed content under the same key");
    assert!(app.invoke("submit", conflicting).is_err());
    let db =
        Connection::open_with_flags(data.join("state.sqlite3"), OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();

    let intake = next_model(&mut app);
    let intake_details = details(&mut app, &project, &intake);
    let input = frame(&data, &intake);
    let project_tools = input["project_tools"].as_array().unwrap();
    for name in ["fs.list", "fs.read", "fs.search"] {
        let entry = project_tools
            .iter()
            .find(|entry| entry["tool"] == name)
            .unwrap();
        assert!(entry["arguments"]["path"].is_string());
        assert!(entry["description"].is_string());
    }
    let lookup = project_tools
        .iter()
        .find(|entry| entry["tool"] == "tool.find")
        .unwrap();
    assert!(lookup["arguments"]["query"].is_string());
    let (patch_tool, read_tools) = project_tools.split_last().unwrap();
    assert_eq!(patch_tool["tool"], "patch.propose");
    assert!(patch_tool["arguments"]["files"].is_array());
    assert_eq!(read_tools.len(), 4);
    assert!(!intake["transport"].to_string().contains(unselected));
    assert_eq!(intake_details["role"], "discriminator");
    assert_eq!(intake_details["session"], discriminator);
    assert_eq!(
        input["project_instructions"][0]["text"],
        "Inspect the supplied text. Keep project files unchanged.\n"
    );
    assert!(
        intake["transport"]
            .to_string()
            .contains("release color is blue")
    );
    assert_eq!(
        fs::read(
            data.join("submissions")
                .join(submitted["id"].as_str().unwrap())
                .join("PROMPT.txt")
        )
        .unwrap(),
        prompt.as_bytes()
    );
    let mut dispatch = envelope(json!([
        task(
            "Inspect the release color",
            "investigator",
            &attachment["id"]
        ),
        task("Inspect the release count", "reviewer", &attachment["id"])
    ]));
    dispatch["dispositions"] = json!([{"start":0,"end":prompt.len(),"attachment":null,"disposition":"Run two independent checks and review accepted evidence"},{"start":0,"end":0,"attachment":attachment["id"],"disposition":"Use this exact fixture for both assignments"}]);
    reply(&mut app, &intake, &dispatch).unwrap();
    reply(&mut app, &intake, &dispatch).unwrap();
    assert_eq!(
        count(&db, "SELECT count(*) FROM branches"),
        2,
        "Duplicate intake must not create siblings again"
    );

    let mut reports = Vec::new();
    for _ in 0..2 {
        let worker = next_model(&mut app);
        let worker_details = details(&mut app, &project, &worker);
        let input = frame(&data, &worker);
        assert_eq!(input["project_tools"].as_array().unwrap(), read_tools);
        assert_ne!(worker_details["session"], discriminator);
        assert_eq!(input["delivery"]["attachments"], json!([attachment["id"]]));
        assert_eq!(
            input["delivery"]["criterion"],
            "The report agrees with the exact submitted fixture"
        );
        let findings = if worker_details["role"] == "investigator" {
            "The release color is blue."
        } else {
            assert_eq!(worker_details["role"], "reviewer");
            "The release count is two."
        };
        let response = envelope(json!([result("report", findings, json!([]))]));
        reply(&mut app, &worker, &response).unwrap();
        reply(&mut app, &worker, &response).unwrap();
        assert_eq!(details(&mut app, &project, &worker)["state"], "accepted");
        reports.push(evidence_action(&mut app, &project, findings));
        if reports.len() == 1 {
            assert_eq!(count(&db, "SELECT count(*) FROM reviews"), 0);
        }
    }
    assert_ne!(reports[0], reports[1]);
    assert_eq!(
        count(
            &db,
            "SELECT count(*) FROM actions WHERE kind='RESULT' AND state='evidence'"
        ),
        2
    );
    assert_eq!(
        count(
            &db,
            "SELECT count(*) FROM branches WHERE accepted IS NOT NULL"
        ),
        2,
        "Each branch retains its scoped report for independent review"
    );
    assert_eq!(count(&db, "SELECT count(*) FROM reviews"), 1);
    drop(app);
    let mut app = App::open(&bundle(), &data).unwrap();
    assert_eq!(
        app.invoke("open_project", json!({"root":root})).unwrap(),
        opened
    );
    let review = next_model(&mut app);
    let review_details = details(&mut app, &project, &review);
    let input = frame(&data, &review);
    let branches = input["delivery"]["branches"].as_array().unwrap();
    assert_eq!(review_details["kind"], "review");
    assert_eq!(review_details["session"], discriminator);
    assert_eq!(branches.len(), 2);
    for branch in branches {
        assert_eq!(branch["acceptance"]["verifier"], "relay");
        assert_eq!(branch["acceptance"]["evidence"], branch["report"]);
        assert_eq!(branch["acceptance"]["revision"], review_details["revision"]);
    }
    let final_evidence = branches
        .iter()
        .map(|b| b["report"].clone())
        .collect::<Vec<_>>();
    let mut finish = envelope(json!([result(
        "final",
        "Both independently accepted checks agree: blue, two.",
        json!(final_evidence)
    )]));
    finish["review_id"] = input["delivery"]["review_id"].clone();
    finish["decision"] = json!("finish");
    finish["achieved"] = json!(
        branches
            .iter()
            .map(|b| b["criterion_id"].clone())
            .collect::<Vec<_>>()
    );
    finish["remaining"] = json!([]);
    let mut missing_evidence = finish.clone();
    missing_evidence["actions"][0]["evidence"] = json!([]);
    assert!(reply(&mut app, &review, &missing_evidence).is_err());
    assert_eq!(count(&db, "SELECT count(*) FROM finals"), 0);
    reply(&mut app, &review, &finish).unwrap();
    reply(&mut app, &review, &finish).unwrap();
    assert_eq!(count(&db, "SELECT count(*) FROM finals"), 1);
    assert_eq!(
        count(&db, "SELECT count(*) FROM requests WHERE state='completed'"),
        1
    );
    let improver_record: (String, String) = db
        .query_row("SELECT session,job FROM improvements", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    assert_eq!(
        count(&db, "SELECT count(*) FROM events WHERE kind='Final result'"),
        0,
        "Final release must wait for its verified publication"
    );
    drop(app);
    let mut app = App::open(&bundle(), &data).unwrap();
    let improver = next_model(&mut app);
    let info = details(&mut app, &project, &improver);
    assert_eq!(info["role"], "workflow_improver");
    assert!(frame(&data, &improver).get("project_tools").is_none());
    assert_eq!(info["session"], improver_record.0);
    assert_eq!(improver["job"], improver_record.1);
    let published = events(&mut app, &project)
        .into_iter()
        .filter(|e| e["kind"] == "Final result")
        .collect::<Vec<_>>();
    assert_eq!(published.len(), 1);
    assert_eq!(
        published[0]["payload"]["result"]["findings"],
        finish["actions"][0]["findings"]
    );
    let final_path: String = db
        .query_row("SELECT path FROM finals", [], |r| r.get(0))
        .unwrap();
    let published_final: Value =
        serde_json::from_slice(&fs::read(data.join(final_path)).unwrap()).unwrap();
    assert_eq!(published_final["payload"], finish["actions"][0]);
    assert_eq!(published_final["request"], submitted["request"]);
    assert_eq!(published_final["revision"], review_details["revision"]);
    assert!(published_final["id"].is_string());
    assert_eq!(
        frame(&data, &improver)["delivery"]["result"],
        finish["actions"][0]
    );
    drop(app);
    let mut app = App::open(&bundle(), &data).unwrap();
    assert!(
        app.invoke("poll", Value::Null).unwrap().is_null(),
        "Restart cannot resend an uncertain provider attempt"
    );
    app.invoke(
        "control",
        json!({"project":project,"op":"retry","job":improver["job"]}),
    )
    .unwrap();
    let retry = next_model(&mut app);
    assert_eq!(retry["job"], improver["job"]);
    assert_ne!(retry["attempt"], improver["attempt"]);
    assert_eq!(retry["transport"], improver["transport"]);
    assert_eq!(
        details(&mut app, &project, &retry)["session"],
        improver_record.0
    );
    let mut report = result(
        "report",
        "No reusable improvement is justified by this small fixture.",
        json!([]),
    );
    report["outcome"] = json!("no_change");
    let improvement = envelope(json!([report]));
    assert!(
        reply(&mut app, &improver, &improvement).is_err(),
        "The old attempt cannot impersonate the new attempt"
    );
    reply(&mut app, &retry, &improvement).unwrap();
    reply(&mut app, &retry, &improvement).unwrap();
    assert!(app.invoke("poll", Value::Null).unwrap().is_null());
    assert_eq!(count(&db, "SELECT count(*) FROM improvements"), 1);
    assert_eq!(
        count(
            &db,
            "SELECT count(*) FROM jobs WHERE role='workflow_improver'"
        ),
        1
    );
    assert_eq!(
        count(
            &db,
            "SELECT count(*) FROM attempts a JOIN jobs j ON j.id=a.job WHERE j.role='workflow_improver'"
        ),
        2
    );
    assert_eq!(
        count(&db, "SELECT count(*) FROM events WHERE kind='Final result'"),
        1
    );
    assert_eq!(
        count(&db, "SELECT count(*) FROM actions WHERE state='held'"),
        0
    );
    assert_eq!(
        fs::read(root.join("Agents.md")).unwrap(),
        b"Inspect the supplied text. Keep project files unchanged.\n"
    );
    let second=app.invoke("submit",json!({"project":project,"key":"next-permanent-relay-request","text":"A new independent request","attachments":[]})).unwrap();
    assert_ne!(second["request"], submitted["request"]);
    let next = next_model(&mut app);
    assert_eq!(
        details(&mut app, &project, &next)["session"],
        discriminator,
        "The permanent project relay remains ready for new requests"
    );
}
