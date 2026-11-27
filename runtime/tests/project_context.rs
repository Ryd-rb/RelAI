use fluxr_app_host::App;
use serde_json::{Value, json};
use std::{fs, path::Path};
fn fixture() -> (tempfile::TempDir, App) {
    let t = tempfile::tempdir().unwrap();
    let a = App::open(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../app"),
        &t.path().join("state"),
    )
    .unwrap();
    (t, a)
}
fn open(a: &mut App, path: &Path) -> Value {
    fs::create_dir(path).unwrap();
    a.invoke("open_project", json!({"root":path})).unwrap()["id"].clone()
}
#[test]
fn linked_lookup_preserves_revisions_and_unique_unicode_spans() {
    let (t, mut a) = fixture();
    let p = open(&mut a, &t.path().join("one"));
    let other = open(&mut a, &t.path().join("two"));
    let original=a.invoke("dictionary",json!({"project":p,"op":"save","name":"Café","kind":"knowledge","definition":"First wording."})).unwrap();
    a.invoke(
        "dictionary",
        json!({"project":other,"op":"link","dictionary":p}),
    )
    .unwrap();
    let refs = json!([original.clone(), original.clone()]);
    let get = |a: &mut App| {
        a.invoke(
            "dictionary",
            json!({"project":other,"op":"expand","refs":refs}),
        )
        .unwrap()
    };
    let result = get(&mut a);
    assert_eq!(result.as_array().unwrap().len(), 1);
    assert_eq!(result[0]["definition"]["definition"], "First wording.");
    let arg =
        json!({"project":other,"text":"Cafe\u{301} ✦ and Café.","attachments":[],"key":"mark"});
    a.invoke("submit", arg).unwrap();
    let job = a.invoke("poll", Value::Null).unwrap();
    a.invoke("seal", job.clone()).unwrap();
    a.invoke("poll", Value::Null).unwrap();
    let input = t
        .path()
        .join("state/turns")
        .join(job["job"].as_str().unwrap())
        .join("input.json");
    let data: Value = serde_json::from_slice(&fs::read(input).unwrap()).unwrap();
    let parts = data["parts"].as_array().unwrap();
    assert_eq!(
        parts
            .iter()
            .map(|v| v["text"].as_str().unwrap())
            .collect::<String>(),
        "Cafe\u{301} ✦ and Café."
    );
    assert_eq!(parts.iter().filter(|v| v.get("term").is_some()).count(), 2);
    a.invoke("dictionary",json!({"project":p,"op":"save","id":original["id"],"revision":1,"name":"Café","definition":"Revised.","kind":"knowledge"})).unwrap();
    assert_eq!(get(&mut a)[0]["definition"]["revision"], 1);
    a.invoke(
        "dictionary",
        json!({"project":other,"op":"unlink","dictionary":p}),
    )
    .unwrap();
    assert!(get(&mut a)[0]["definition"].is_null());
}
#[test]
fn snapshot_inspection_reads_retained_bytes_after_source_removal() {
    let (t, mut a) = fixture();
    let root = t.path().join("work");
    let p = open(&mut a, &root);
    fs::write(root.join("draft.txt"), "original").unwrap();
    let snap = a.invoke("capture_graph", json!({"project":p})).unwrap();
    fs::remove_file(root.join("draft.txt")).unwrap();
    let v = a
        .invoke(
            "dashboard",
            json!({"project":p,"view":"graph","snapshot":snap["snapshot"],"selected":"draft.txt"}),
        )
        .unwrap();
    assert!(v.to_string().contains("original"));
}
#[test]
fn skill_uses_pinned_content_and_reports_removed_revision() {
    let (t, mut a) = fixture();
    let p = open(&mut a, &t.path().join("work"));
    let folder = t.path().join("style");
    fs::create_dir(&folder).unwrap();
    let file = folder.join("SKILL.md");
    fs::write(
        &file,
        "---\nname: style\ndescription: Writing terms\n---\nUse short words.",
    )
    .unwrap();
    let installed = a
        .invoke("skill", json!({"project":p,"op":"install","path":folder}))
        .unwrap();
    fs::write(file, "updated outside RelAI").unwrap();
    let data = a
        .invoke(
            "skill",
            json!({"project":p,"op":"inspect","id":installed["id"]}),
        )
        .unwrap();
    assert_eq!(data["body"], "Use short words.");
    a.invoke(
        "skill",
        json!({"project":p,"op":"remove","id":installed["id"]}),
    )
    .unwrap();
    assert!(a.invoke("submit",json!({"project":p,"text":"Use style","key":"a","attachments":[],"skills":[installed["id"]]})).is_err());
}
#[test]
fn installed_skill_drafts_remain_inert_until_explicit_proposal_and_qualification() {
    let (t, mut app) = fixture();
    let p = open(&mut app, &t.path().join("work"));
    let dir = t.path().join("package");
    fs::create_dir(&dir).unwrap();
    fs::write(
        dir.join("SKILL.md"),
        "---\nname: checks\ndescription: Arithmetic checks\n---\nUse the declared checker.",
    )
    .unwrap();
    let contract = json!({"name":"plus","export":"run","capability":"integer.increment"});
    fs::write(dir.join("contract.json"), contract.to_string()).unwrap();
    fs::write(
        dir.join("check.fluxr"),
        "fluxr 1\npub run 1 x int int ret add x 1",
    )
    .unwrap();
    fs::write(
        dir.join("case.json"),
        r#"{"arguments":[{"i64":2}],"inputs":[],"expected":{"i64":3}}"#,
    )
    .unwrap();
    let mut manifest = json!({"tool_refs":[{"tool_id":"another","version":"unavailable","required":true}],"tool_drafts":[{"source_path":"check.fluxr","contract_path":"contract.json","test_paths":["case.json"]}]});
    let install = json!({"project":p,"op":"install","path":dir});
    fs::write(dir.join("relai-skill.json"), manifest.to_string()).unwrap();
    let id = app.invoke("skill", install.clone()).unwrap()["id"].clone();
    let read = app
        .invoke("skill", json!({"project":p,"op":"inspect","id":id}))
        .unwrap();
    assert_eq!(read["dependencies"][0]["available"], 0);
    assert_eq!(read["drafts"][0]["tests"][0]["expected"]["i64"], 3);
    let db = rusqlite::Connection::open(app.data.join("state.sqlite3")).unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM tools", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert!(
        app.invoke(
            "submit",
            json!({"project":p,"key":"pending","text":"Use checker","attachments":[],"skills":[id]})
        )
        .is_err()
    );
    fs::write(dir.join("check.fluxr"), "unused edited source").unwrap();
    let draft = app
        .invoke(
            "skill_draft",
            json!({"project":p,"version":id,"path":"check.fluxr"}),
        )
        .unwrap();
    assert_eq!(draft["status"], "draft");
    let inspected = app
        .invoke(
            "tool_run",
            json!({"project":p,"tool":"tool.inspect","arguments":{"version":draft["version"]}}),
        )
        .unwrap();
    let origin = &inspected["value"]["skill_draft_origins"][0];
    assert_eq!(origin["skill_version"], id);
    assert_eq!(origin["path"], "check.fluxr");
    assert_eq!(origin["author_cases"][0]["expected"]["i64"], 3);
    assert_eq!(
        db.query_row("SELECT count(*) FROM verified_checks", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        app.invoke(
            "tool_qualify",
            json!({"project":p,"version":draft["version"]})
        )
        .unwrap()["active"],
        true
    );

    manifest["tool_refs"] = json!([{"tool_id":"plus","version":draft["version"],"required":true}]);
    fs::write(dir.join("relai-skill.json"), manifest.to_string()).unwrap();
    let dependent = app.invoke("skill", install.clone()).unwrap()["id"].clone();
    let inspect = json!({"project":p,"op":"inspect","id":dependent});
    assert_eq!(
        app.invoke("skill", inspect.clone()).unwrap()["dependencies"][0]["available"],
        1
    );
    db.execute(
        "UPDATE tools SET runtime='obsolete' WHERE version=?",
        [draft["version"].as_str().unwrap()],
    )
    .unwrap();
    assert_eq!(
        app.invoke("skill", inspect).unwrap()["dependencies"][0]["available"],
        0
    );
    let error = app.invoke("submit", json!({"project":p,"key":"old-runtime","text":"Check","attachments":[],"skills":[dependent]}))
        .unwrap_err().to_string();
    assert!(
        error.contains("Required skill tool is unavailable"),
        "{error}"
    );

    manifest["tool_drafts"][0]["contract_path"] = json!("../contract.json");
    fs::write(dir.join("relai-skill.json"), manifest.to_string()).unwrap();
    assert!(app.invoke("skill", install).is_err());
}

