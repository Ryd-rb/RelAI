use fluxr_app_host::App;
use serde_json::{Value, json};
use std::{fs, path::PathBuf};

struct Fixture {
    _temp: tempfile::TempDir,
    app: App,
    project: Value,
    root: PathBuf,
    data: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let bundle = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../app");
        let root = temp.path().join("project");
        let data = temp.path().join("private");
        fs::create_dir(&root).unwrap();
        let mut app = App::open(&bundle, &data).unwrap();
        let project = app.invoke("open_project", json!({"root":root})).unwrap()["id"].clone();
        Self {
            _temp: temp,
            app,
            project,
            root,
            data,
        }
    }
    fn call(&mut self, name: &str, mut arguments: Value) -> anyhow::Result<Value> {
        arguments["project"] = self.project.clone();
        self.app.invoke(name, arguments)
    }
    fn propose(&mut self, name: &str, source: &str, capability: &str) -> Value {
        self.call(
            "tool_propose",
            json!({"name":name,"source":source,"export":"run","capability":capability}),
        )
        .unwrap()
    }
    fn run(&mut self, tool: &str, arguments: Value) -> Value {
        self.call("tool_run", json!({"tool":tool,"arguments":arguments}))
            .unwrap()
    }
}

#[test]
fn independent_counterexample_keeps_constant_answer_inactive_and_receipts_durable() {
    let mut f = Fixture::new();
    let draft = f.propose(
        "increment",
        "fluxr 1\npub run 1 x int int ret 2",
        "integer.increment",
    );
    assert_eq!(draft["status"], "draft");
    let version = draft["version"].clone();
    let author=f.call("tool_test",json!({"version":version,"case":{"arguments":[{"i64":1}],"inputs":[],"expected":{"i64":2}}})).unwrap();
    assert_eq!(author["passed"], true);
    assert_eq!(author["provisional"], true);
    assert!(
        f.call(
            "tool_run",
            json!({"tool":"increment","version":version,"arguments":[{"i64":1}]})
        )
        .is_err()
    );
    let qualified = f.call("tool_qualify", json!({"version":version})).unwrap();
    assert_eq!(qualified["active"], false);
    assert_eq!(qualified["checks"].as_array().unwrap().len(), 6);
    assert_eq!(qualified["checks"][0]["expected"], json!({"i64":1}));
    assert_eq!(qualified["checks"][0]["observed"], json!({"i64":2}));
    assert_eq!(
        f.run("tool.inspect", json!({"version":version}))["value"]["status"],
        "draft"
    );
    let db = rusqlite::Connection::open(f.data.join("state.sqlite3")).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM tool_tests WHERE version=?",
            [version.as_str().unwrap()],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        7
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM tool_defaults", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn active_exact_versions_defaults_and_retirement_use_checked_registry() {
    let mut f = Fixture::new();
    let first = f.propose(
        "increment",
        "fluxr 1\npub run 1 x int int ret add x 1",
        "integer.increment",
    )["version"]
        .clone();
    assert_eq!(
        f.call("tool_qualify", json!({"version":first})).unwrap()["active"],
        true
    );
    let receipt = f.run("increment", json!([{"i64":14}]));
    assert_eq!(receipt["value"], json!({"i64":15}));
    assert_eq!(receipt["version"], first);
    assert_eq!(receipt["execution"]["route"], "portable");
    assert_eq!(receipt["provisional"], false);
    let second = f.propose(
        "increment",
        "fluxr 1\npub run 1 x int int ret add 1 x",
        "integer.increment",
    )["version"]
        .clone();
    assert_ne!(first, second);
    assert_eq!(
        f.call("tool_qualify", json!({"version":second})).unwrap()["active"],
        true
    );
    assert_eq!(f.run("increment", json!([{"i64":3}]))["version"], first);
    assert!(
        f.call(
            "tool_promote",
            json!({"tool":"increment","version":second,"expected":null})
        )
        .is_err()
    );
    f.call(
        "tool_promote",
        json!({"tool":"increment","version":second,"expected":first}),
    )
    .unwrap();
    assert_eq!(f.run("increment", json!([{"i64":3}]))["version"], second);
    f.call("tool_retire", json!({"version":second})).unwrap();
    assert!(
        f.call(
            "tool_run",
            json!({"tool":"increment","version":second,"arguments":[{"i64":3}]})
        )
        .is_err()
    );
    assert_eq!(
        f.call(
            "tool_run",
            json!({"tool":"increment","version":first,"arguments":[{"i64":3}]})
        )
        .unwrap()["value"],
        json!({"i64":4})
    );
    f.call("tool_revoke", json!({"version":first})).unwrap();
    assert!(f.call("tool_retire", json!({"version":first})).is_err());
    assert!(
        f.call(
            "tool_run",
            json!({"tool":"increment","version":first,"arguments":[{"i64":3}]})
        )
        .is_err()
    );
}

