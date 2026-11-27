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
fn obstructed_retained_documents_hold_only_their_delivery_and_can_recover() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("state");
    let mut app = App::open(&bundle(), &data).unwrap();
    let submit = |app: &mut App, name: &str| {
        let root = temp.path().join(name);
        fs::create_dir(&root).unwrap();
        let project = app.invoke("open_project", json!({"root":root})).unwrap()["id"].clone();
        let sub = app
            .invoke("submit", json!({"project":project,"key":name,"text":name}))
            .unwrap();
        (project, sub)
    };
    let (project, sub) = submit(&mut app, "blocked");
    let sealing = app.invoke("poll", Value::Null).unwrap();
    assert_eq!(sealing["local"], "seal");
    let job = sealing["job"].as_str().unwrap();
    app.invoke("seal", sealing.clone()).unwrap();
    let parent = data.join("turns");
    fs::create_dir_all(&parent).unwrap();
    let obstruction = parent.join(job);
    fs::write(&obstruction, "obstruction").unwrap();
    let (_, other) = submit(&mut app, "healthy");
    assert!(app.invoke("poll", Value::Null).unwrap().is_null());
    let sql = rusqlite::Connection::open(data.join("state.sqlite3")).unwrap();
    let state: (String, i64) = sql.query_row(
        "SELECT state,(SELECT count(*) FROM attempts a WHERE a.job=j.id) FROM jobs j WHERE id=?",
        [job],
        |r| Ok((r.get(0)?, r.get(1)?)),
    ).unwrap();
    assert_eq!(state, ("held".into(), 0));
    let path = format!("turns/{job}/input.json");
    let failed = || {
        sql.query_row(
        "SELECT f.verified,f.content,p.failures FROM files f JOIN publication_failures p USING(path,digest) WHERE f.path=?",
        [&path],
        |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?)),
    ).unwrap()
    };
    let before = failed();
    assert_eq!(before.0, 0);
    let healthy = next(&mut app);
    assert_ne!(healthy["job"], sealing["job"]);
    let healthy_request: String = sql
        .query_row(
            "SELECT request FROM jobs WHERE id=?",
            [healthy["job"].as_str().unwrap()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(other["request"], healthy_request);
    assert_eq!(failed(), before);
    assert_eq!(fs::read(&obstruction).unwrap(), b"obstruction");
    fs::remove_file(obstruction).unwrap();
    app.invoke("control", json!({"project":project,"op":"retry","job":job}))
        .unwrap();
    let retried = next(&mut app);
    assert_eq!(retried["job"], sealing["job"]);
    assert_eq!(
        sql.query_row("SELECT verified FROM files WHERE path=?", [&path], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        sql.query_row(
            "SELECT count(*) FROM publication_failures WHERE path=?",
            [&path],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(fs::read_to_string(data.join(path)).unwrap(), before.1);
    assert_eq!(
        fs::read_to_string(
            data.join("submissions")
                .join(sub["id"].as_str().unwrap())
                .join("PROMPT.txt")
        )
        .unwrap(),
        "blocked"
    );
}

#[test]
fn updating_a_request_while_reading_the_project_can_reach_acceptance() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("notes.txt"), "Earlier context").unwrap();
    let data = temp.path().join("state");
    let mut app = App::open(&bundle(), &data).unwrap();
    let project = app.invoke("open_project", json!({"root":root})).unwrap()["id"].clone();
    let request = app
        .invoke(
            "submit",
            json!({"project":project,"key":"initial","text":"Read notes."}),
        )
        .unwrap()["request"]
        .clone();
    let job = next(&mut app);
    let read = json!({"type":"RUN_TOOL","tool":"fs.read","summary":"Read notes.","arguments_json":json!({"path":"notes.txt"}).to_string()});
    let mut response = intake(json!([read]), "Read notes.".len());
    response["next"] = json!("wait_for");
    deliver(&mut app, &job, response).unwrap();
    let read = app.invoke("poll", Value::Null).unwrap();
    assert_eq!(read["local"], "tool_run");
    let receipt = app.invoke("tool_run", read["arguments"].clone()).unwrap();
    app.invoke(
        "submit",
        json!({"project":project,"key":"update","text":"Say hello.","steer":request}),
    )
    .unwrap();
    app.invoke(
        "tool_done",
        json!({"job":read["job"],"state":read["state"],"result":receipt}),
    )
    .unwrap();
    let updated = next(&mut app);
    deliver(
        &mut app,
        &updated,
        intake(json!([result("final", "Hello.")]), "Say hello.".len()),
    )
    .unwrap();
    let view = app.invoke("dashboard", json!({"project":project})).unwrap();
    let transcript = view["components"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["type"] == "transcript")
        .unwrap();
    let answer = &transcript["destination"]["bind"]["revise"];
    assert!(answer.is_string());
    app.invoke(
        "control",
        json!({"project":project,"op":"accept","action":answer}),
    )
    .unwrap();
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
    let controls = frame["controls"].as_array().unwrap();
    for (role, text) in [("outgoing", "Colour?"), ("incoming", "Blue.")] {
        let message = controls
            .iter()
            .find(|c| c["role"] == role && c["value"] == text)
            .unwrap();
        assert_eq!(message["kind"], "detail");
        assert_ne!(message["area"], "composer");
        assert_ne!(message["area"], "header");
        assert_ne!(message["side"], true);
    }
    assert!(controls.iter().any(|c| c["id"] == frame["composer"]
        && c["kind"] == "multiline"
        && c["area"] == "composer"));
    let accept = controls
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
    assert!(
        !controls
            .iter()
            .any(|c| c["label"] == "Accept answer" || c["label"] == "Follow up")
    );
    assert!(controls.iter().any(|c| c["label"] == "New topic"));
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
        let conversation = |app: &mut App| {
            let view = app.invoke("dashboard", json!({"project":p})).unwrap();
            app.invoke("desktop_frame", json!({"view":view,"state":{}}))
                .unwrap()
        };
        let before = conversation(&mut a);
        let message = before["controls"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["role"] == "incoming" && c["value"] == msg)
            .unwrap();
        a.invoke(
            "control",
            json!({"project":p,"op":"accept","action":action}),
        )
        .unwrap();
        let after = conversation(&mut a);
        let controls = after["controls"].as_array().unwrap();
        assert!(controls.iter().any(|c| c == message));
        assert!(
            !controls
                .iter()
                .any(|c| c["event"]["spec"]["fixed"]["action"] == action)
        );
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
fn a_selected_reply_quotes_only_its_scoped_persisted_request_and_question() {
    for recipient_role in ["discriminator", "planner"] {
        let temp = tempfile::tempdir().unwrap();
        let files = temp.path().join("repo");
        let other = temp.path().join("other");
        fs::create_dir(&files).unwrap();
        fs::create_dir(&other).unwrap();
        fs::write(files.join("project.txt"), "Project source evidence.").unwrap();
        let data = temp.path().join("db");
        let mut app = App::open(&bundle(), &data).unwrap();
        let project = app.invoke("open_project", json!({"root":files})).unwrap()["id"].clone();
        let foreign = app.invoke("open_project", json!({"root":other})).unwrap()["id"].clone();
        let original = "Improve this project.\nKeep the current behaviour. 🙂";
        let submitted = app
            .invoke(
                "submit",
                json!({"project":project,"key":"original","text":original,"attachments":[]}),
            )
            .unwrap();
        let first = next(&mut app);
        let asker = if recipient_role == "planner" {
            deliver(
                &mut app,
                &first,
                intake(
                    json!([task("Plan improvements", json!([]))]),
                    original.len(),
                ),
            )
            .unwrap();
            next(&mut app)
        } else {
            first.clone()
        };
        let question = "Do you have a handoff or a source snapshot?\nWhich one?";
        let ask = json!({"type":"ASK","summary":"Check the available context.","question":question,"work":"Plan improvements","choices":["A handoff","A source snapshot","Neither"]});
        let mut response = if recipient_role == "discriminator" {
            intake(json!([ask]), original.len())
        } else {
            envelope(json!([ask]))
        };
        response["next"] = json!("wait_for");
        deliver(&mut app, &asker, response).unwrap();
        let sql = rusqlite::Connection::open(data.join("state.sqlite3")).unwrap();
        let (q, recipient): (String, String) = sql
            .query_row("SELECT id,session FROM questions", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        if recipient_role == "planner" {
            let review = next(&mut app);
            let id: String = sql
                .query_row(
                    "SELECT id FROM reviews WHERE job=?",
                    [review["job"].as_str().unwrap()],
                    |r| r.get(0),
                )
                .unwrap();
            let mut wait = envelope(json!([]));
            wait["review_id"] = json!(id);
            wait["decision"] = json!("wait");
            wait["achieved"] = json!([]);
            wait["remaining"] = json!([q]);
            deliver(&mut app, &review, wait).unwrap();
        }
        // Existing databases have no context snapshot on their questions or jobs.
        // Unrelated fields on those records must never become reply history.
        sql.execute(
            "UPDATE questions SET body=json_set(body,'$.internal_note','private-question-field')",
            [],
        )
        .unwrap();
        sql.execute("UPDATE jobs SET input=json_set(input,'$.old_grant','private-prior-job-field') WHERE id=?", [asker["job"].as_str().unwrap()]).unwrap();
        drop(app);
        let mut app = App::open(&bundle(), &data).unwrap();
        let answer = "Neither.\nPlease use this project. 🙂";
        let mut args =
            json!({"project":foreign,"key":"reply","text":answer,"attachments":[],"reply":q});
        assert!(app.invoke("submit", args.clone()).is_err());
        args["project"] = project.clone();
        let reply = app.invoke("submit", args).unwrap();
        assert_eq!(reply["request"], submitted["request"]);
        let sealing = app.invoke("poll", Value::Null).unwrap();
        assert_eq!(sealing["local"], "seal");
        // A queued reply cannot quote another request, project, or revision.
        sql.execute("INSERT INTO requests(id,project,intent) VALUES('unrelated-request',?,'unrelated request secret')", [project.as_str().unwrap()]).unwrap();
        sql.execute(
            "UPDATE questions SET request='unrelated-request' WHERE id=?",
            [&q],
        )
        .unwrap();
        assert!(app.invoke("seal", sealing.clone()).is_err());
        sql.execute(
            "UPDATE questions SET request=? WHERE id=?",
            [submitted["request"].as_str().unwrap(), &q],
        )
        .unwrap();
        sql.execute("UPDATE questions SET revision=2 WHERE id=?", [&q])
            .unwrap();
        assert!(app.invoke("seal", sealing.clone()).is_err());
        sql.execute("UPDATE questions SET revision=1 WHERE id=?", [&q])
            .unwrap();
        sql.execute(
            "UPDATE requests SET project=? WHERE id=?",
            [
                foreign.as_str().unwrap(),
                submitted["request"].as_str().unwrap(),
            ],
        )
        .unwrap();
        assert!(app.invoke("seal", sealing.clone()).is_err());
        sql.execute(
            "UPDATE requests SET project=? WHERE id=?",
            [
                project.as_str().unwrap(),
                submitted["request"].as_str().unwrap(),
            ],
        )
        .unwrap();
        app.invoke("seal", sealing).unwrap();
        let intake_job = next(&mut app);
        let sealed = |job: &Value| -> Value {
            serde_json::from_slice(
                &fs::read(
                    data.join("turns")
                        .join(job["job"].as_str().unwrap())
                        .join("input.json"),
                )
                .unwrap(),
            )
            .unwrap()
        };
        let input = sealed(&intake_job);
        let expected = json!({"quoted_request":original,"question_id":q,"question":question,"choices":["A handoff","A source snapshot","Neither"],"recipient":recipient,"recipient_role":recipient_role,"request":submitted["request"],"revision":1});
        assert_eq!(input["delivery"]["reply_context"], expected);
        assert_eq!(input["delivery"]["text"], answer);
        assert_eq!(input["prompt_bytes"], answer.len());
        let raw: String = sql
            .query_row(
                "SELECT input FROM jobs WHERE id=?",
                [intake_job["job"].as_str().unwrap()],
                |r| r.get(0),
            )
            .unwrap();
        assert!(!raw.contains("reply_context"));
        assert!(!input.to_string().contains("private-question-field"));
        assert!(!input.to_string().contains("private-prior-job-field"));
        assert!(!input.to_string().contains("unrelated request secret"));
        let message = json!({"type":"MESSAGE","summary":"Deliver the selected reply.","destination":recipient,"question":q,"payload":answer,"revision":1,"attachments":[]});
        deliver(
            &mut app,
            &intake_job,
            intake(json!([message]), answer.len()),
        )
        .unwrap();
        let sealing = app.invoke("poll", Value::Null).unwrap();
        assert_eq!(sealing["local"], "seal");
        let message_job = sealing["job"].as_str().unwrap();
        sql.execute(
            "UPDATE jobs SET session='unrelated-recipient' WHERE id=?",
            [message_job],
        )
        .unwrap();
        assert!(app.invoke("seal", sealing.clone()).is_err());
        sql.execute(
            "UPDATE jobs SET session=? WHERE id=?",
            [&recipient, message_job],
        )
        .unwrap();
        app.invoke("seal", sealing).unwrap();
        let delivered = next(&mut app);
        let input = sealed(&delivered);
        assert_eq!(input["role"], recipient_role);
        assert_eq!(input["delivery"]["reply_context"], expected);
        assert_eq!(input["delivery"]["payload"], answer);
        assert!(!input.to_string().contains("private-question-field"));
        assert!(!input.to_string().contains("private-prior-job-field"));
        let recorded: String = sql
            .query_row(
                "SELECT content FROM submissions WHERE id=?",
                [reply["id"].as_str().unwrap()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&recorded).unwrap()["text"],
            answer
        );
        assert!(!recorded.contains("reply_context"));
        let read = json!({"type":"RUN_TOOL","summary":"Read the project source.","tool":"fs.read","arguments_json":json!({"path":"project.txt"}).to_string()});
        let mut response = envelope(json!([read]));
        response["next"] = json!("wait_for");
        deliver(&mut app, &delivered, response).unwrap();
        let local = app.invoke("poll", Value::Null).unwrap();
        assert_eq!(local["local"], "tool_run");
        let receipt = app.invoke("tool_run", local["arguments"].clone()).unwrap();
        assert_eq!(receipt["value"]["text"], "Project source evidence.");
        app.invoke("tool_done", json!({"job":local["job"],"result":receipt}))
            .unwrap();
        let continuation = next(&mut app);
        let input = sealed(&continuation);
        assert_eq!(
            input["delivery"]["original_assignment"]["reply_context"],
            expected
        );
        assert_eq!(input["delivery"]["original_assignment"]["payload"], answer);
        assert!(!input.to_string().contains("private-question-field"));
        assert!(!input.to_string().contains("private-prior-job-field"));
    }
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
            .any(|c| c["role"] == "incoming" && c["value"] == "Include examples?")
    );
    for answer in ["Yes", "No"] {
        assert!(
            controls
                .iter()
                .any(|c| c["label"] == answer && c["event"]["reply"] == q)
        );
    }
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
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM events e JOIN jobs j ON j.id=e.payload->>'$.job' AND j.project=e.project WHERE e.kind='Job held' AND e.payload->>'$.reason'='Selected source access changed' AND j.state='held'",
            [],
            |r| r.get::<_, i64>(0)
        ).unwrap(),
        2
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