#[test]
fn skill_package_rejects_duplicate_entries_and_incomplete_contracts_on_install() {
    let (t, mut app) = fixture();
    let project = open(&mut app, &t.path().join("project"));
    let package = t.path().join("package");
    fs::create_dir(&package).unwrap();
    fs::write(
        package.join("SKILL.md"),
        "---\nname: checks\ndescription: Checks\n---\nUse checks.",
    )
    .unwrap();
    let dependency = json!({"tool_id":"test","version":"one","required":true});
    let draft = json!({"source_path":"code.fluxr","contract_path":"contract.json","test_paths":[]});
    let install = json!({"project":project,"op":"install","path":package});
    for (manifest, error) in [
        (
            json!({"tool_refs":[dependency,dependency]}),
            "Duplicate skill dependency",
        ),
        (
            json!({"tool_drafts":[draft,draft]}),
            "Duplicate skill draft",
        ),
    ] {
        fs::write(package.join("relai-skill.json"), manifest.to_string()).unwrap();
        let actual = app
            .invoke("skill", install.clone())
            .unwrap_err()
            .to_string();
        assert!(actual.contains(error), "{actual}");
    }
    fs::write(
        package.join("relai-skill.json"),
        json!({"tool_drafts":[draft]}).to_string(),
    )
    .unwrap();
    fs::write(
        package.join("code.fluxr"),
        "fluxr 1\npub run 1 x int int ret add x 1",
    )
    .unwrap();
    fs::write(
        package.join("contract.json"),
        r#"{"name":"test","capability":"integer.increment"}"#,
    )
    .unwrap();
    assert!(app.invoke("skill", install).is_err());
}