#[test]
fn source_admission_and_vm_failures_preserve_next_valid_invocation() {
    let mut f = Fixture::new();
    for source in [
        "echo hello",
        "fluxr 1\nnative-use std::fs",
        "fluxr 1\npub run 1 x int int ret host steal int 0",
        "fluxr 1\npub run 1 x uint uint ret x",
    ] {
        assert!(f.call("tool_propose",json!({"name":"bad","source":source,"export":"run","capability":"integer.increment"})).is_err(),"{source}");
    }
    let fuel = f.propose(
        "loop",
        "fluxr 1\npub run 1 x int int do 0 2 while true 0 ret x",
        "integer.increment",
    )["version"]
        .clone();
    let qualification = f.call("tool_qualify", json!({"version":fuel})).unwrap();
    assert_eq!(qualification["active"], false);
    assert!(qualification["checks"][0]["error"].is_string());
    let valid=f.propose("sum","fluxr 1\npub run 1 xs list int int do 2 total int i int 4 set total 0 set i 0 while lt i 0 0 ret total","integer.sum");
    assert_eq!(valid["status"], "draft");
    assert_eq!(
        f.run("data.parse", json!({"text":"null"}))["value"],
        Value::Null
    );
    assert_eq!(
        f.run("data.summarize", json!({"values":[2,-1,8]}))["value"]["sum"],
        9
    );
}

#[test]
fn deterministic_file_tools_read_fresh_bytes_and_report_exact_literal_spans() {
    let mut f = Fixture::new();
    fs::write(f.root.join("readme.md"), "café one\nsecond one\n").unwrap();
    assert_eq!(
        f.run("fs.list", json!({"path":""}))["value"][0]["name"],
        "readme.md"
    );
    assert_eq!(
        f.run("fs.count", json!({"path":"readme.md","rule":"lines"}))["value"]["count"],
        2
    );
    assert_eq!(
        f.run("fs.count", json!({"path":"readme.md","rule":"words"}))["value"]["count"],
        4
    );
    let matches =
        f.run("fs.search", json!({"path":"readme.md","query":"one"}))["value"]["matches"].clone();
    assert_eq!(
        matches,
        json!([{"byte_start":6,"byte_end":9,"line":1,"column":6},{"byte_start":17,"byte_end":20,"line":2,"column":7}])
    );
    let before = f.run("fs.digest", json!({"path":"readme.md"}))["value"]["digest"].clone();
    fs::write(f.root.join("readme.md"), "changed").unwrap();
    assert_ne!(
        before,
        f.run("fs.digest", json!({"path":"readme.md"}))["value"]["digest"]
    );
    assert_eq!(
        f.run("fs.read", json!({"path":"readme.md"}))["value"]["text"],
        "changed"
    );
    assert!(
        f.call(
            "tool_run",
            json!({"tool":"fs.read","arguments":{"path":"../private/state.sqlite3"}})
        )
        .is_err()
    );
}

#[test]
fn project_scope_and_data_projection_are_enforced() {
    let mut f = Fixture::new();
    let draft = f.propose(
        "increment",
        "fluxr 1\npub run 1 x int int ret add x 1",
        "integer.increment",
    );
    let other = f._temp.path().join("other");
    fs::create_dir(&other).unwrap();
    let p = f.app.invoke("open_project", json!({"root":other})).unwrap()["id"].clone();
    assert!(f.app.invoke("tool_test",json!({"project":p,"version":draft["version"],"case":{"arguments":[{"i64":1}],"inputs":[],"expected":{"i64":2}}})).is_err());
    let transformed=f.run("data.transform",json!({"rows":[{"a":1,"keep":true},{"a":2,"keep":false}],"filter":{"field":"keep","value":true},"fields":["a"]}));
    assert_eq!(transformed["value"], json!([{"a":1}]));
    let catalog = f.call("tool_catalog", json!({})).unwrap();
    assert!(catalog["builtins"].as_array().unwrap().len() >= 10);
}
