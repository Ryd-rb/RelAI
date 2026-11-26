use base64::{Engine, engine::general_purpose::STANDARD};
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
    fn artifact(&self, id: &Value) -> Vec<u8> {
        let db = rusqlite::Connection::open(self.data.join("state.sqlite3")).unwrap();
        let encoded: String = db
            .query_row(
                "SELECT encoded FROM artifacts WHERE id=? AND project=?",
                rusqlite::params![id.as_str().unwrap(), self.project.as_str().unwrap()],
                |row| row.get(0),
            )
            .unwrap();
        STANDARD.decode(encoded).unwrap()
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
fn publication_requires_a_reproduced_defect_and_preserves_the_same_receipt() {
    let mut f = Fixture::new();
    let source = "fluxr 1\npub run 1 x int int ret add x 1";
    let baseline = f.propose(
        "increment",
        "fluxr 1\npub run 1 x int int ret 2",
        "integer.increment",
    )["version"]
        .clone();
    let version = f.propose("increment", source, "integer.increment")["version"].clone();
    assert_eq!(
        f.call("tool_qualify", json!({"version":version})).unwrap()["active"],
        true
    );
    let args = json!({"operation":"increment","contract":"integer.increment","source":source,"description":"Correct constant output","checks":["counterexample"],"evidence":["regression"],"version":version,"baseline":baseline});
    let id = f.run("workflow.propose", args.clone())["value"]["candidate"].clone();
    let db = rusqlite::Connection::open(f.data.join("state.sqlite3")).unwrap();
    db.execute(
        "INSERT INTO requests(id,project,state,intent) VALUES('closed',?,'completed','increment')",
        [f.project.as_str().unwrap()],
    )
    .unwrap();
    db.execute(
        "INSERT INTO improvements VALUES('closed','improvement-session','job')",
        [],
    )
    .unwrap();
    let publish =
        json!({"tool":"workflow.publish","arguments":{"candidate":id,"request":"closed"}});
    assert!(f.call("tool_run", publish.clone()).is_err());
    assert_eq!(
        f.call("tool_qualify", json!({"version":baseline})).unwrap()["active"],
        false
    );
    let receipt = f.call("tool_run", publish.clone()).unwrap()["value"].clone();
    assert_eq!(receipt["version"], version);
    assert!(receipt["independent_check"]["failed"].is_string());
    assert!(receipt["independent_check"]["passed"].is_string());
    assert_eq!(receipt, f.call("tool_run", publish).unwrap()["value"]);
    let mut other = args;
    other["baseline"] = version;
    assert_ne!(f.run("workflow.propose", other)["value"]["candidate"], id);
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
fn promotion_updates_only_the_stable_term_binding_and_preserves_human_wording() {
    let mut f = Fixture::new();
    let first = f.propose(
        "tick",
        "fluxr 1\npub run 1 x int int ret add x 1",
        "integer.increment",
    )["version"]
        .clone();
    f.call("tool_qualify", json!({"version":first})).unwrap();
    let lookup = f.call("suggest_terms", json!({"prefix":"tick"})).unwrap();
    let term = lookup[0].clone();
    let id = term["id"].clone();
    let p = f.project.clone();
    let human=f.call("dictionary",json!({"op":"save","name":"tick","kind":"tool","id":id,"revision":1,"definition":"One more than the input."})).unwrap();
    let next = f.propose(
        "tick",
        "fluxr 1\npub run 1 x int int ret add 1 x",
        "integer.increment",
    )["version"]
        .clone();
    f.call("tool_qualify", json!({"version":next})).unwrap();
    f.call(
        "tool_promote",
        json!({"tool":"tick","version":next,"expected":first}),
    )
    .unwrap();
    let earlier = f.call("term_view", json!({"ref":human})).unwrap();
    assert_eq!(earlier["entry"]["tool_version"], first);
    let newer = f
        .call(
            "term_view",
            json!({"ref":{"id":id,"dictionary_id":p,"revision":3}}),
        )
        .unwrap();
    assert_eq!(newer["entry"]["tool_version"], next);
    assert_eq!(newer["entry"]["definition"], "One more than the input.");
    let sum=f.propose("tick","fluxr 1\npub run 1 xs list int int do 1 total int 3 set total 0 while not empty xs 2 bump total last xs pop xs ret total","integer.sum")["version"].clone();
    assert_eq!(
        f.call("tool_qualify", json!({"version":sum})).unwrap()["active"],
        true
    );
    assert!(
        f.call(
            "tool_promote",
            json!({"tool":"tick","version":sum,"expected":next})
        )
        .is_err()
    );
    assert_eq!(f.run("tick", json!([{"i64":7}]))["value"]["i64"], 8);
    f.call("tool_retire", json!({"version":next})).unwrap();
    assert!(
        f.call(
            "term_view",
            json!({"ref":{"id":id,"dictionary_id":p,"revision":4}})
        )
        .unwrap()["entry"]["tool_version"]
            .is_null()
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
    let snap = f.call("capture_graph", json!({})).unwrap()["snapshot"].clone();
    let pattern = json!({"path":"readme.md","snapshot":snap,"query":"o[n]+e","mode":"regex"});
    assert_eq!(f.run("fs.search", pattern)["value"]["matches"], matches);
    assert!(
        f.call(
            "tool_run",
            json!({"tool":"fs.search","arguments":{"path":"readme.md","query":"[","mode":"regex"}})
        )
        .is_err()
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

#[test]
fn document_and_csv_codecs_create_real_immutable_artifacts() {
    let mut f = Fixture::new();
    let content = "A café & a <tag>\nSecond paragraph";
    let written = f.run(
        "document.write",
        json!({"format":"docx","text":content,"name":"report.docx"}),
    )["value"]
        .clone();
    let bytes = f.artifact(&written["id"]);
    assert!(bytes.starts_with(b"PK"));
    let read = f.run(
        "document.read",
        json!({"artifact":written["id"],"format":"docx"}),
    );
    assert_eq!(read["value"]["text"].as_str().unwrap().trim_end(), content);
    assert_eq!(read["value"]["paragraphs"], 2);
    let markdown = f.run(
        "document.write",
        json!({"format":"markdown","text":"# Saved\n\nExact bytes.\n"}),
    )["value"]
        .clone();
    assert_eq!(f.artifact(&markdown["id"]), b"# Saved\n\nExact bytes.\n");
    let csv = f.run(
        "data.parse",
        json!({"format":"csv","text":"name,value\r\n\"a,b\",\"one\nline\"\r\n"}),
    );
    assert_eq!(
        csv["value"]["rows"],
        json!([["name", "value"], ["a,b", "one\nline"]])
    );
}

#[test]
fn spreadsheet_formula_and_notebook_edits_round_trip_through_generic_codecs() {
    let mut f = Fixture::new();
    let written=f.run("sheet.write",json!({"sheets":[{"name":"Evidence","cells":[{"row":0,"column":0,"value":2},{"row":1,"column":0,"value":3},{"row":2,"column":0,"formula":"=SUM(A1:A2)"},{"row":0,"column":1,"value":"café"}]}]}))["value"].clone();
    let read = f.run("sheet.read", json!({"artifact":written["id"]}));
    assert_eq!(read["value"]["sheets"][0]["name"], "Evidence");
    let cells = read["value"]["sheets"][0]["cells"].as_array().unwrap();
    assert!(
        cells
            .iter()
            .any(|cell| cell["row"] == 2 && cell["column"] == 0 && cell["formula"] == "SUM(A1:A2)")
    );
    assert!(cells.iter().any(|cell| cell["value"] == "café"));
    let notebook = json!({"nbformat":4,"nbformat_minor":5,"metadata":{},"cells":[{"cell_type":"code","id":"keep-id","metadata":{},"source":["old()"],"execution_count":9,"outputs":[{"output_type":"stream","name":"stdout","text":["old output"]}]}]});
    fs::write(
        f.root.join("input.ipynb"),
        serde_json::to_vec(&notebook).unwrap(),
    )
    .unwrap();
    let edited = f.run(
        "notebook.edit",
        json!({"path":"input.ipynb","edits":[{"index":0,"source":"new()"}]}),
    )["value"]
        .clone();
    let decoded: Value = serde_json::from_slice(&f.artifact(&edited["id"])).unwrap();
    assert_eq!(decoded["cells"][0]["source"], json!(["new()"]));
    assert_eq!(decoded["cells"][0]["outputs"], json!([]));
    assert!(decoded["cells"][0]["execution_count"].is_null());
    assert_eq!(decoded["cells"][0]["id"], "keep-id");
    assert_eq!(
        serde_json::from_slice::<Value>(&fs::read(f.root.join("input.ipynb")).unwrap()).unwrap(),
        notebook
    );
}

#[test]
fn archive_creation_and_extraction_are_contained_immutable_derivations() {
    let mut f = Fixture::new();
    let written=f.run("archive.create",json!({"entries":[{"path":"docs/first.txt","base64":STANDARD.encode(b"first")},{"path":"second.txt","base64":STANDARD.encode(b"second")}]}))["value"].clone();
    let listing = f.run("archive.list", json!({"artifact":written["id"]}));
    assert_eq!(listing["value"]["entries"].as_array().unwrap().len(), 2);
    let extracted = f.run("archive.extract", json!({"artifact":written["id"]}))["value"].clone();
    assert_eq!(f.artifact(&extracted[0]["id"]), b"first");
    assert_eq!(f.artifact(&extracted[1]["id"]), b"second");
    assert!(f.call("tool_run",json!({"tool":"archive.create","arguments":{"entries":[{"path":"../escape.txt","base64":STANDARD.encode(b"bad")}]}})).is_err());
    assert!(f.call("tool_run",json!({"tool":"archive.create","arguments":{"entries":[{"path":"a.txt","base64":""},{"path":"A.txt","base64":""}]}})).is_err());
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    zip.start_file("../escape.txt", zip::write::SimpleFileOptions::default())
        .unwrap();
    std::io::Write::write_all(&mut zip, b"bad").unwrap();
    let bytes = zip.finish().unwrap().into_inner();
    fs::write(f.root.join("hostile.zip"), bytes).unwrap();
    assert!(
        f.call(
            "tool_run",
            json!({"tool":"archive.extract","arguments":{"path":"hostile.zip"}})
        )
        .is_err()
    );
    assert!(!f._temp.path().join("escape.txt").exists());
}

#[test]
fn verification_runs_checked_source_in_separate_portable_phases() {
    let mut f = Fixture::new();
    let source = "fluxr 1\npub run 1 x int int ret add x 1";
    let formatted = f.run("verify.format", json!({"source":source,"export":"run"}));
    assert!(
        formatted["value"]["source"]
            .as_str()
            .unwrap()
            .contains("pub run")
    );
    assert_eq!(formatted["value"]["activated"], false);
    let pass = f.run(
        "verify.test",
        json!({"source":source,"export":"run","arguments":[{"i64":7}],"expected":{"i64":8}}),
    );
    assert_eq!(pass["value"]["passed"], true);
    let fail = f.run(
        "verify.test",
        json!({"source":source,"export":"run","arguments":[{"i64":7}],"expected":{"i64":99}}),
    );
    assert_eq!(fail["value"]["passed"], false);
    assert_eq!(fail["value"]["execution"]["value"], json!({"i64":8}));
    let lint = f.run("verify.lint", json!({"source":"echo shell","export":"run"}));
    assert!(lint["value"]["error"].is_string());
    let measurement = f.run(
        "verify.benchmark",
        json!({"source":source,"export":"run","arguments":[{"i64":1}]}),
    );
    assert!(measurement["value"]["execution"]["elapsed_micros"].is_number());
    assert_eq!(measurement["value"]["execution"]["route"], "portable");
}

#[test]
fn pdf_text_is_extracted_without_external_programs() {
    use lopdf::{
        Document, Object, Stream,
        content::{Content, Operation},
        dictionary,
    };
    let mut document = Document::with_version("1.5");
    let pages = document.new_object_id();
    let font = document
        .add_object(dictionary! {"Type"=>"Font","Subtype"=>"Type1","BaseFont"=>"Helvetica"});
    let resources = document.add_object(dictionary! {"Font"=>dictionary!{"F1"=>font}});
    let content = Content {
        operations: vec![
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec![Object::Name(b"F1".to_vec()), 12.into()]),
            Operation::new("Td", vec![20.into(), 20.into()]),
            Operation::new(
                "Tj",
                vec![Object::string_literal("Independent text evidence")],
            ),
            Operation::new("ET", vec![]),
        ],
    };
    let stream = document.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
    let page=document.add_object(dictionary!{"Type"=>"Page","Parent"=>pages,"Resources"=>resources,"Contents"=>stream,"MediaBox"=>vec![0.into(),0.into(),300.into(),300.into()]});
    document.objects.insert(
        pages,
        dictionary! {"Type"=>"Pages","Kids"=>vec![page.into()],"Count"=>1}.into(),
    );
    let catalog = document.add_object(dictionary! {"Type"=>"Catalog","Pages"=>pages});
    document.trailer.set("Root", catalog);
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).unwrap();
    let mut f = Fixture::new();
    fs::write(f.root.join("text.pdf"), bytes).unwrap();
    let read = f.run("document.read", json!({"path":"text.pdf","format":"pdf"}));
    assert!(
        read["value"]["text"]
            .as_str()
            .unwrap()
            .contains("Independent text evidence")
    );
    assert_eq!(read["value"]["pages"], 1);
}

#[test]
fn malformed_formats_and_expansion_limits_fail_without_publishing_artifacts() {
    let mut f = Fixture::new();
    assert!(
        f.call(
            "tool_run",
            json!({"tool":"document.write","arguments":{"format":"docx","text":"bad\u{0000}XML"}})
        )
        .is_err()
    );
    assert!(f.call("tool_run",json!({"tool":"sheet.write","arguments":{"sheets":[{"name":"Large","cells":[{"row":65536,"column":1,"value":1}]}]}})).is_err());
    fs::write(
        f.root.join("invalid.ipynb"),
        b"{\"nbformat\":3,\"cells\":[]}",
    )
    .unwrap();
    assert!(
        f.call(
            "tool_run",
            json!({"tool":"notebook.edit","arguments":{"path":"invalid.ipynb","edits":[]}})
        )
        .is_err()
    );
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    zip.start_file(
        "expanded.txt",
        zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated),
    )
    .unwrap();
    std::io::Write::write_all(&mut zip, &vec![0u8; 8 * 1024 * 1024 + 1]).unwrap();
    let mut bytes = zip.finish().unwrap().into_inner();
    // Lie about the expanded size in both headers; stream validation still rejects it.
    if let Some(offset) = bytes.windows(4).position(|w| w == b"PK\x01\x02") {
        bytes[offset + 24..offset + 28].copy_from_slice(&0u32.to_le_bytes());
    }
    bytes[22..26].copy_from_slice(&0u32.to_le_bytes());
    fs::write(f.root.join("forged.zip"), bytes).unwrap();
    assert!(
        f.call(
            "tool_run",
            json!({"tool":"archive.list","arguments":{"path":"forged.zip"}})
        )
        .is_err()
    );
    let db = rusqlite::Connection::open(f.data.join("state.sqlite3")).unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM artifacts", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

fn vm_paths(paths: &[&str]) -> Value {
    json!({"list":paths.iter().map(|path|json!({"list":path.as_bytes().iter().map(|byte|json!({"u64":byte})).collect::<Vec<_>>() })).collect::<Vec<_>>()})
}

#[test]
fn authored_markdown_tool_qualifies_against_portable_independent_fixtures() {
    let mut f = Fixture::new();
    let good = f.propose("markdown", MARKDOWN_TOOL, "markdown.relative_links")["version"].clone();
    let qualified = f.call("tool_qualify", json!({"version":good})).unwrap();
    assert_eq!(qualified["active"], true, "{qualified:#}");
    let receipt = f.run(
        "markdown",
        json!([
            vm_paths(&["docs/guide/start.md", "docs/contact.md"]),
            vm_paths(&["docs/guide/start.md"]),
            vm_paths(&["../contact.md"])
        ]),
    );
    assert_eq!(receipt["value"], json!({"list":[]}));
    assert_eq!(receipt["execution"]["route"], "portable");
}

#[test]
fn authored_markdown_qualification_uses_independent_counterexamples_and_exact_snapshots() {
    let mut f = Fixture::new();
    fs::create_dir_all(f.root.join("docs/guide")).unwrap();
    fs::create_dir(f.root.join("examples")).unwrap();
    fs::write(
        f.root.join("docs/guide/start.md"),
        "[contact](../contact.md)\n",
    )
    .unwrap();
    fs::write(f.root.join("docs/contact.md"), "Contact").unwrap();
    fs::write(
        f.root.join("examples/example.md"),
        "[missing](missing.md)\n",
    )
    .unwrap();
    let snapshot = f.call("capture_graph", json!({})).unwrap()["snapshot"].clone();
    let wrong = "fluxr 1\npub run 3 paths list list uint origins list list uint targets list list uint list uint ret list uint 0";
    let bad = f.propose("markdown", wrong, "markdown.relative_links")["version"].clone();
    let author=f.call("tool_test",json!({"version":bad,"case":{"arguments":[vm_paths(&["docs/guide/start.md","docs/contact.md"]),vm_paths(&["docs/guide/start.md"]),vm_paths(&["../contact.md"])],"inputs":[],"expected":{"list":[]}}})).unwrap();
    assert_eq!(author["passed"], true);
    assert_eq!(author["provisional"], true);
    let rejected = f.call("tool_qualify", json!({"version":bad})).unwrap();
    assert_eq!(rejected["active"], false);
    assert_eq!(
        rejected["checks"][1]["expected"],
        json!({"list":[{"u64":0}]})
    );
    assert_eq!(rejected["checks"][1]["observed"], json!({"list":[]}));
    assert!(
        f.call(
            "tool_run",
            json!({"tool":"markdown","version":bad,"arguments":{"snapshot":snapshot}})
        )
        .is_err()
    );
    let good = f.propose("markdown", MARKDOWN_TOOL, "markdown.relative_links")["version"].clone();
    let accepted = f.call("tool_qualify", json!({"version":good})).unwrap();
    assert_eq!(accepted["active"], true, "{accepted:#}");
    let clean = f.run("markdown", json!({"snapshot":snapshot}));
    assert_eq!(clean["count"], 0);
    assert_eq!(clean["findings"], json!([]));
    assert_eq!(clean["source_selection"]["snapshot"], snapshot);
    let examples = f.run(
        "markdown",
        json!({"snapshot":snapshot,"include_examples":true}),
    );
    assert_eq!(examples["count"], 1);
    assert_eq!(examples["findings"][0]["path"], "examples/example.md");
    assert_eq!(examples["findings"][0]["target"], "missing.md");
    assert_eq!(examples["execution"]["route"], "portable");
    fs::remove_file(f.root.join("docs/contact.md")).unwrap();
    assert_eq!(f.run("markdown", json!({"snapshot":snapshot}))["count"], 0);
    let changed = f.call("capture_graph", json!({})).unwrap()["snapshot"].clone();
    let broken = f.run("markdown", json!({"snapshot":changed}));
    assert_eq!(broken["count"], 1);
    assert_eq!(broken["findings"][0]["path"], "docs/guide/start.md");
    assert_eq!(broken["findings"][0]["target"], "../contact.md");
    assert!(broken["findings"][0]["start"].is_number());
    assert!(broken["findings"][0]["digest"].is_string());
}

#[test]
fn markdown_reference_and_percent_encoded_paths_use_commonmark_and_utf8() {
    let mut f = Fixture::new();
    fs::create_dir_all(f.root.join("docs/guide")).unwrap();
    fs::write(f.root.join("docs/guide/start.md"),"[contact][who]\n\n[who]: ../caf%C3%A9.md#section\n\n[remote](https://example.invalid/)\n[anchor](#local)\n").unwrap();
    fs::write(f.root.join("docs/café.md"), "# Contact").unwrap();
    let snapshot = f.call("capture_graph", json!({})).unwrap()["snapshot"].clone();
    let good = f.propose("markdown", MARKDOWN_TOOL, "markdown.relative_links")["version"].clone();
    assert_eq!(
        f.call("tool_qualify", json!({"version":good})).unwrap()["active"],
        true
    );
    assert_eq!(f.run("markdown", json!({"snapshot":snapshot}))["count"], 0);
}

// AUTHORED_MARKDOWN_FIXTURE
const MARKDOWN_TOOL: &str = r###"fluxr 1

fn byte_length 1 value list uint uint do 1 n uint 3
set n 0u
while not empty value 2
pop value
set n add n 1u
ret n

fn path_length 1 value list list uint uint do 1 n uint 3
set n 0u
while not empty value 2
pop value
set n add n 1u
ret n

fn equal_bytes 2 a list uint b list uint bool do 0  2
while and not empty a not empty b 3
if not eq last a last b 1
ret false
0

pop a
pop b
ret and empty a empty b

fn dirname 1 path list uint list uint do 0  2
while not empty path 2
if eq last path 47u 1
ret path
0

pop path
ret path

fn reverse 1 value list uint list uint do 1 out list uint 3
set out list uint 0
while not empty value 2
push out last value
pop value
ret out

fn normalized 2 origin list uint target list uint list uint do 9 combined list uint parts list list uint piece list uint i uint n uint byte uint out list uint part list uint k uint 12
if and not empty target eq at target 0u 47u 1
set combined target
4
set combined call dirname 1 origin
set i 0u
set n call byte_length 1 target
while lt i n 2
push combined at target i
set i add i 1u
set parts list list uint 0
set piece list uint 0
push combined 47u
set i 0u
set n call byte_length 1 combined
while lt i n 3
set byte at combined i
if eq byte 47u 2
if call equal_bytes 2 piece utf8 ".." 2
if empty parts 1
ret utf8 "\u0000"
0

pop parts
1
if and not empty piece not call equal_bytes 2 piece utf8 "." 1
push parts piece
0

set piece list uint 0
1
push piece byte
set i add i 1u
set out list uint 0
set i 0u
set n call path_length 1 parts
while lt i n 5
if lt 0u i 1
push out 47u
0

set part at parts i
set k 0u
while lt k call byte_length 1 part 2
push out at part k
set k add k 1u
set i add i 1u
ret out

fn has_path 2 paths list list uint target list uint bool do 0  2
while not empty paths 2
if call equal_bytes 2 last paths target 1
ret true
0

pop paths
ret false

pub run 3 paths list list uint origins list list uint targets list list uint list uint do 3 out list uint resolved list uint indexval uint 5
set out list uint 0
set indexval call path_length 1 targets
while not empty targets 6
if empty origins 1
ret list uint 1 18446744073709551615u
0

set indexval sub indexval 1u
set resolved call normalized 2 last origins last targets
if not call has_path 2 paths resolved 1
push out indexval
0

pop origins
pop targets
if not empty origins 1
ret list uint 1 18446744073709551615u
0

ret call reverse 1 out
"###;