#[test]
fn semantic_find_returns_dictionary_ranked_passages_at_exact_retained_byte_offsets() {
    let (t, mut app) = fixture();
    let root = t.path().join("repo");
    let p = open(&mut app, &root);
    app.invoke("dictionary",json!({"project":p,"op":"save","name":"Orchid","kind":"knowledge","definition":"Project codename"})).unwrap();
    let bytes = "🚀\nThe Orchid design is ready.\n";
    fs::write(root.join("minutes.txt"), bytes).unwrap();
    let id = app.invoke("capture_graph", json!({"project":p})).unwrap()["snapshot"].clone();
    fs::write(root.join("minutes.txt"), "replaced").unwrap();
    let result=app.invoke("tool_run",json!({"project":p,"tool":"project.semantic_find","arguments":{"query":"discuss Orchid","snapshot":id}})).unwrap();
    let hit = &result["value"]["passages"][0];
    assert_eq!(hit["path"], "minutes.txt");
    assert_eq!(hit["snapshot"], id);
    assert_eq!(hit["start"], 5);
    assert_eq!(
        &bytes[hit["start"].as_u64().unwrap() as usize..hit["end"].as_u64().unwrap() as usize],
        hit["body"].as_str().unwrap()
    );
}

#[test]
fn linked_definitions_use_exact_versions_and_do_not_confer_third_dictionary_access() {
    let (t, mut app) = fixture();
    let viewer = open(&mut app, &t.path().join("viewer"));
    let owner = open(&mut app, &t.path().join("owner"));
    let third = open(&mut app, &t.path().join("third"));
    let target=app.invoke("dictionary",json!({"project":third,"op":"save","name":"Beach","kind":"knowledge","definition":"Original."})).unwrap();
    let link = |p: &Value, id: &Value| json!({"project":p,"op":"link","dictionary":id});
    app.invoke("dictionary", link(&owner, &third)).unwrap();
    let source=app.invoke("dictionary",json!({"project":owner,"op":"save","name":"Coast","kind":"knowledge","definition":"Visit Beach."})).unwrap();
    app.invoke("dictionary", link(&viewer, &owner)).unwrap();
    let view = json!({"project":viewer,"ref":source});
    let locked = app.invoke("term_view", view.clone()).unwrap();
    assert!(
        locked["parts"]
            .as_array()
            .unwrap()
            .iter()
            .all(|p| p["term"].is_null())
    );
    assert_eq!(locked["entry"]["links"], json!([]));
    app.invoke("dictionary", link(&viewer, &third)).unwrap();
    app.invoke("dictionary",json!({"project":third,"op":"save","id":target["id"],"revision":1,"name":"Beach","kind":"knowledge","definition":"Revised."})).unwrap();
    let visible = app.invoke("term_view", view).unwrap();
    let term = visible["entry"]["links"][0].clone();
    assert_eq!(term["revision"], 1);
    assert_eq!(
        app.invoke("term_view", json!({"project":viewer,"ref":term}))
            .unwrap()["entry"]["definition"],
        "Original."
    );
}

