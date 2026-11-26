//! Checked desktop workflow contracts, exercised without creating a desktop window.
use fluxr_app_host::App;
use serde_json::{Value, json};
use std::time::Instant;
use std::{
    fs,
    path::{Path, PathBuf},
};

struct Desktop {
    _temp: tempfile::TempDir,
    root: PathBuf,
    app: App,
    state: Value,
    view: Value,
    frame: Value,
}
impl Desktop {
    fn empty() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("project");
        fs::create_dir(&root).unwrap();
        let app = App::open(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("../app"),
            &temp.path().join("state"),
        )
        .unwrap();
        let mut ui = Self {
            _temp: temp,
            root,
            app,
            state: json!({}),
            view: Value::Null,
            frame: Value::Null,
        };
        ui.render(true);
        ui
    }
    fn open() -> Self {
        let mut ui = Self::empty();
        ui.change("Existing project folder", "input", json!({"value":ui.root}));
        ui.click("Open project");
        assert!(ui.view["project"].is_string());
        ui
    }
    fn render(&mut self, reload: bool) {
        if reload {
            self.view = self
                .app
                .invoke("dashboard", self.state["nav"].clone())
                .unwrap();
        }
        self.frame = self
            .app
            .invoke(
                "desktop_frame",
                json!({"view":self.view,"state":self.state}),
            )
            .unwrap();
        let mut ids = std::collections::HashSet::new();
        for c in self.controls() {
            assert!(
                ids.insert(c["id"].as_str().unwrap()),
                "Duplicate native control ID: {}",
                c["id"]
            );
        }
    }
    fn controls(&self) -> &[Value] {
        self.frame["controls"].as_array().unwrap()
    }
    fn find(&self, predicate: impl Fn(&Value) -> bool) -> Value {
        self.controls()
            .iter()
            .find(|c| predicate(c))
            .unwrap_or_else(|| panic!("Missing control in {}", self.frame))
            .clone()
    }
    fn control(&self, label: &str, kind: &str) -> Value {
        self.find(|c| c["label"] == label && c["kind"] == kind)
    }
    fn event(&mut self, control: Value, input: Value) -> Value {
        self.app.invoke("desktop_event", json!({"state":self.state,"project":self.view["project"],"event":control["event"],"input":input})).unwrap()
    }
    fn apply(&mut self, mut response: Value) {
        self.state = response["state"].clone();
        let mut depth = 0;
        while let Some(request) = response.get("request").cloned() {
            depth += 1;
            assert!(depth < 16, "Unexpected request chain");
            let result = self
                .app
                .invoke(request["op"].as_str().unwrap(), request["input"].clone())
                .unwrap();
            response = self.app.invoke("desktop_result", json!({"state":self.state,"project":self.view["project"],"after":request["after"],"result":result})).unwrap();
            self.state = response["state"].clone();
        }
        if response["refresh"] != "none" {
            self.render(response["refresh"] == "data");
        }
    }
    fn click(&mut self, label: &str) {
        let control = self.control(label, "button");
        let response = self.event(control, json!({}));
        self.apply(response);
    }
    fn change(&mut self, label: &str, kind: &str, input: Value) {
        let response = self.event(self.control(label, kind), input);
        self.apply(response);
    }
    fn type_message(&mut self, text: &str) {
        let caret = text.encode_utf16().count();
        self.change(
            "Message",
            "multiline",
            json!({"value":text,"start":caret,"end":caret}),
        );
    }
    fn invoke(&mut self, export: &str, mut input: Value) -> Value {
        input["project"] = self.view["project"].clone();
        self.app.invoke(export, input).unwrap()
    }
}

