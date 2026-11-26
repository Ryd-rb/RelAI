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
        ui.click("Enter folder path");
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
        #[cfg(windows)]
        fluxr_win32::validate_frame(&self.frame).expect("Invalid native frame");
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
fn welcome_opens_a_picked_project_and_workspace_identifies_the_current_view() {
    let mut ui = Desktop::empty();
    assert_eq!(ui.frame["theme"], "dark");
    assert_eq!(ui.frame["layout"], "welcome");
    assert_eq!(ui.control("Open a project", "heading")["role"], "title");
    let choose = ui.control("Choose folder…", "button");
    assert_eq!(ui.frame["default"], choose["id"]);
    assert_eq!(choose["role"], "primary");
    let effect = ui.event(choose, json!({}));
    assert_eq!(effect["platform"], "folder");
    let cancelled = ui
        .app
        .invoke(
            "desktop_result",
            json!({"state":ui.state,"after":effect["after"],"result":{"path":null}}),
        )
        .unwrap();
    assert!(cancelled.get("request").is_none());
    let selected = ui
        .app
        .invoke(
            "desktop_result",
            json!({"state":ui.state,"after":effect["after"],"result":{"path":ui.root}}),
        )
        .unwrap();
    ui.apply(selected);
    assert_eq!(ui.frame["layout"], "conversation");
    assert_eq!(ui.control("Relay", "button")["role"], "selected");
    assert_eq!(ui.control("project", "button")["role"], "selected");
    assert_eq!(ui.control("project", "button")["side"], true);
    assert_eq!(
        ui.control("Send", "button")["row"],
        ui.control("Attach files", "button")["row"]
    );
    assert_eq!(ui.control("Send", "button")["role"], "primary");
    assert!(
        !ui.control("project", "heading")["side"]
            .as_bool()
            .unwrap_or(false)
    );
    ui.click("Models");
    assert_eq!(ui.control("Models", "button")["role"], "selected");
    assert_eq!(ui.control("Relay", "button")["role"], "nav");
    assert_eq!(ui.control("Models", "heading")["role"], "title");
    assert!(
        ui.controls()
            .iter()
            .any(|c| c["label"] == "Edit preset" && c["kind"] == "heading")
    );
    assert_eq!(ui.control("Model ID", "input")["value"], "gpt-6.2-sol");
    ui.change("Role", "select", json!({"value":"reviewer"}));
    assert_eq!(ui.control("Reasoning effort", "select")["value"], "high");
    ui.change("Model ID", "input", json!({"value":"gpt-6.1-sol"}));
    ui.click("Save preset");
    assert_eq!(ui.control("gpt-6.1-sol", "heading")["id"], "models/active");
    ui.change("Role", "select", json!({"value":"planner"}));
    assert_eq!(ui.control("Model ID", "input")["value"], "gpt-6.2-sol");
    ui.click("Dictionary");
    let group = ui.control("Manage dictionaries", "button");
    assert_eq!(group["expanded"], false);
    assert!(
        !ui.controls()
            .iter()
            .any(|c| c["event"]["spec"]["label"] == "Create dictionary")
    );
    ui.click("Manage dictionaries");
    assert_eq!(
        ui.control("Manage dictionaries", "button")["expanded"],
        true
    );
    ui.control("Create dictionary", "button");
    let records = ui.find(|c| {
        c["label"]
            .as_str()
            .is_some_and(|s| s.starts_with("View records ("))
    });
    let open = ui.event(records, json!({}));
    ui.apply(open);
    assert_eq!(ui.control("Hide records", "button")["expanded"], true);
    ui.click("Open project");
    ui.control("Your projects", "label");
    ui.click("project");
    assert_eq!(ui.frame["layout"], "conversation");
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
    assert!(!ui.controls().iter().any(|c| c["label"] == "Tool ID"));
    ui.change("Search tools", "input", json!({"value":"project.info"}));
    let request = ui.event(ui.control("Find tools", "button"), json!({}));
    assert_eq!(
        request["request"]["input"]["arguments"],
        json!({"query":"project.info"})
    );
    assert_eq!(request["request"]["input"]["tool"], "tool.find");
    ui.apply(request);
    let matches = ui.control("Matching tools", "list");
    let selected = matches["choices"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["label"].as_str().unwrap().starts_with("project.info "))
        .unwrap()["value"]
        .clone();
    let select = ui.event(matches, json!({"value":selected}));
    assert!(select["request"].is_null());
    ui.apply(select);
    assert_eq!(ui.control("Tool ID", "input")["value"], "project.info");
    let run = ui.control("Run tool", "button");
    let scope = run["event"]["scope"].clone();
    let args = ui.find(|c| c["label"] == "Arguments" && c["event"]["scope"] == scope);
    let invalid = ui.event(args.clone(), json!({"value":"{broken"}));
    ui.apply(invalid);
    ui.click("Run a tool");
    assert!(!ui.controls().iter().any(|c| c["label"] == "Tool ID"));
    ui.click("Run a tool");
    assert_eq!(ui.find(|c| c["id"] == args["id"])["value"], "{broken");
    let rejected = ui.app.invoke(
        "desktop_event",
        json!({"state":ui.state,"project":ui.view["project"],"event":run["event"],"input":{}}),
    );
    assert!(
        rejected.is_err(),
        "Malformed arguments must fail before dispatch"
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
fn chat_pages_keep_durable_messages_visible_across_internal_traffic() {
    let mut ui = Desktop::open();
    let db = rusqlite::Connection::open(ui._temp.path().join("state/state.sqlite3")).unwrap();
    let project = ui.view["project"].as_str().unwrap().to_owned();
    for i in 0..29 {
        let request = format!("request-{i}");
        let message = format!("Message {i}");
        db.execute(
            "INSERT INTO requests (id,project,state,intent) VALUES(?,?,'completed',?)",
            (&request, &project, &message),
        )
        .unwrap();
        db.execute(
            "INSERT INTO events (project,kind,payload) VALUES(?,'Submission',?)",
            (
                &project,
                json!({"request":request,"submission":request,"text":message}).to_string(),
            ),
        )
        .unwrap();
    }
    db.execute("WITH RECURSIVE s(n) AS (VALUES(1) UNION ALL SELECT n+1 FROM s WHERE n<150) INSERT INTO events(project,kind,payload) SELECT ?,'Internal trace',json_object('sequence',n) FROM s",[&project]).unwrap();
    ui.render(true);
    assert_eq!(ui.frame["follow_tail"], true);
    assert_eq!(
        ui.frame["composer"],
        ui.control("Message", "multiline")["id"]
    );
    for label in ["Send", "Attach files"] {
        assert_eq!(ui.control(label, "button")["area"], "composer");
    }
    let transcript = |ui: &Desktop| {
        ui.controls()
            .iter()
            .filter(|c| c["role"] == "outgoing")
            .cloned()
            .collect::<Vec<_>>()
    };
    let page = transcript(&ui);
    assert_eq!(page.len(), 24);
    assert_eq!(page[0]["value"], "Message 5");
    assert_eq!(page[23]["value"], "Message 28");
    assert!(!ui.frame.to_string().contains("Internal trace"));
    ui.type_message("Unsent draft");
    ui.click("Earlier messages");
    assert_eq!(ui.frame["follow_tail"], false);
    let first = transcript(&ui);
    assert_eq!(first.len(), 5);
    assert_eq!(first[0]["value"], "Message 0");
    ui.invoke(
        "submit",
        json!({"key":"new","text":"A new message","attachments":[]}),
    );
    ui.render(true);
    assert_eq!(transcript(&ui), first);
    ui.click("Newer messages");
    assert_eq!(transcript(&ui), page);
    ui.click("Jump to latest");
    assert_eq!(transcript(&ui).last().unwrap()["value"], "A new message");
    ui.click("Activity");
    ui.control("Internal trace", "label");
    ui.click("Relay");
    assert_eq!(ui.control("Message", "multiline")["value"], "Unsent draft");
}

#[test]
fn a_question_arriving_while_typing_cannot_redirect_the_draft() {
    let mut ui = Desktop::open();
    let project = ui.view["project"].as_str().unwrap().to_owned();
    let req = ui.invoke(
        "submit",
        json!({"key":"first","text":"Please check","attachments":[]}),
    );
    ui.render(true);
    ui.type_message("Already composing");
    let db = rusqlite::Connection::open(ui._temp.path().join("state/state.sqlite3")).unwrap();
    let q = json!({"question":"Which folder?","choices":["One","Two"]});
    db.execute("INSERT INTO questions(id,request,revision,session,role,body) VALUES('question',?,1,'session','planner',?)",(req["request"].as_str().unwrap(),q.to_string())).unwrap();
    db.execute(
        "INSERT INTO events(project,kind,payload) VALUES(?,'ASK',?)",
        (
            &project,
            json!({"action":"question","request":req["request"],"payload":q}).to_string(),
        ),
    )
    .unwrap();
    ui.render(true);
    let send = ui.event(ui.control("Send", "button"), json!({}));
    assert!(send["request"]["input"]["reply"].is_null());
    ui.click("Reply");
    assert_eq!(
        ui.control("Message", "multiline")["value"],
        "Already composing"
    );
    assert_eq!(
        ui.event(ui.control("Send", "button"), json!({}))["request"]["input"]["reply"],
        "question"
    );
    ui.click("New topic");
    assert!(
        ui.event(ui.control("Send", "button"), json!({}))["request"]["input"]["reply"].is_null()
    );
    ui.click("Reply");
    ui.type_message("Two");
    ui.click("Send");
    assert!(!ui.controls().iter().any(|c| c["label"] == "Reply"));
    assert!(
        ui.app
            .invoke(
                "submit",
                json!({"project":project,"key":"another","text":"One","reply":"question"})
            )
            .is_err()
    );
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
    ui.click("Move tabs to top");
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
    ui.click("Earlier events");
    ui.find(|c| c["value"] == "Message 87");
    ui.click("Newer events");
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
    ui.click("Enter folder path");
    ui.change("Existing project folder", "input", json!({"value":ui.root}));
    ui.click("Open project");
    let pid = ui.state["nav"]["project"].clone();
    assert!(pid.is_string());
    ui.click("Move tabs to top");
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
