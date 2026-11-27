use fluxr_app_host::App;
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
};
struct TestProject {
    _temp: tempfile::TempDir,
    app: App,
    p: Value,
    path: PathBuf,
}
impl TestProject {
    fn new() -> Self {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("repo");
        fs::create_dir(&root).unwrap();
        let mut app = App::open(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("../app"),
            &t.path().join("db"),
        )
        .unwrap();
        let p = app.invoke("open_project", json!({"root":root})).unwrap()["id"].clone();
        Self {
            _temp: t,
            app,
            p,
            path: root,
        }
    }
    fn tool(&mut self, name: &str, args: Value) -> anyhow::Result<Value> {
        Ok(self.app.invoke(
            "tool_run",
            json!({"project":self.p,"tool":name,"arguments":args}),
        )?["value"]
            .clone())
    }
}
#[test]
fn scoped_catalog_reads_return_exact_project_records_and_artifact_ranges() {
    let mut f = TestProject::new();
    let root = f
        .app
        .invoke("open_project", json!({"root":f.path}))
        .unwrap();
    let sub = f
        .app
        .invoke(
            "submit",
            json!({"project":f.p,"key":"records","text":"Record a plan","attachments":[]}),
        )
        .unwrap();
    let rid = &sub["request"];
    let job = f.app.invoke("poll", Value::Null).unwrap()["job"].clone();
    f.app.invoke("seal", json!({"job":job})).unwrap();
    f.app.invoke("poll", Value::Null).unwrap();
    assert_eq!(
        f.tool("project.lookup", json!({"request":rid})).unwrap()[0]["revision"],
        1
    );
    assert!(
        f.tool("plan.update", json!({"request":"missing","plan":[]}))
            .is_err()
    );
    f.tool(
        "plan.update",
        json!({"request":rid,"revision":1,"plan":["write"]}),
    )
    .unwrap();
    assert_eq!(
        f.tool("plan.get", json!({"request":rid})).unwrap()[0]["plan"],
        json!(["write"])
    );
    assert_eq!(
        f.tool("session.list", json!({})).unwrap()[0]["session"],
        root["discriminator"]
    );
    assert_eq!(
        f.tool("session.get", json!({"id":root["discriminator"]}))
            .unwrap()[0]["role"],
        "discriminator"
    );
    assert_eq!(
        f.tool("job.status", json!({"id":job})).unwrap()[0]["state"],
        "running"
    );
    assert_eq!(
        f.tool("job.receipt", json!({"id":job})).unwrap()[0]["state"],
        "running"
    );
    assert_eq!(
        f.tool("job.trace", json!({"id":job})).unwrap()[0]["job"],
        job
    );
    assert_eq!(
        f.tool("daemon.metrics", json!({})).unwrap()[0]["model_attempts"],
        1
    );
    for name in [
        "workflow.find",
        "skill.find",
        "schedule.list",
        "task.list",
        "resource.find",
    ] {
        assert!(
            f.tool(name, json!({}))
                .unwrap()
                .as_array()
                .unwrap()
                .is_empty(),
            "{name}"
        );
    }
    let out = f
        .tool(
            "artifact.write",
            json!({"name":"log.txt","mime":"text/plain","base64":"YWJjZGU="}),
        )
        .unwrap();
    assert_eq!(
        f.tool("artifact.list", json!({})).unwrap()[0]["artifact"]["id"],
        out["id"]
    );
    let cut = f
        .tool(
            "artifact.read",
            json!({"artifact":out["id"],"range":{"unit":"bytes","start":1,"end":3}}),
        )
        .unwrap();
    assert_eq!(cut["text"], "bc");
    assert_eq!(cut["meta"]["digest"], out["digest"]);
    assert!(
        f.tool("artifact.read", json!({"artifact":"missing"}))
            .is_err()
    );
    assert!(
        !f.tool("tool.find", json!({"query":"git.stage"}))
            .unwrap()
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn real_git_objects_and_immutable_commit_snapshot() {
    use gix::objs::{
        Tree,
        tree::{Entry, EntryKind},
    };
    let mut f = TestProject::new();
    let repo = gix::init(&f.path).unwrap();
    let blob = repo.write_blob(b"hello").unwrap().detach();
    let tree = repo
        .write_object(Tree {
            entries: vec![Entry {
                mode: EntryKind::Blob.into(),
                filename: "hello.txt".into(),
                oid: blob,
            }],
        })
        .unwrap()
        .detach();
    let shown = f
        .tool("git.show", json!({"object":blob.to_string()}))
        .unwrap();
    assert_eq!(shown["text"], "hello");
    let listed = f
        .tool("git.tree", json!({"object":tree.to_string()}))
        .unwrap();
    assert_eq!(listed[0]["name"], "hello.txt");
    assert!(f.tool("git.show", json!({"object":"missing"})).is_err());
    assert!(f.tool("git.refs", json!({})).unwrap().is_array());
}
#[test]
fn stage_operations_stay_in_private_workspace() {
    let mut f = TestProject::new();
    f.tool("stage.mkdir", json!({"path":"a"})).unwrap();
    f.tool("stage.create", json!({"path":"a/one","base64":"eA=="}))
        .unwrap();
    f.tool("stage.copy", json!({"path":"a/one","destination":"a/two"}))
        .unwrap();
    assert!(
        f.tool("stage.copy", json!({"path":"a/one","destination":"a/two"}))
            .is_err()
    );
    f.tool(
        "stage.move",
        json!({"path":"a/two","destination":"a/three"}),
    )
    .unwrap();
    assert!(
        f.tool("stage.remove", json!({"path":"../state.sqlite3"}))
            .is_err()
    );
    f.tool("stage.remove", json!({"path":"a/three"})).unwrap();
    assert_eq!(fs::read_dir(&f.path).unwrap().count(), 0);
}
#[test]
fn timer_dispatch_requires_current_authority_and_creates_no_model_attempt() {
    let mut f = TestProject::new();
    let schedule=f.app.invoke("schedule",json!({"project":f.p,"op":"create","due":0,"period":60,"action":{"tool":"project.info","arguments":{}}})).unwrap();
    let occurrence = f.app.invoke("poll", Value::Null).unwrap();
    assert_eq!(occurrence["local"], "scheduled_read");
    let receipt = f
        .app
        .invoke("scheduled_read", occurrence["arguments"].clone())
        .unwrap();
    assert_eq!(receipt["tool"], "project.info");
    f.app
        .invoke(
            "scheduled_done",
            json!({"state":occurrence["state"],"result":receipt}),
        )
        .unwrap();
    f.app
        .invoke(
            "schedule",
            json!({"project":f.p,"op":"cancel","id":schedule["id"],"revision":1}),
        )
        .unwrap();
    assert!(
        f.app
            .invoke("scheduled_read", occurrence["arguments"].clone())
            .is_err()
    );
    assert!(f.app.invoke("poll", Value::Null).unwrap().is_null());
}
#[test]
fn source_queries_return_byte_spans_and_limit_results_to_the_project() {
    let mut f = TestProject::new();
    let text = "// λ\nfn target() {}\nfn caller() { target(); }";
    fs::write(f.path.join("lib.rs"), text).unwrap();
    let args = json!({"path":"lib.rs","language":"rust","symbol":"target"});
    let def = f.tool("code.definition", args.clone()).unwrap();
    let span = &def["candidates"][0];
    assert_eq!(
        &text[span["start"].as_u64().unwrap() as usize..span["end"].as_u64().unwrap() as usize],
        "target"
    );
    let refs = f.tool("code.references", args).unwrap();
    assert_eq!(refs["candidates"].as_array().unwrap().len(), 1);
    fs::write(f.path.join("broken.rs"), "fn {").unwrap();
    let diagnostic = f
        .tool(
            "code.diagnostics",
            json!({"path":"broken.rs","language":"rust"}),
        )
        .unwrap();
    assert!(!diagnostic["diagnostics"].as_array().unwrap().is_empty());
    let events = f
        .tool(
            "data.read_query",
            json!({"template":"recent_events","after":0}),
        )
        .unwrap();
    assert_eq!(events[0]["kind"], "Project opened");
    assert!(
        f.tool("data.read_query", json!({"template":"any_other_query"}))
            .is_err()
    );
}

#[test]
fn a_hard_link_cannot_expose_private_state_as_project_content() {
    let mut f = TestProject::new();
    fs::hard_link(f.app.data.join("state.sqlite3"), f.path.join("alias.bin")).unwrap();
    let err = f
        .tool("fs.digest", json!({"path":"alias.bin"}))
        .unwrap_err()
        .to_string();
    assert!(err.contains("hard-linked"), "{err}");
}

#[test]
fn restart_recovers_one_reserved_occurrence_under_the_same_identity() {
    let mut f = TestProject::new();
    f.app.invoke("schedule",json!({"project":f.p,"op":"create","due":0,"period":0,"action":{"tool":"project.info","arguments":{}}})).unwrap();
    let first = f.app.invoke("poll", Value::Null).unwrap();
    let data = f.app.data.clone();
    let bundle = f.app.bundle.clone();
    drop(f.app);
    let mut app = App::open(&bundle, &data).unwrap();
    let replay = app.invoke("poll", Value::Null).unwrap();
    assert_eq!(first, replay);
    assert!(app.invoke("poll", Value::Null).unwrap().is_null());
    let value = app
        .invoke("scheduled_read", replay["arguments"].clone())
        .unwrap();
    app.invoke(
        "scheduled_done",
        json!({"state":replay["state"],"result":value}),
    )
    .unwrap();
    drop(app);
    assert!(
        App::open(&bundle, &data)
            .unwrap()
            .invoke("poll", Value::Null)
            .unwrap()
            .is_null()
    );
}
#[test]
fn file_ranges_find_cursor_host_reads_and_monitor_cancellation() {
    let mut f = TestProject::new();
    fs::write(f.path.join("a.md"), "é\nsecond\n").unwrap();
    fs::write(f.path.join("b.md"), [0xff, 0x10]).unwrap();
    let page = f
        .tool("fs.find", json!({"pattern":"*.md","limit":1}))
        .unwrap();
    assert_eq!(page["entries"][0]["path"], "a.md");
    let next = f
        .tool(
            "fs.find",
            json!({"pattern":"*.md","limit":1,"cursor":page["cursor"],"snapshot":page["snapshot"]}),
        )
        .unwrap();
    assert_eq!(next["entries"][0]["path"], "b.md");
    assert!(next["cursor"].is_null());
    let range=f.tool("fs.read",json!({"path":"a.md","snapshot":page["snapshot"],"range":{"unit":"lines","start":1,"end":2}})).unwrap();
    assert_eq!(range["text"], "second\n");
    assert_eq!(
        f.tool("fs.read", json!({"path":"a.md"})).unwrap()["text"],
        "é\nsecond\n"
    );
    for range in [json!([]), json!({"unit":42}), json!("bytes")] {
        assert!(
            f.tool("fs.read", json!({"path":"a.md","range":range}))
                .is_err()
        );
    }
    let bytes = f
        .tool(
            "fs.read",
            json!({"path":"b.md","range":{"start":0,"end":1}}),
        )
        .unwrap();
    assert_eq!(bytes["encoding"], "binary");
    let outside = f._temp.path().join("outside");
    fs::write(&outside, "one").unwrap();
    assert_eq!(
        f.tool("fs.read", json!({"host_path":outside})).unwrap()["text"],
        "one"
    );
    fs::write(&outside, "two").unwrap();
    assert_eq!(
        f.tool("fs.read", json!({"host_path":outside})).unwrap()["text"],
        "two"
    );
    let monitor = f
        .tool(
            "monitor.start",
            json!({"due":0,"period":3600,"action":{"tool":"project.info","arguments":{}}}),
        )
        .unwrap();
    let occurrence = f.app.invoke("poll", Value::Null).unwrap();
    let result = f
        .app
        .invoke("scheduled_read", occurrence["arguments"].clone())
        .unwrap();
    f.app
        .invoke(
            "scheduled_done",
            json!({"state":occurrence["state"],"result":result}),
        )
        .unwrap();
    let status = f
        .tool("monitor.status", json!({"id":monitor["id"]}))
        .unwrap();
    assert_eq!(status["latest"]["tool"], "project.info");
    f.tool("monitor.stop", json!({"id":monitor["id"],"revision":1}))
        .unwrap();
    assert_eq!(
        f.tool("monitor.status", json!({"id":monitor["id"]}))
            .unwrap()["state"],
        "cancelled"
    );
}
