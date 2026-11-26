use fluxr_app_host::App;
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
};

fn bundle() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../app")
}
struct Harness {
    temp: tempfile::TempDir,
    app: App,
    data: PathBuf,
    project: Value,
}
impl Harness {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("project");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("readme.txt"), "Alpha project evidence.").unwrap();
        let data = temp.path().join("state");
        let mut app = App::open(&bundle(), &data).unwrap();
        let project = app.invoke("open_project", json!({"root":root})).unwrap()["id"].clone();
        Self {
            temp,
            app,
            data,
            project,
        }
    }
    fn sql(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(self.data.join("state.sqlite3")).unwrap()
    }
    fn restart(self) -> Self {
        let Self {
            temp,
            app,
            data,
            project,
        } = self;
        drop(app);
        let app = App::open(&bundle(), &data).unwrap();
        Self {
            temp,
            app,
            data,
            project,
        }
    }
    fn submit(&mut self, extra: Value) -> Value {
        let mut args = json!({"project":self.project,"key":"first","text":"Read","attachments":[]});
        for (key, value) in extra.as_object().unwrap() {
            args[key] = value.clone();
        }
        self.app.invoke("submit", args).unwrap()
    }
    fn provider(&mut self) -> Value {
        for _ in 0..20 {
            let job = self.app.invoke("poll", Value::Null).unwrap();
            if job.is_null() {
                continue;
            }
            if job["local"] == "seal" {
                self.app.invoke("seal", job).unwrap();
                continue;
            }
            assert!(!job["attempt"].is_null(), "Expected model turn: {job}");
            return job;
        }
        panic!("Expected model turn")
    }
    fn respond(&mut self, job: &Value, actions: Value, next: &str) {
        let (kind, input): (String, String) = self
            .sql()
            .query_row(
                "SELECT kind,input FROM jobs WHERE id=?",
                [job["job"].as_str().unwrap()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        let mut response = json!({"version":1,"actions":actions,"next":next});
        if kind == "intake" {
            let input: Value = serde_json::from_str(&input).unwrap();
            response["dispositions"] = json!([{"start":0,"end":input["text"].as_str().unwrap().len(),"attachment":null,"disposition":"Read the project."}]);
        }
        self.app.invoke("receive", json!({"job":job["job"],"attempt":job["attempt"],"response":{"message":{"params":{"item":{"text":response.to_string()}}}}})).unwrap();
    }
    fn local(&mut self) -> Value {
        let job = self.app.invoke("poll", Value::Null).unwrap();
        assert!(
            job["local"] == "tool_run" || job["local"] == "read_selected_file",
            "Expected local read: {job}"
        );
        job
    }
    fn execute(&mut self, job: &Value) -> Value {
        self.app
            .invoke(job["local"].as_str().unwrap(), job["arguments"].clone())
            .unwrap()
    }
    fn done(&mut self, job: &Value, receipt: &Value) {
        self.app
            .invoke(
                "tool_done",
                json!({"job":job["job"],"state":job["state"],"result":receipt}),
            )
            .unwrap();
    }
    fn batch_count(&self, source: &Value) -> i64 {
        self.sql()
            .query_row(
                "SELECT count(*) FROM jobs WHERE input->>'$.tool_batch_source'=?",
                [source["job"].as_str().unwrap()],
                |r| r.get(0),
            )
            .unwrap()
    }
    fn delivery(&self, job: &Value) -> Value {
        let input: Value = serde_json::from_slice(
            &fs::read(
                self.data
                    .join("turns")
                    .join(job["job"].as_str().unwrap())
                    .join("input.json"),
            )
            .unwrap(),
        )
        .unwrap();
        input["delivery"].clone()
    }
}
fn tool(name: &str, args: Value) -> Value {
    json!({"type":"RUN_TOOL","summary":"Read project evidence.","tool":name,"arguments_json":args.to_string()})
}
fn reads() -> Value {
    json!([
        tool("fs.list", json!({"path":""})),
        tool("fs.read", json!({"path":"readme.txt"})),
        tool("fs.search", json!({"path":"readme.txt","query":"Alpha"})),
        tool("tool.find", json!({"query":"fs.read"}))
    ])
}

#[test]
#[cfg(windows)]
fn checking_two_effects_returns_one_answer_that_can_be_accepted() {
    let mut h = Harness::new();
    let mut ids = Vec::new();
    for name in ["first", "second"] {
        let path = h.temp.path().join("project").join(name);
        let proposal = h.app.invoke("effect_propose",json!({
            "project":h.project,"operation":"fs.mkdir","path":path,"expected":{"kind":"absent"}
        })).unwrap();
        ids.push(proposal["effect"].clone());
    }
    let request = h.submit(json!({"text":"Check both proposals."}));
    let source = h.provider();
    let checks: Vec<_> = ids
        .iter()
        .map(|id| tool("effect.status", json!({"effect":id})))
        .collect();
    h.respond(&source, json!(checks), "wait_for");
    for id in &ids {
        let local = h.local();
        let receipt = h.execute(&local);
        assert_eq!(receipt["id"], *id);
        assert_eq!(receipt["state"], "planned");
        h.done(&local, &receipt);
    }
    assert_eq!(h.batch_count(&source), 1);
    let resumed = h.provider();
    let delivery = h.delivery(&resumed);
    let receipts = delivery["tool_receipts"].as_array().unwrap();
    assert_eq!(receipts.len(), ids.len());
    for (entry, id) in receipts.iter().zip(ids) {
        assert_eq!(entry["tool"], "effect.status");
        assert_eq!(entry["receipt"]["id"], id);
    }
    h.respond(&resumed,json!([{"type":"RESULT","summary":"Checked both proposals.","findings":"Both proposals await approval.","result_kind":"final","evidence":[],"outcome":null}]),"complete_when");
    let answer: String = h
        .sql()
        .query_row(
            "SELECT id FROM actions WHERE state='awaiting_acceptance'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    h.app
        .invoke(
            "control",
            json!({"project":h.project,"op":"accept","action":answer}),
        )
        .unwrap();
    let state: String = h
        .sql()
        .query_row(
            "SELECT state FROM requests WHERE id=?",
            [request["request"].as_str().unwrap()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(state, "completed");
    for name in ["first", "second"] {
        assert!(!h.temp.path().join("project").join(name).exists());
    }
}

#[test]
fn sibling_reads_resume_once_with_all_receipts_and_sealed_reply_context() {
    let mut h = Harness::new();
    h.submit(json!({}));
    let first = h.provider();
    h.respond(&first, json!([{"type":"ASK","summary":"Check the context.","question":"Do you have a handoff?","work":"Read project evidence","choices":["Yes","No"]}]), "wait_for");
    let (question, recipient): (String, String) = h
        .sql()
        .query_row("SELECT id,session FROM questions", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    h.submit(json!({"key":"reply","reply":question,"text":"No"}));
    let reply = h.provider();
    h.respond(&reply, json!([{"type":"MESSAGE","summary":"Deliver the reply.","destination":recipient,"question":question,"payload":"No","revision":1,"attachments":[]}]), "wait_for");
    let source = h.provider();
    let assignment = h.delivery(&source);
    assert_eq!(
        assignment["reply_context"]["question"],
        "Do you have a handoff?"
    );
    h.respond(&source, reads(), "wait_for");
    let mut receipts = Vec::new();
    for index in 0..4 {
        let local = h.local();
        let receipt = h.execute(&local);
        h.done(&local, &receipt);
        h.done(
            &local,
            &json!({"error":"duplicate completion must not replace evidence"}),
        );
        let recorded: String = h
            .sql()
            .query_row(
                "SELECT receipt FROM jobs WHERE id=?",
                [local["job"].as_str().unwrap()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(serde_json::from_str::<Value>(&recorded).unwrap(), receipt);
        receipts.push((local, receipt));
        assert_eq!(h.batch_count(&source), if index == 3 { 1 } else { 0 });
        if index == 0 {
            h = h.restart();
        }
    }
    let continuation = h.provider();
    let delivery = h.delivery(&continuation);
    assert_eq!(delivery["tool_batch_source"], source["job"]);
    assert_eq!(delivery["original_assignment"], assignment);
    assert!(delivery["tool_receipt"].is_null());
    let entries = delivery["tool_receipts"].as_array().unwrap();
    assert_eq!(entries.len(), 4);
    for (entry, (local, receipt)) in entries.iter().zip(&receipts) {
        assert_eq!(entry["job"], local["job"]);
        assert_eq!(entry["receipt"], *receipt);
        let action: String = h
            .sql()
            .query_row(
                "SELECT input->>'$.action' FROM jobs WHERE id=?",
                [local["job"].as_str().unwrap()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(entry["action"], action);
        assert_eq!(entry["tool"], receipt["tool"]);
    }
    for (local, receipt) in &receipts {
        h.done(local, receipt);
    }
    assert_eq!(h.batch_count(&source), 1);
    assert_eq!(
        h.sql()
            .query_row("SELECT count(*) FROM attempts", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        4
    );
}

#[test]
fn held_retries_and_replacements_wait_for_current_execution_receipts() {
    let mut h = Harness::new();
    h.submit(json!({}));
    let source = h.provider();
    h.respond(
        &source,
        json!([
            tool("fs.read", json!({"path":"readme.txt"})),
            tool("fs.list", json!({"path":""}))
        ]),
        "wait_for",
    );
    let old = h.local();
    let stale = h.execute(&old);
    h = h.restart();
    h.done(&old, &stale);
    assert_eq!(h.batch_count(&source), 0);
    h.app
        .invoke(
            "control",
            json!({"project":h.project,"op":"retry","job":old["job"]}),
        )
        .unwrap();
    let current = h.local();
    assert_eq!(current["job"], old["job"]);
    assert_ne!(current["state"], old["state"]);
    h.done(&old, &stale);
    let state: String = h
        .sql()
        .query_row(
            "SELECT state FROM jobs WHERE id=?",
            [current["job"].as_str().unwrap()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(state, "executing");
    let receipt = h.execute(&current);
    h.done(&current, &receipt);
    assert_eq!(h.batch_count(&source), 0);
    let superseded = h.local();
    let stale = h.execute(&superseded);
    h = h.restart();
    h.app
        .invoke(
            "control",
            json!({"project":h.project,"op":"replace","job":superseded["job"]}),
        )
        .unwrap();
    h.done(&superseded, &stale);
    let replacement = h.local();
    assert_ne!(replacement["job"], superseded["job"]);
    let receipt = h.execute(&replacement);
    h.done(&replacement, &receipt);
    assert_eq!(h.batch_count(&source), 1);
    let followup = h.provider();
    let delivery = h.delivery(&followup);
    assert_eq!(delivery["tool_receipts"][1]["job"], replacement["job"]);
    assert_eq!(delivery["tool_receipts"][1]["receipt"], receipt);
}

#[test]
fn stopping_or_revising_a_request_discards_late_batch_completions() {
    for stop in [true, false] {
        let mut h = Harness::new();
        let sub = h.submit(json!({}));
        let source = h.provider();
        h.respond(
            &source,
            json!([
                tool("fs.list", json!({"path":""})),
                tool("fs.read", json!({"path":"readme.txt"}))
            ]),
            "wait_for",
        );
        let local = h.local();
        let receipt = h.execute(&local);
        h.done(&local, &receipt);
        let late = h.local();
        let receipt = h.execute(&late);
        if stop {
            h.app
                .invoke(
                    "control",
                    json!({"project":h.project,"op":"stop","request":sub["request"]}),
                )
                .unwrap();
        } else {
            h.submit(json!({"key":"revision","steer":sub["request"],"text":"New scope"}));
        }
        h.done(&late, &receipt);
        assert_eq!(h.batch_count(&source), 0);
    }
}

#[test]
fn live_external_reads_keep_legacy_rechecks_and_never_join_a_portable_batch() {
    let mut h = Harness::new();
    let external = h.temp.path().join("external.txt");
    fs::write(&external, "private external evidence").unwrap();
    let sub = h.submit(json!({}));
    let grant = json!({"project":h.project,"request":sub["request"],"revision":1,"op":"allow","host_path":external});
    h.app.invoke("source_access", grant.clone()).unwrap();
    let source = h.provider();
    h.respond(
        &source,
        json!([
            tool("fs.read", json!({"path":"readme.txt"})),
            tool("fs.read", json!({"host_path":external}))
        ]),
        "wait_for",
    );
    let internal = h.local();
    let receipt = h.execute(&internal);
    h.done(&internal, &receipt);
    assert_eq!(h.batch_count(&source), 0);
    let external = h.local();
    let receipt = h.execute(&external);
    let mut revoke = grant;
    revoke["op"] = json!("revoke");
    h.app.invoke("source_access", revoke).unwrap();
    h.done(&external, &receipt);
    assert_eq!(
        h.sql()
            .query_row(
                "SELECT state FROM jobs WHERE id=?",
                [external["job"].as_str().unwrap()],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "held"
    );
    assert_eq!(
        h.sql()
            .query_row("SELECT count(*) FROM selected_sources", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(h.sql().query_row("SELECT count(*) FROM jobs WHERE kind='message' AND input->'$.tool_receipt' IS NOT NULL",[],|r|r.get::<_,i64>(0)).unwrap(),1);
    let message: String = h
        .sql()
        .query_row(
            "SELECT input FROM jobs WHERE kind='message' AND input->'$.tool_receipt' IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!message.contains("private external evidence"));
}

#[test]
fn an_upgrade_midway_through_a_legacy_split_does_not_redeliver_its_first_receipt() {
    let mut h = Harness::new();
    h.submit(json!({}));
    let source = h.provider();
    let assignment = h.delivery(&source);
    h.respond(
        &source,
        json!([
            tool("fs.list", json!({"path":""})),
            tool("fs.read", json!({"path":"readme.txt"}))
        ]),
        "wait_for",
    );
    let first = h.local();
    let receipt = h.execute(&first);
    h.done(&first, &receipt);
    let sql = h.sql();
    let action: String = sql
        .query_row(
            "SELECT input->>'$.action' FROM jobs WHERE id=?",
            [first["job"].as_str().unwrap()],
            |r| r.get(0),
        )
        .unwrap();
    let legacy = json!({"tool_receipt":receipt,"action":action,"original_assignment":assignment});
    // Durable pre-upgrade shape: an already-routed single receipt for this source.
    sql.execute("INSERT INTO jobs(id,project,request,revision,session,role,kind,input) SELECT 'legacy-message',project,request,revision,session,role,'message',? FROM jobs WHERE id=?",[legacy.to_string(),first["job"].as_str().unwrap().to_owned()]).unwrap();
    drop(sql);
    h = h.restart();
    let second = h.local();
    let receipt = h.execute(&second);
    h.done(&second, &receipt);
    h.done(&first, &json!({"error":"duplicate"}));
    assert_eq!(h.batch_count(&source), 0);
    let messages: i64 = h.sql().query_row("SELECT count(*) FROM jobs WHERE kind='message' AND input->'$.tool_receipt' IS NOT NULL",[],|r|r.get(0)).unwrap();
    assert_eq!(messages, 2);
    let retained: String = h
        .sql()
        .query_row(
            "SELECT input FROM jobs WHERE id='legacy-message'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(serde_json::from_str::<Value>(&retained).unwrap(), legacy);
}

#[test]
fn different_foreground_sessions_release_only_their_own_sources() {
    let mut h = Harness::new();
    h.submit(json!({}));
    let intake = h.provider();
    let task = |label: &str| json!({"type":"TASK","summary":label,"role":"planner","assignment":label,"deliverable":label,"criterion":label,"constraints":[],"skills":[],"targets":[],"attachments":[],"prerequisites":[],"sources":[]});
    h.respond(
        &intake,
        json!([task("Inspect files"), task("Search files")]),
        "wait_for",
    );
    let first = h.provider();
    h.respond(
        &first,
        json!([
            tool("fs.list", json!({"path":""})),
            tool("fs.read", json!({"path":"readme.txt"}))
        ]),
        "wait_for",
    );
    let second = h.provider();
    h.respond(
        &second,
        json!([
            tool("fs.search", json!({"path":"readme.txt","query":"Alpha"})),
            tool("tool.find", json!({"query":"fs.read"}))
        ]),
        "wait_for",
    );
    let first_session: String = h
        .sql()
        .query_row(
            "SELECT session FROM jobs WHERE id=?",
            [first["job"].as_str().unwrap()],
            |r| r.get(0),
        )
        .unwrap();
    let second_session: String = h
        .sql()
        .query_row(
            "SELECT session FROM jobs WHERE id=?",
            [second["job"].as_str().unwrap()],
            |r| r.get(0),
        )
        .unwrap();
    assert_ne!(first_session, second_session);
    for index in 0..4 {
        let local = h.local();
        let receipt = h.execute(&local);
        h.done(&local, &receipt);
        assert_eq!(h.batch_count(&first), if index >= 1 { 1 } else { 0 });
        assert_eq!(h.batch_count(&second), if index == 3 { 1 } else { 0 });
    }
    for source in [&first, &second] {
        let delivery: String = h
            .sql()
            .query_row(
                "SELECT input FROM jobs WHERE input->>'$.tool_batch_source'=?",
                [source["job"].as_str().unwrap()],
                |r| r.get(0),
            )
            .unwrap();
        let delivery: Value = serde_json::from_str(&delivery).unwrap();
        for entry in delivery["tool_receipts"].as_array().unwrap() {
            let owner: String = h
                .sql()
                .query_row(
                    "SELECT input->>'$.source' FROM jobs WHERE id=?",
                    [entry["job"].as_str().unwrap()],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(owner, source["job"].as_str().unwrap());
        }
    }
}

#[test]
fn concurrent_duplicate_callbacks_publish_one_complete_batch() {
    use std::sync::{Arc, Barrier, Mutex};
    let mut h = Harness::new();
    h.submit(json!({}));
    let source = h.provider();
    h.respond(
        &source,
        json!([
            tool("fs.list", json!({"path":""})),
            tool("fs.read", json!({"path":"readme.txt"}))
        ]),
        "wait_for",
    );
    let first = h.local();
    let receipt = h.execute(&first);
    h.done(&first, &receipt);
    let last = h.local();
    let receipt = h.execute(&last);
    let callbacks = Arc::new(Barrier::new(3));
    let app = Arc::new(Mutex::new(h.app));
    let completions: Vec<_> = (0..2)
        .map(|_| {
            let callbacks = callbacks.clone();
            let app = app.clone();
            let input = json!({"job":last["job"],"state":last["state"],"result":receipt});
            std::thread::spawn(move || {
                callbacks.wait();
                app.lock().unwrap().invoke("tool_done", input).unwrap();
            })
        })
        .collect();
    callbacks.wait();
    for completion in completions {
        completion.join().unwrap();
    }
    let sql = rusqlite::Connection::open(h.data.join("state.sqlite3")).unwrap();
    assert_eq!(
        sql.query_row(
            "SELECT count(*) FROM jobs WHERE input->>'$.tool_batch_source'=?",
            [source["job"].as_str().unwrap()],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    let delivery: String = sql
        .query_row(
            "SELECT input FROM jobs WHERE input->>'$.tool_batch_source'=?",
            [source["job"].as_str().unwrap()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&delivery).unwrap()["tool_receipts"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn failed_read_is_reported_once_with_its_completed_siblings() {
    let mut h = Harness::new();
    h.submit(json!({}));
    let source = h.provider();
    h.respond(
        &source,
        json!([
            tool("fs.read", json!({"path":"missing.txt"})),
            tool("fs.list", json!({"path":""}))
        ]),
        "wait_for",
    );
    let missing = h.local();
    assert!(
        h.app
            .invoke("tool_run", missing["arguments"].clone())
            .is_err()
    );
    h.app.invoke("tool_done",json!({"job":missing["job"],"state":missing["state"],"error":"The requested project file does not exist."})).unwrap();
    assert_eq!(h.batch_count(&source), 0);
    let listed = h.local();
    let receipt = h.execute(&listed);
    h.done(&listed, &receipt);
    let continuation = h.provider();
    let delivery = h.delivery(&continuation);
    assert_eq!(
        delivery["tool_receipts"][0]["receipt"]["error"],
        "The requested project file does not exist."
    );
    assert_eq!(delivery["tool_receipts"][1]["receipt"], receipt);
    assert_eq!(h.batch_count(&source), 1);
}

#[test]
fn a_background_improvement_keeps_its_legacy_single_tool_continuation() {
    let mut h = Harness::new();
    h.submit(json!({}));
    let first = h.provider();
    h.respond(&first,json!([{"type":"RESULT","summary":"Read the findings.","result_kind":"final","findings":"Project reviewed.","evidence":[],"outcome":null}]),"complete_when");
    let action: String = h
        .sql()
        .query_row(
            "SELECT id FROM actions WHERE state='awaiting_acceptance'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    h.app
        .invoke(
            "control",
            json!({"project":h.project,"op":"accept","action":action}),
        )
        .unwrap();
    let background = h.provider();
    let assignment = h.delivery(&background);
    h.respond(
        &background,
        json!([tool("data.summarize", json!({"values":[2,4,6]}))]),
        "wait_for",
    );
    let local = h.local();
    let receipt = h.execute(&local);
    h.done(&local, &receipt);
    let continuation = h.provider();
    let delivery = h.delivery(&continuation);
    assert_eq!(delivery["tool_receipt"], receipt);
    assert_eq!(delivery["original_assignment"], assignment);
    assert!(delivery["tool_batch_source"].is_null());
    assert!(delivery["tool_receipts"].is_null());
    let role: String = h
        .sql()
        .query_row(
            "SELECT role FROM jobs WHERE id=?",
            [continuation["job"].as_str().unwrap()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(role, "workflow_improver");
}