#[test]
fn proposed_human_wording_requires_owner_acceptance_at_the_exact_revision() {
    let (t, mut app) = fixture();
    let p = open(&mut app, &t.path().join("project"));
    let owner = open(&mut app, &t.path().join("owner"));
    let original=app.invoke("dictionary",json!({"op":"save","project":owner,"name":"Week","kind":"knowledge","definition":"Seven days."})).unwrap();
    app.invoke(
        "dictionary",
        json!({"op":"link","project":p,"dictionary":owner}),
    )
    .unwrap();
    let source = app
        .invoke(
            "capture",
            json!({"project":p,"name":"note.txt","mime":"text/plain","base64":"aW5wdXQ="}),
        )
        .unwrap();
    let propose = |text: &str, rev: u64| json!({"project":p,"tool":"dictionary.propose","arguments":{"entry":original["id"],"revision":rev,"source":source["id"],"definition":text}});
    let first = app
        .invoke("tool_run", propose("Five working days.", 1))
        .unwrap()["value"]
        .clone();
    assert_eq!(
        app.invoke("tool_run", propose("Five working days.", 1))
            .unwrap()["value"],
        first
    );
    assert!(
        app.invoke(
            "review_definition",
            json!({"project":p,"op":"accept","proposal":first})
        )
        .is_err()
    );
    let stale = app.invoke("tool_run", propose("A schedule.", 1)).unwrap()["value"].clone();
    let accepted = app
        .invoke(
            "review_definition",
            json!({"project":owner,"op":"accept","proposal":first}),
        )
        .unwrap();
    assert_eq!(accepted["status"], "accepted");
    assert_eq!(accepted["ref"]["revision"], 2);
    assert_eq!(
        app.invoke(
            "review_definition",
            json!({"project":owner,"op":"accept","proposal":stale})
        )
        .unwrap()["status"],
        "conflict"
    );
    assert_eq!(
        app.invoke("term_view", json!({"project":p,"ref":original}))
            .unwrap()["entry"]["definition"],
        "Seven days."
    );
    assert_eq!(
        app.invoke("term_view", json!({"project":p,"ref":accepted["ref"]}))
            .unwrap()["entry"]["definition"],
        "Five working days."
    );
    let mut bogus = propose("No source", 2);
    bogus["arguments"]["source"] = json!("missing");
    assert!(app.invoke("tool_run", bogus).is_err());
}