#[test]
fn unicode_completion_shifts_pinned_markers_and_explicit_definition_history() {
    let mut ui = Desktop::open();
    let tea = ui.invoke(
        "dictionary",
        json!({"op":"save","name":"Thé","kind":"knowledge","definition":"A warm drink."}),
    );
    let cafe = ui.invoke(
        "dictionary",
        json!({"op":"save","name":"Café","kind":"knowledge","definition":"Use Thé."}),
    );
    let composer = ui.control("Message", "multiline");
    let scope = composer["event"]["scope"].as_str().unwrap().to_owned();
    ui.type_message("🙂 Caf");
    assert!(
        ui.state["definition"].is_null(),
        "Typing must not open a definition"
    );
    let completion =
        ui.find(|c| c["event"]["action"] == "complete" && c["event"]["hit"]["id"] == cafe["id"]);
    let response = ui.event(completion, json!({}));
    assert_eq!(response["focus"], composer["id"]);
    assert_eq!(
        response["selection"],
        json!({"id":composer["id"],"start":10,"end":10})
    );
    ui.apply(response);
    assert_eq!(ui.control("Message", "multiline")["value"], "🙂 Café");
    assert_eq!(ui.state["drafts"][&scope]["$markers"][0]["start"], 5);
    assert_eq!(ui.state["drafts"][&scope]["$markers"][0]["end"], 10);
    ui.type_message("Préfixe 🙂 Café!");
    // A single replacement spanning the whole value correctly drops overlapping markers.
    assert!(
        ui.state["drafts"][&scope]["$markers"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    ui.type_message("🙂 Caf");
    let completion =
        ui.find(|c| c["event"]["action"] == "complete" && c["event"]["hit"]["id"] == cafe["id"]);
    let response = ui.event(completion, json!({}));
    ui.apply(response);
    ui.type_message("Préfixe 🙂 Café");
    let marker = &ui.state["drafts"][&scope]["$markers"][0];
    assert_eq!(marker["start"], "Préfixe 🙂 ".len());
    assert_eq!(marker["end"], "Préfixe 🙂 Café".len());
    assert_eq!(marker["ref"]["id"], cafe["id"]);
    let submit = ui.event(ui.control("Send", "button"), json!({}));
    assert_eq!(submit["request"]["input"]["text"], "Préfixe 🙂 Café");
    assert_eq!(
        submit["request"]["input"]["markers"][0]["ref"]["revision"],
        1
    );
    ui.apply(submit);
    assert!(ui.state["definition"].is_null());
    let definition =
        ui.find(|c| c["event"]["action"] == "term" && c["event"]["ref"]["id"] == cafe["id"]);
    let response = ui.event(definition, json!({}));
    ui.apply(response);
    assert_eq!(ui.control("Café", "heading")["label"], "Café");
    assert_eq!(ui.state["termStack"].as_array().unwrap().len(), 1);
    let first_save_id = ui.control("Save revision", "button")["id"].clone();
    let nested =
        ui.find(|c| c["event"]["action"] == "term" && c["event"]["ref"]["id"] == tea["id"]);
    let response = ui.event(nested, json!({}));
    ui.apply(response);
    assert_eq!(ui.state["definition"]["entry"]["name"], "Thé");
    assert_ne!(ui.control("Save revision", "button")["id"], first_save_id);
    ui.click("Back");
    assert_eq!(ui.state["definition"]["entry"]["name"], "Café");
    ui.change(
        "Edit owner definition",
        "multiline",
        json!({"value":"Revised café definition."}),
    );
    ui.click("Save revision");
    assert_eq!(ui.state["definition"]["entry"]["revision"], 2);
    assert_eq!(
        ui.state["definition"]["entry"]["definition"],
        "Revised café definition."
    );
    assert_ne!(ui.control("Save revision", "button")["id"], first_save_id);
    ui.click("Close definition");
    assert!(ui.state["definition"].is_null());
    assert!(ui.state["termStack"].as_array().unwrap().is_empty());
}

#[test]
fn completing_terms_right_to_left_submits_in_text_order() {
    let mut ui = Desktop::open();
    for name in ["Café", "Thé"] {
        ui.invoke(
            "dictionary",
            json!({"op":"save","name":name,"kind":"knowledge","definition":name}),
        );
    }
    ui.type_message("Caf");
    let choice = ui.find(|c| c["event"]["hit"]["name"] == "Café");
    let r = ui.event(choice, json!({}));
    ui.apply(r);
    ui.change(
        "Message",
        "multiline",
        json!({"value":"Th Café","start":2,"end":2}),
    );
    let choice = ui.find(|c| c["event"]["hit"]["name"] == "Thé");
    let r = ui.event(choice, json!({}));
    ui.apply(r);
    let submit = ui.event(ui.control("Send", "button"), json!({}));
    let args = &submit["request"]["input"];
    assert_eq!(args["text"], "Thé Café");
    assert_eq!(args["markers"][0]["start"], 0);
    assert_eq!(args["markers"][1]["start"], 5);
    ui.apply(submit);
}

#[test]
fn graph_filter_pause_zoom_comparison_and_old_snapshot_read_retained_content() {
    let mut ui = Desktop::open();
    fs::write(ui.root.join("note.md"), "Original café.").unwrap();
    fs::write(ui.root.join("other.txt"), "Other file.").unwrap();
    ui.click("Project graph");
    ui.click("Capture snapshot");
    let old = ui.state["nav"]["snapshot"].clone();
    assert!(old.is_string());
    ui.change("Filter paths", "input", json!({"value":"NOTE"}));
    assert_eq!(
        ui.control("Project paths", "list")["choices"],
        json!([{"label":"note.md","value":"note.md"}])
    );
    ui.click("Pause graph");
    assert_eq!(ui.state["paused"], true);
    // The native driver delivers refresh notifications independently of controls.
    let response = ui.app.invoke("desktop_event",json!({"state":ui.state,"project":ui.view["project"],"event":{"action":"refresh"},"input":{}})).unwrap();
    assert_eq!(response["refresh"], "none");
    ui.apply(response);
    ui.click("Zoom in");
    assert!(
        (ui.control("Project paths", "list")["zoom"]
            .as_f64()
            .unwrap()
            - 1.2)
            .abs()
            < 1e-9
    );
    ui.click("Zoom out");
    assert!((ui.state["zoom"].as_f64().unwrap() - 1.0).abs() < 1e-9);
    ui.click("Zoom in");
    ui.click("Fit");
    assert_eq!(ui.state["zoom"], 1.0);
    ui.click("Resume graph");
    assert_eq!(ui.state["paused"], false);
    fs::write(ui.root.join("note.md"), "Changed café.").unwrap();
    ui.click("Capture snapshot");
    let current = ui.state["nav"]["snapshot"].clone();
    assert_ne!(old, current);
    ui.change("Compare with snapshot", "select", json!({"value":old}));
    assert!(
        ui.state["layers"]["changes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["path"] == "note.md")
    );
    fs::remove_file(ui.root.join("note.md")).unwrap();
    ui.click("Open previous note.md");
    assert_eq!(ui.state["nav"]["snapshot"], old);
    assert!(
        ui.controls()
            .iter()
            .any(|c| c["kind"] == "detail" && c["value"] == "Original café."),
        "Previous bytes must come from the snapshot"
    );
}

#[test]
fn tools_fold_preserves_typed_json_and_dispatches_exact_data() {
    let mut ui = Desktop::open();
    ui.click("Tools");
    assert!(
        !ui.controls()
            .iter()
            .any(|c| c["label"] == "Search arguments")
    );
    ui.click("Find and run a tool");
    ui.change(
        "Search arguments",
        "multiline",
        json!({"value":"{\"query\":\"dictionary\",\"limit\":3}"}),
    );
    ui.click("Find and run a tool");
    assert!(
        !ui.controls()
            .iter()
            .any(|c| c["label"] == "Search arguments")
    );
    ui.click("Find and run a tool");
    assert_eq!(
        ui.control("Search arguments", "multiline")["value"],
        "{\"query\":\"dictionary\",\"limit\":3}"
    );
    let request = ui.event(ui.control("Find tools", "button"), json!({}));
    assert_eq!(
        request["request"]["input"]["arguments"],
        json!({"query":"dictionary","limit":3})
    );
    assert_eq!(request["request"]["input"]["tool"], "tool.find");
    ui.apply(request);
    assert!(
        ui.state["outputs"]
            .as_object()
            .unwrap()
            .values()
            .any(|v| v["tool"] == "tool.find")
    );
    ui.change("Search arguments", "multiline", json!({"value":"{broken"}));
    let button = ui.control("Find tools", "button");
    let rejected = ui.app.invoke(
        "desktop_event",
        json!({"state":ui.state,"project":ui.view["project"],"event":button["event"],"input":{}}),
    );
    assert!(
        rejected.is_err(),
        "Malformed JSON must fail before dispatch"
    );
    ui.click("Tool versions");
    let promote = ui.control("Promote a version", "button");
    let scope = promote["event"]["scope"].clone();
    let payload = ui.find(|c| c["label"] == "Typed request" && c["event"]["scope"] == scope);
    let response = ui.event(
        payload,
        json!({"value":"{\"version\":\"literal\",\"nested\":{\"flag\":true},\"count\":2}"}),
    );
    ui.apply(response);
    let request = ui.event(promote, json!({}));
    assert_eq!(request["request"]["op"], "tool_promote");
    assert_eq!(request["request"]["input"]["nested"], json!({"flag":true}));
    assert_eq!(request["request"]["input"]["count"], 2);
    assert!(request["request"]["input"]["project"].is_string());
}

#[test]
fn many_project_tabs_keep_each_draft_addressable() {
    let mut ui = Desktop::open();
    let first = ui.view["project"].clone();
    ui.type_message("Draft one.");
    let mut last = Value::Null;
    for n in 1..8 {
        let folder = ui.root.with_file_name(format!("project-{n}"));
        fs::create_dir(&folder).unwrap();
        last = ui
            .app
            .invoke("open_project", json!({"root":folder}))
            .unwrap()["id"]
            .clone();
    }
    ui.render(true);
    ui.click("Move tabs to left");
    let switch = ui.control("Active project", "select");
    assert_eq!(switch["choices"].as_array().unwrap().len(), 8);
    assert_eq!(switch["side"], true);
    ui.change("Active project", "select", json!({"value":last}));
    ui.type_message("Draft eight.");
    ui.change("Active project", "select", json!({"value":first}));
    assert_eq!(ui.control("Message", "multiline")["value"], "Draft one.");
}

#[test]
fn large_history_and_graph_comparison_remain_browsable() {
    let mut ui = Desktop::empty();
    let cards:Vec<_>=(0..100).map(|n|json!({"type":"card","label":"Submission","text":format!("Message {n}"),"details":{"request":n},"items":[{"type":"button","label":"Steer","navigate":{"steer":n}}]})).collect();
    ui.view = json!({"project":"project","subtitle":"Project relay","components":[{"type":"feed","label":"Relay","items":cards}]});
    ui.render(false);
    ui.find(|c| c["value"] == "Message 99");
    ui.click("Older messages");
    ui.find(|c| c["value"] == "Message 87");
    ui.click("Newer messages");
    ui.find(|c| c["value"] == "Message 99");
    let edges: Vec<_> = (0..512)
        .map(|n| json!({"source":"source","target":format!("target-{n}"),"name":"call"}))
        .collect();
    let changes: Vec<_> = (0..2000)
        .map(|n| json!({"path":format!("file-{n}"),"old":"digest"}))
        .collect();
    ui.state = json!({"before":"old","layers":{"coverage":{"indexed":2,"total":2},"edges":edges,"changes":changes}});
    ui.view = json!({"project":"project","subtitle":"Graph","components":[{"type":"graph_layers","label":"Source relationships and activity","op":"graph_layers","snapshot":"new","versions":[{"id":"old"}]}]});
    ui.render(false);
    assert!(ui.controls().len() < 150);
    ui.click("More changes");
    ui.control("Open previous file-12", "button");
    ui.click("More relationships");
    ui.control("source → target-12 · call", "button");
}

#[test]
fn native_project_navigation_preserves_composer_and_dispatches_exact_unicode() {
    let start = Instant::now();
    let mut ui = Desktop::empty();
    eprintln!(
        "Checked application open to first desktop frame {:?}",
        start.elapsed()
    );
    ui.change("Existing project folder", "input", json!({"value":ui.root}));
    ui.click("Open project");
    let pid = ui.state["nav"]["project"].clone();
    assert!(pid.is_string());
    ui.click("Move tabs to left");
    assert_eq!(ui.control("Open project", "button")["side"], true);
    let body = "🙂 line\nCafé draft.";
    ui.change(
        "Message",
        "multiline",
        json!({"value":body,"start":19,"end":19}),
    );
    ui.click("Models");
    ui.control("Relay", "button");
    ui.click("Relay");
    assert_eq!(ui.control("Message", "multiline")["value"], body);
    let mut durations = vec![];
    for _ in 0..25 {
        let now = Instant::now();
        ui.render(false);
        durations.push(now.elapsed());
    }
    durations.sort();
    eprintln!(
        "Warm relay frame p50 {:?}, p95 {:?}",
        durations[12], durations[23]
    );
    let mut edits = Vec::new();
    let composer = ui.control("Message", "multiline");
    for i in 0..50 {
        let sample = format!("{body} {i}.");
        let units = sample.encode_utf16().count();
        let start = Instant::now();
        let r = ui.event(
            composer.clone(),
            json!({"value":sample,"start":units,"end":units}),
        );
        assert_eq!(r["refresh"], "none");
        ui.apply(r);
        edits.push(start.elapsed());
    }
    edits.sort();
    eprintln!(
        "Checked composer edit p50 {:?}, p95 {:?}",
        edits[25], edits[47]
    );
    ui.type_message(body);
    let r = ui.event(ui.control("Send", "button"), json!({}));
    assert_eq!(r["request"]["input"]["text"], body);
    assert!(r["request"]["input"]["key"].is_string());
    ui.apply(r);
    assert_eq!(ui.control("Message", "multiline")["value"], "");
    ui.click("Tools");
    ui.click("Dictionary");
    ui.click("Project graph");
    ui.click("Relay");
}