#[test]
fn graph_does_not_read_a_different_projects_snapshot() {
    let (t, mut a) = fixture();
    let p = open(&mut a, &t.path().join("one"));
    let q = open(&mut a, &t.path().join("two"));
    let snap = a.invoke("capture_graph", json!({"project":p})).unwrap();
    assert!(
        a.invoke(
            "dashboard",
            json!({"project":q,"view":"graph","snapshot":snap["snapshot"]})
        )
        .is_err()
    );
}
#[test]
fn explicit_selection_disambiguates_two_dictionaries_and_validates_exact_utf8_spans() {
    let (t, mut a) = fixture();
    let p = open(&mut a, &t.path().join("one"));
    let other = open(&mut a, &t.path().join("two"));
    for project in [&p, &other] {
        a.invoke("dictionary",json!({"project":project,"op":"save","name":"Café","kind":"knowledge","definition":"A local definition."})).unwrap();
    }
    a.invoke(
        "dictionary",
        json!({"project":p,"op":"link","dictionary":other}),
    )
    .unwrap();
    let matches = a
        .invoke("suggest_terms", json!({"project":p,"prefix":"Caf"}))
        .unwrap();
    assert_eq!(matches.as_array().unwrap().len(), 2);
    let chosen = matches
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["dictionary_id"] == other)
        .unwrap();
    let marker = json!({"start":5,"end":10,"ref":chosen});
    let body = "🍵 Café!";
    let message = |key: &str, markers: Value| json!({"project":p,"text":body,"attachments":[],"key":key,"markers":markers});
    assert!(
        a.invoke("submit", message("overlap", json!([marker, marker])))
            .is_err()
    );
    let mut broken = marker.clone();
    broken["start"] = json!(1);
    assert!(
        a.invoke("submit", message("invalid", json!([broken])))
            .is_err()
    );
    a.invoke("submit", message("valid", json!([marker])))
        .unwrap();
    let job = a.invoke("poll", Value::Null).unwrap();
    a.invoke("seal", job.clone()).unwrap();
    a.invoke("poll", Value::Null).unwrap();
    let frame: Value = serde_json::from_slice(
        &fs::read(
            a.data
                .join("turns")
                .join(job["job"].as_str().unwrap())
                .join("input.json"),
        )
        .unwrap(),
    )
    .unwrap();
    let parts = frame["parts"].as_array().unwrap();
    assert_eq!(
        parts
            .iter()
            .filter_map(|i| i["text"].as_str())
            .collect::<String>(),
        body
    );
    assert!(
        parts
            .iter()
            .any(|v| v["term"]["dictionary_id"] == other && v["term"]["revision"] == 1)
    );
}
#[test]
fn graph_reindexes_legacy_coverage_and_resolves_module_file_layouts() {
    let (t, mut app) = fixture();
    let folder = t.path().join("repo");
    let project = open(&mut app, &folder);
    fs::create_dir_all(folder.join("src/foo")).unwrap();
    fs::create_dir_all(folder.join("src/sub")).unwrap();
    for (file, text) in [
        ("src/lib.rs", "mod foo; mod sub;"),
        ("src/foo/mod.rs", "mod leaf;"),
        ("src/foo/leaf.rs", "pub fn leaf() {}"),
        ("src/sub.rs", "mod inner;"),
        ("src/sub/inner.rs", "pub fn inner() {}"),
    ] {
        fs::write(folder.join(file), text).unwrap();
    }
    fs::write(folder.join("src/bad.rs"), [0xff, 0xff]).unwrap();
    let snapshot = app
        .invoke("capture_graph", json!({"project":project}))
        .unwrap()["snapshot"]
        .clone();
    let input = json!({"project":project,"snapshot":snapshot});
    let sql = rusqlite::Connection::open(app.data.join("state.sqlite3")).unwrap();
    sql.execute(
        "INSERT OR REPLACE INTO graph_indexes VALUES(?,6,6)",
        [snapshot.as_str().unwrap()],
    )
    .unwrap();
    let indexed = app.invoke("graph_layers", input.clone()).unwrap();
    assert_eq!(indexed["coverage"], json!({"indexed":5,"total":6}));
    let modules = indexed["edges"].as_array().unwrap();
    for (src, dst) in [
        ("src/lib.rs", "src/foo/mod.rs"),
        ("src/lib.rs", "src/sub.rs"),
        ("src/foo/mod.rs", "src/foo/leaf.rs"),
        ("src/sub.rs", "src/sub/inner.rs"),
    ] {
        assert!(
            modules
                .iter()
                .any(|edge| edge["source"] == src && edge["target"] == dst)
        );
    }
    assert_eq!(
        app.invoke("graph_layers", input).unwrap()["edges"],
        indexed["edges"]
    );
}

#[test]
fn graph_reports_relationship_and_identifier_limits_with_partial_results() {
    let (t, mut app) = fixture();
    let folder = t.path().join("repo");
    let project = open(&mut app, &folder);
    let mut source = String::from("fluxr 1\npub scan 0 int do 1 x int 4098\n");
    source.push_str(&"set x call h 0\n".repeat(4097));
    source.push_str("ret x\n");
    fs::write(folder.join("calls.fluxr"), source).unwrap();
    fs::write(folder.join("target.fluxr"), "fluxr 1\npub h 0 int ret 1\n").unwrap();
    let snapshot = app
        .invoke("capture_graph", json!({"project":project}))
        .unwrap();
    let layers = app
        .invoke(
            "graph_layers",
            json!({"project":project,"snapshot":snapshot["snapshot"]}),
        )
        .unwrap();
    assert_eq!(layers["edges"].as_array().unwrap().len(), 512);
    assert_eq!(layers["partial"], true);
    let coverage = layers["coverage_label"].as_str().unwrap();
    assert!(coverage.contains("source files examined"));
    assert!(coverage.contains("4,096"));
    assert!(coverage.contains("first 512"));
}

#[test]
fn graph_edges_reference_snapshot_bytes_and_stale_evidence_survives_comparison() {
    let (t, mut a) = fixture();
    let folder = t.path().join("repo");
    let p = open(&mut a, &folder);
    fs::write(folder.join("lib.rs"), "mod engine;").unwrap();
    fs::write(folder.join("engine.rs"), "pub fn go() {}").unwrap();
    fs::write(folder.join("a.fluxr"), "fluxr 1\nfn core 0 int ret 1").unwrap();
    fs::write(
        folder.join("b.fluxr"),
        "fluxr 1\npub run 0 int ret call core 0",
    )
    .unwrap();
    let old = a.invoke("capture_graph", json!({"project":p})).unwrap()["snapshot"].clone();
    let result = a
        .invoke("graph_layers", json!({"project":p,"snapshot":old}))
        .unwrap();
    let list = result["edges"].as_array().unwrap();
    let edge = list.iter().find(|v| v["kind"] == "module").unwrap();
    assert_eq!(edge["source"], "lib.rs");
    assert_eq!(edge["target"], "engine.rs");
    assert_eq!(edge["start"], 4);
    assert!(
        list.iter()
            .any(|v| v["kind"] == "call" && v["target"] == "a.fluxr")
    );
    fs::write(folder.join("lib.rs"), "pub fn main(){}").unwrap();
    let current = a.invoke("capture_graph", json!({"project":p})).unwrap()["snapshot"].clone();
    let diff = a
        .invoke(
            "graph_layers",
            json!({"project":p,"snapshot":current,"before":old}),
        )
        .unwrap();
    assert!(
        diff["stale_evidence"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["path"] == "lib.rs")
    );
    assert!(
        diff["edges"]
            .as_array()
            .unwrap()
            .iter()
            .all(|v| v["kind"] != "module")
    );
    assert_eq!(
        a.invoke("graph_layers", json!({"project":p,"snapshot":old}))
            .unwrap()["edges"],
        result["edges"]
    );
}

#[test]
fn retained_cargo_roots_and_path_attributes_resolve_exact_module_targets() {
    let (temp, mut app) = fixture();
    let root = temp.path().join("graph");
    let project = open(&mut app, &root);
    fs::create_dir_all(root.join("src")).unwrap();
    fs::create_dir_all(root.join("cli")).unwrap();
    fs::write(root.join("Cargo.toml"), "[package]\nname=\"example\"\nversion=\"0.1.0\"\n[lib]\npath=\"src/entry.rs\"\n[[bin]]\nname=\"example-bin\"\npath=\"cli/launch.rs\"\n").unwrap();
    fs::write(
        root.join("src/entry.rs"),
        "#[path = \"actual.rs\"] mod child;\nmod ordinary;\n",
    )
    .unwrap();
    fs::write(root.join("cli/launch.rs"), "mod helper;").unwrap();
    for file in [
        "src/actual.rs",
        "src/child.rs",
        "src/ordinary.rs",
        "cli/helper.rs",
    ] {
        fs::write(root.join(file), "").unwrap();
    }
    let captured = app
        .invoke("capture_graph", json!({"project":project}))
        .unwrap();
    let snapshot = captured["snapshot"].clone();
    fs::remove_file(root.join("Cargo.toml")).unwrap();
    fs::write(root.join("src/entry.rs"), "// edited after capture").unwrap();
    let graph = app
        .invoke(
            "graph_layers",
            json!({"project":project,"snapshot":snapshot}),
        )
        .unwrap();
    let edges = graph["edges"].as_array().unwrap();
    assert!(edges.iter().any(|e| e["source"] == "src/entry.rs"
        && e["target"] == "src/actual.rs"
        && e["name"] == "child"));
    assert!(
        !edges
            .iter()
            .any(|e| e["source"] == "src/entry.rs" && e["target"] == "src/child.rs")
    );
    assert!(
        edges
            .iter()
            .any(|e| e["source"] == "src/entry.rs" && e["target"] == "src/ordinary.rs")
    );
    assert!(
        edges
            .iter()
            .any(|e| e["source"] == "cli/launch.rs" && e["target"] == "cli/helper.rs")
    );
}
