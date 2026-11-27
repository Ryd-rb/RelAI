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
    fn receive_intake(&mut self, prompt: &str, actions: Value, next: &str) -> Value {
        let seal = self.app.invoke("poll", Value::Null).unwrap();
        self.app.invoke("seal", seal).unwrap();
        let job = self.app.invoke("poll", Value::Null).unwrap();
        let response = json!({
            "version":1, "actions":actions, "next":next,
            "dispositions":[{"start":0,"end":prompt.len(),"attachment":null,"disposition":"Answer the question."}]
        });
        self.app.invoke("receive",json!({"job":job["job"],"attempt":job["attempt"],"response":{"message":{"params":{"item":{"text":response.to_string()}}}}})).unwrap();
        self.render(true);
        job["job"].clone()
    }
}

#[test]
#[cfg(windows)]
fn chat_patch_requires_review_and_an_exact_grant_before_changing_files() {
    let mut ui = Desktop::empty();
    let topic = "Correct the README and add a second file.";
    let path = ui.root.join("README.md");
    let new_path = ui.root.join("guide.md");
    fs::write(&path, "wrong\n").unwrap();
    for args in [
        vec!["init", "--quiet"],
        vec!["add", "README.md"],
        vec![
            "-c",
            "user.email=relai@example.invalid",
            "-c",
            "user.name=RelAI",
            "commit",
            "--quiet",
            "-m",
            "base",
        ],
    ] {
        assert!(
            std::process::Command::new("git")
                .args(args)
                .current_dir(&ui.root)
                .status()
                .unwrap()
                .success()
        );
    }
    ui.click("Enter folder path");
    ui.change("Existing project folder", "input", json!({"value":ui.root}));
    ui.click("Open project");
    let file = ui.invoke(
        "tool_run",
        json!({"tool":"fs.read","arguments":{"path":"README.md"}}),
    );
    let arguments = json!({"files":[
        {"path":"README.md","digest":file["value"]["digest"],"edits":[{"old":"wrong","new":"correct"}]},
        {"path":"guide.md","digest":null,"edits":[{"old":"","new":"Guide\n"}]}
    ]});
    ui.type_message(topic);
    ui.click("Send");
    ui.receive_intake(topic, json!([{"type":"RUN_TOOL","summary":"Prepare the change.","tool":"patch.propose","arguments_json":arguments.to_string()}]), "wait_for");
    let job = ui.app.invoke("poll", Value::Null).unwrap();
    assert_eq!(job["local"], "chat_patch_propose");
    let proposed = ui
        .app
        .invoke("chat_patch_propose", job["arguments"].clone())
        .unwrap();
    let replay = ui
        .app
        .invoke("chat_patch_propose", job["arguments"].clone())
        .unwrap();
    assert_eq!(proposed["effect"], replay["effect"]);
    assert_eq!(proposed["state"], "planned");
    ui.app
        .invoke(
            "tool_done",
            json!({"job":job["job"],"state":job["state"],"result":proposed}),
        )
        .unwrap();
    ui.render(true);
    assert_eq!(fs::read_to_string(&path).unwrap(), "wrong\n");
    assert!(!new_path.exists());
    ui.control("Patch ready for review", "heading");
    assert!(
        ui.controls()
            .iter()
            .all(|c| c["label"] != "Approve and apply patch")
    );
    assert!(
        ui.app
            .invoke(
                "chat_patch_apply",
                json!({"project":ui.view["project"],"effect":proposed["effect"],"digest":"wrong"})
            )
            .is_err()
    );
    ui.click("Review patch");
    let preview = ui.control("README.md", "detail");
    assert!(
        preview["value"]
            .as_str()
            .unwrap()
            .contains("-wrong\n+correct\n")
    );
    ui.control("guide.md · New file", "detail");
    ui.type_message("Draft to preserve");
    ui.click("Approve and apply patch");
    ui.control("Patch applied", "heading");
    assert_eq!(ui.control("Review patch", "button")["expanded"], false);
    assert_eq!(
        ui.control("Message", "multiline")["value"],
        "Draft to preserve"
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), "correct\n");
    assert_eq!(fs::read_to_string(&new_path).unwrap(), "Guide\n");
    let replay = ui.app.invoke("chat_patch_apply", json!({"project":ui.view["project"],"effect":proposed["effect"],"digest":proposed["digest"]})).unwrap();
    assert_eq!(replay["state"], "completed");
}

#[test]
fn starter_prompts_focus_an_editable_draft_without_sending_or_overwriting() {
    let mut ui = Desktop::open();
    ui.control("Start a conversation", "heading");
    ui.control("Relay · project", "heading");
    ui.control("Enter to send · Shift+Enter for a new line", "label");
    let starter = ui.control("Find how to run it", "button");
    assert_eq!(starter["enabled"], true);
    ui.type_message(" \n ");
    assert_eq!(ui.control("Find how to run it", "button")["enabled"], false);
    let stale = ui.event(starter.clone(), json!({}));
    assert_eq!(stale["refresh"], "none");
    assert_eq!(stale["state"], ui.state);
    ui.type_message("");
    let effect = ui.event(starter.clone(), json!({}));
    assert_eq!(effect["focus"], ui.frame["composer"]);
    assert!(effect.get("request").is_none());
    ui.apply(effect);
    let message = ui.control("Message", "multiline");
    assert_eq!(
        message["value"],
        "How do I run this project locally? Include any prerequisites."
    );
    assert_eq!(ui.control("Send", "button")["enabled"], true);
    assert_eq!(ui.control("Find how to run it", "button")["enabled"], false);

    ui.type_message("My own draft");
    let stale = ui.event(starter.clone(), json!({}));
    assert_eq!(stale["refresh"], "none");
    assert_eq!(stale["state"], ui.state);
    ui.click("Send");
    assert_eq!(ui.control("You", "detail")["value"], "My own draft");
    let stale = ui.event(starter, json!({}));
    assert_eq!(stale["refresh"], "none");
    assert_eq!(stale["state"], ui.state);
}

#[test]
fn leaving_project_selection_restores_the_addressed_chat_draft_without_a_picker() {
    let mut ui = Desktop::open();
    let project = ui.view["project"].clone();
    ui.invoke(
        "submit",
        json!({"key":"opening-question","text":"Choose.","attachments":[]}),
    );
    let seal = ui.app.invoke("poll", Value::Null).unwrap();
    ui.app.invoke("seal", seal).unwrap();
    let model = ui.app.invoke("poll", Value::Null).unwrap();
    let response = json!({"version":1,"actions":[{"type":"ASK","summary":"Choose the style.","question":"Which style?","work":"Plan the interface","choices":["Buttons.","Links."]}],"next":"wait_for","dispositions":[{"start":0,"end":7,"attachment":null,"disposition":"Ask for the presentation."}]});
    ui.app.invoke("receive",json!({"job":model["job"],"attempt":model["attempt"],"response":{"message":{"params":{"item":{"text":response.to_string()}}}}})).unwrap();
    ui.render(true);
    ui.click("Buttons.");
    ui.type_message("Buttons. Keep this draft 🙂");
    let nav = ui.state["nav"].clone();
    let drafts = ui.state["drafts"].clone();
    ui.click("Open project");
    assert_eq!(ui.frame["layout"], "welcome");
    assert_eq!(ui.state["openReturn"], nav);
    let back = ui.control("Back", "button");
    assert_eq!(back["role"], "quiet");
    assert_eq!(
        ui.frame["default"],
        ui.control("Choose folder…", "button")["id"]
    );
    let returned = ui.event(back, json!({}));
    assert!(returned.get("platform").is_none());
    assert!(returned.get("request").is_none());
    let focus = returned["focus"].clone();
    ui.apply(returned);
    assert_eq!(ui.state["nav"], nav);
    assert_eq!(ui.view["project"], project);
    assert_eq!(ui.state["drafts"], drafts);
    assert_eq!(
        ui.control("Message", "multiline")["value"],
        "Buttons. Keep this draft 🙂"
    );
    assert_eq!(focus, ui.control("Message", "multiline")["id"]);
    ui.click("Open project");
    let pick = ui.event(ui.control("Choose folder…", "button"), json!({}));
    assert_eq!(pick["platform"], "folder");
    let cancelled = ui.app.invoke("desktop_result",json!({"state":ui.state,"project":ui.view["project"],"after":pick["after"],"result":{"path":null}})).unwrap();
    ui.apply(cancelled);
    assert_eq!(ui.state["nav"], nav);
    assert_eq!(ui.state["drafts"], drafts);
    assert_eq!(ui.frame["layout"], "conversation");
}

#[test]
fn project_selection_restores_its_previous_page_and_preserves_manual_path_recovery() {
    let mut ui = Desktop::open();
    let first = ui.view["project"].clone();
    ui.type_message("Keep the first project draft.");
    ui.click("Models");
    ui.click("Edit assignment");
    let nav = ui.state["nav"].clone();
    let panes = ui.state["panes"].clone();
    ui.click("Open project");
    ui.click("Enter folder path");
    let missing = ui.root.with_file_name("missing-project");
    ui.change("Existing project folder", "input", json!({"value":missing}));
    let manual = ui.event(ui.control("Open project", "button"), json!({}));
    assert_eq!(manual["request"]["op"], "open_project");
    assert!(
        ui.app
            .invoke("open_project", manual["request"]["input"].clone())
            .is_err()
    );
    ui.state = manual["state"].clone();
    ui.render(false);
    assert_eq!(
        ui.control("Existing project folder", "input")["value"],
        missing.to_string_lossy().as_ref()
    );
    ui.click("Back");
    assert_eq!(ui.state["nav"], nav);
    assert_eq!(ui.state["panes"], panes);
    ui.control("Model ID", "input");
    ui.click("Open project");
    let pick = ui.event(ui.control("Choose folder…", "button"), json!({}));
    let cancelled = ui.app.invoke("desktop_result",json!({"state":ui.state,"project":ui.view["project"],"after":pick["after"],"result":{"path":null}})).unwrap();
    assert!(cancelled.get("focus").is_none());
    ui.apply(cancelled);
    assert_eq!(ui.state["nav"], nav);
    assert_eq!(ui.state["panes"], panes);
    ui.control("Model ID", "input");
    ui.click("Open project");
    let picked = ui.root.with_file_name("second-project");
    fs::create_dir(&picked).unwrap();
    let pick = ui.event(ui.control("Choose folder…", "button"), json!({}));
    let selected = ui.app.invoke("desktop_result",json!({"state":ui.state,"project":ui.view["project"],"after":pick["after"],"result":{"path":picked}})).unwrap();
    ui.apply(selected);
    assert_ne!(ui.view["project"], first);
    assert_eq!(ui.frame["layout"], "conversation");
    assert!(ui.state["openReturn"].is_null());
    assert_eq!(ui.state["manualFolder"], false);
    assert_eq!(ui.control("Message", "multiline")["value"], "");
    ui.click("project");
    assert_eq!(
        ui.control("Message", "multiline")["value"],
        "Keep the first project draft."
    );
}

#[test]
fn cancelling_first_run_project_selection_keeps_welcome_and_the_manual_path() {
    let mut ui = Desktop::empty();
    assert!(!ui.controls().iter().any(|c| c["label"] == "Back"));
    ui.click("Enter folder path");
    ui.change("Existing project folder", "input", json!({"value":ui.root}));
    let draft = ui.state["drafts"].clone();
    let pick = ui.event(ui.control("Choose folder…", "button"), json!({}));
    let cancelled = ui
        .app
        .invoke(
            "desktop_result",
            json!({"state":ui.state,"after":pick["after"],"result":{"path":null}}),
        )
        .unwrap();
    assert!(cancelled.get("request").is_none());
    ui.apply(cancelled);
    assert_eq!(ui.frame["layout"], "welcome");
    assert_eq!(ui.state["drafts"], draft);
    assert!(!ui.controls().iter().any(|c| c["label"] == "Back"));
    ui.click("Open project");
    assert_eq!(ui.frame["layout"], "conversation");
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
    assert_eq!(ui.control("Relay · project", "heading")["area"], "header");
    let composer = ui.control("Message", "multiline");
    assert_eq!(composer["caption"], false);
    assert_eq!(composer["lines"], 2);
    assert_eq!(composer["placeholder"], "Message project…");
    assert!(
        !ui.control("Relay · project", "heading")["side"]
            .as_bool()
            .unwrap_or(false)
    );
    ui.click("Models");
    assert_eq!(ui.control("Models", "button")["role"], "selected");
    assert_eq!(ui.control("Relay", "button")["role"], "nav");
    assert_eq!(ui.control("Models", "heading")["area"], "header");
    assert_eq!(ui.control("Overview", "button")["selected"], true);
    assert!(
        ui.control("Role assignments", "list")["choices"]
            .as_array()
            .unwrap()
            .len()
            > 1
    );
    let edit = ui.event(ui.control("Edit assignment", "button"), json!({}));
    let focus = edit["focus"].clone();
    ui.apply(edit);
    assert_eq!(focus, ui.control("Provider", "select")["id"]);
    assert_eq!(ui.control("Model ID", "input")["section"], "model-settings");
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
    ui.click("Configuration");
    ui.change("Role", "select", json!({"value":"planner"}));
    assert_eq!(ui.control("Model ID", "input")["value"], "gpt-6.2-sol");
    ui.click("Dictionary");
    ui.control("Search terms", "input");
    ui.click("Dictionaries");
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
    ui.control("Recent projects", "label");
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
    ui.type_message("Unsent Relay draft");
    assert!(ui.controls().iter().all(|c| c["event"]["action"] != "term"));
    ui.click("Definitions (1)");
    let definition =
        ui.find(|c| c["event"]["action"] == "term" && c["event"]["ref"]["id"] == cafe["id"]);
    let response = ui.event(definition.clone(), json!({}));
    ui.apply(response);
    assert_eq!(ui.control("Café", "heading")["label"], "Café");
    assert_eq!(ui.state["termStack"].as_array().unwrap().len(), 1);
    let first_save_id = ui.control("Save revision", "button")["id"].clone();
    ui.change(
        "Edit owner definition",
        "multiline",
        json!({"value":"Revised café definition."}),
    );
    let nested =
        ui.find(|c| c["event"]["action"] == "term" && c["event"]["ref"]["id"] == tea["id"]);
    let response = ui.event(nested.clone(), json!({}));
    ui.apply(response);
    assert_eq!(ui.state["definition"]["entry"]["name"], "Thé");
    assert_ne!(ui.control("Save revision", "button")["id"], first_save_id);
    assert_eq!(
        ui.control("Edit owner definition", "multiline")["value"],
        "A warm drink."
    );
    ui.change("Edit owner definition", "multiline", json!({"value":""}));
    ui.click("Back");
    assert_eq!(ui.state["definition"]["entry"]["name"], "Café");
    assert_eq!(
        ui.control("Edit owner definition", "multiline")["value"],
        "Revised café definition."
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
    assert_eq!(
        ui.control("Message", "multiline")["value"],
        "Unsent Relay draft"
    );
    let reopen = ui.event(definition.clone(), json!({}));
    ui.apply(reopen);
    assert_eq!(
        ui.control("Edit owner definition", "multiline")["value"],
        "Use Thé."
    );
    let linked = ui.event(nested, json!({}));
    ui.apply(linked);
    assert_eq!(
        ui.control("Edit owner definition", "multiline")["value"],
        ""
    );
    ui.click("Back");
    ui.change(
        "Edit owner definition",
        "multiline",
        json!({"value":"Edit against old revision."}),
    );
    let stale = ui.event(ui.control("Save revision", "button"), json!({}));
    assert!(
        ui.app
            .invoke("dictionary", stale["request"]["input"].clone())
            .is_err()
    );
    ui.click("Close definition");
    let reopen = ui.event(definition, json!({}));
    ui.apply(reopen);
    assert_eq!(
        ui.control("Edit owner definition", "multiline")["value"],
        "Edit against old revision."
    );
}

#[test]
fn dictionary_proposal_review_shows_wording_and_decides_the_selected_proposal() {
    let mut ui = Desktop::open();
    let source = ui.invoke(
        "submit",
        json!({"key":"proposal-evidence","text":"Two terms for review.","attachments":[]}),
    );
    let mut ids = Vec::new();
    for (name, wording) in [
        ("First", "Original wording"),
        ("Second", "Selected wording"),
    ] {
        ids.push(ui.invoke("tool_run", json!({"tool":"dictionary.propose","arguments":{"name":name,"definition":wording,"source":source["id"]}}))["value"].clone());
    }
    ui.click("Dictionary");
    ui.click("Create or review definitions");
    let choice = ui.control("Pending proposals", "list");
    let selected = ui.event(choice, json!({"value":ids[1]}));
    ui.apply(selected);
    ui.find(|c| c["kind"] == "detail" && c["value"] == "Selected wording");
    let accept = ui.control("Accept proposal", "button");
    assert_eq!(accept["event"]["spec"]["fixed"]["proposal"], ids[1]);
    ui.click("Accept proposal");
    let remaining = ui.control("Pending proposals", "list");
    assert_eq!(remaining["choices"].as_array().unwrap().len(), 1);
    assert_eq!(remaining["value"], ids[0]);
    ui.click("Decline proposal");
    assert!(
        !ui.controls()
            .iter()
            .any(|c| c["label"] == "Accept proposal")
    );
}

#[test]
fn workspace_search_and_selected_activity_survive_navigation_and_refresh() {
    let mut ui = Desktop::open();
    for (name, definition) in [
        ("Green tea", "Unoxidized leaf."),
        ("Black tea", "Oxidized leaf."),
    ] {
        ui.invoke(
            "dictionary",
            json!({"op":"save","name":name,"kind":"knowledge","definition":definition}),
        );
    }
    ui.click("Dictionary");
    ui.change("Search terms", "input", json!({"value":"Green tea"}));
    let terms = ui.control("Terms", "list");
    assert_eq!(terms["choices"].as_array().unwrap().len(), 1);
    ui.click("Open definition");
    assert_eq!(
        ui.state["definition"]["entry"]["definition"],
        "Unoxidized leaf."
    );
    ui.click("Close definition");
    assert_eq!(ui.control("Search terms", "input")["value"], "Green tea");
    ui.click("Skills");
    ui.click("Tools");
    ui.click("Automation");
    ui.click("Dictionary");
    assert_eq!(ui.control("Skills", "button")["selected"], true);
    ui.click("Tools");
    assert_eq!(ui.control("Automation", "button")["selected"], true);

    let db = rusqlite::Connection::open(ui._temp.path().join("state/state.sqlite3")).unwrap();
    let project = ui.view["project"].as_str().unwrap().to_owned();
    let record = |kind: &str| {
        db.execute(
            "INSERT INTO events(project,kind,payload) VALUES(?,?,?)",
            (&project, kind, json!({"text":kind}).to_string()),
        )
        .unwrap();
        db.last_insert_rowid().to_string()
    };
    let saved = record("Worker failed");
    record("Submission");
    ui.click("Activity");
    ui.change("Events", "list", json!({"value":saved}));
    record("New event");
    ui.render(true);
    assert_eq!(ui.control("Events", "list")["value"], saved);
    assert_eq!(
        ui.find(|c| c["id"] == "activity/summary")["value"],
        "Worker failed"
    );
    assert!(ui.controls().iter().all(|c| c["id"] != "activity/details"));
    ui.click("Event record");
    assert_eq!(
        ui.control("Event details", "detail")["value"],
        json!({"text":"Worker failed"}).to_string()
    );
    ui.click("Needs attention");
    ui.find(|c| c["id"] == "activity/empty");
}

#[test]
fn activity_names_project_reads_and_keeps_the_record_available() {
    let mut ui = Desktop::open();
    fs::write(ui.root.join("notes.txt"), "Check the project").unwrap();
    ui.invoke(
        "tool_run",
        json!({"tool":"fs.read","arguments":{"path":"notes.txt"}}),
    );
    ui.click("Activity");
    let events = ui.control("Events", "list");
    let read = events["choices"]
        .as_array()
        .unwrap()
        .iter()
        .find(|choice| {
            choice["label"]
                .as_str()
                .is_some_and(|label| label.contains("Tool receipt · fs.read · notes.txt"))
        })
        .expect("Read is identifiable directly in the event list");
    ui.change("Events", "list", json!({"value":read["value"]}));
    ui.control("fs.read · notes.txt", "label");
    assert!(
        ui.find(|c| c["id"] == "activity/time")["value"]
            .as_str()
            .is_some_and(|time| time.ends_with(" UTC"))
    );
    assert!(ui.controls().iter().all(|c| c["id"] != "activity/details"));
    ui.click("Event record");
    assert!(
        ui.control("Event details", "detail")["value"]
            .as_str()
            .is_some_and(|record| record.contains("output_digest"))
    );
}

#[test]
fn term_search_and_chat_completion_fold_unicode_without_changing_the_inserted_name() {
    let mut ui = Desktop::open();
    for (name, search) in [("Straße", "STRASSE"), ("Café", "CAFE\u{301}")] {
        ui.invoke(
            "dictionary",
            json!({"op":"save","name":name,"kind":"knowledge","definition":name}),
        );
        let suggestions = ui.invoke("suggest_terms", json!({"prefix":search}));
        assert!(
            suggestions
                .as_array()
                .unwrap()
                .iter()
                .any(|hit| hit["name"] == name)
        );
        ui.click("Dictionary");
        ui.change("Search terms", "input", json!({"value":search}));
        assert_eq!(
            ui.control("Terms", "list")["choices"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        ui.click("Relay");
        ui.type_message(search);
        let completion = ui.find(|c| c["event"]["hit"]["name"] == name);
        let selected = ui.event(completion, json!({}));
        ui.apply(selected);
        assert_eq!(
            ui.control("Message", "multiline")["value"]
                .as_str()
                .unwrap()
                .trim_end(),
            name
        );
    }
}

#[test]
fn activity_pages_are_stable_and_attention_tracks_an_open_question() {
    let mut ui = Desktop::open();
    let project = ui.view["project"].as_str().unwrap().to_owned();
    let db = rusqlite::Connection::open(ui._temp.path().join("state/state.sqlite3")).unwrap();
    db.execute("DELETE FROM events WHERE project=?", [&project])
        .unwrap();
    for index in 0..200 {
        db.execute(
            "INSERT INTO events(project,kind,payload) VALUES(?,'Recorded',?)",
            (
                &project,
                json!({"text":format!("event {index}")}).to_string(),
            ),
        )
        .unwrap();
    }
    db.execute(
        "DELETE FROM event_times WHERE cursor=(SELECT min(cursor) FROM events WHERE project=?)",
        [&project],
    )
    .unwrap();
    ui.click("Activity");
    let choices = ui.control("Events", "list")["choices"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(choices.len(), 100);
    assert!(choices[0]["label"].as_str().unwrap().contains("UTC"));
    assert!(choices[0]["label"].as_str().unwrap().ends_with("event 199"));
    ui.click("Older activity");
    let older = ui.control("Events", "list")["choices"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(older.len(), 100);
    assert!(
        older[99]["label"]
            .as_str()
            .unwrap()
            .to_lowercase()
            .contains("unknown")
    );
    assert!(older[99]["label"].as_str().unwrap().ends_with("event 0"));
    assert!(
        !ui.controls()
            .iter()
            .any(|c| c["label"] == "Older activity" && c["enabled"] != false)
    );
    db.execute(
        "INSERT INTO events(project,kind,payload) VALUES(?,'Recorded','{}')",
        [&project],
    )
    .unwrap();
    ui.render(true);
    assert_eq!(ui.control("Events", "list")["choices"], json!(older));
    ui.click("Return to latest activity");
    assert_ne!(
        ui.control("Events", "list")["choices"][0]["value"],
        choices[0]["value"]
    );

    ui.click("Relay");
    ui.invoke(
        "submit",
        json!({"key":"current-question","text":"Choose.","attachments":[]}),
    );
    ui.receive_intake("Choose.",json!([{"type":"ASK","summary":"Choose a layout.","question":"Which layout?","work":"Choose a layout","choices":["List","Grid"]}]),"wait_for");
    ui.click("Activity");
    ui.click("Needs attention");
    let pending = ui.control("Events", "list")["choices"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(pending.len(), 1);
    assert!(
        pending[0]["label"]
            .as_str()
            .unwrap()
            .contains("Which layout?")
    );
    db.execute("UPDATE questions SET state='answered' WHERE request IN (SELECT id FROM requests WHERE project=?)", [&project]).unwrap();
    ui.render(true);
    ui.find(|c| c["id"] == "activity/empty");
    ui.click("Timeline");
    assert!(
        ui.control("Events", "list")["choices"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["value"] == pending[0]["value"])
    );
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
fn graph_browses_snapshot_children_and_restores_project_zoom() {
    let mut ui = Desktop::open();
    fs::create_dir_all(ui.root.join("src/nested")).unwrap();
    fs::write(ui.root.join("src/nested/file.txt"), "unchanged").unwrap();
    ui.click("Project graph");
    ui.click("Capture snapshot");
    let snapshot = ui.state["nav"]["snapshot"].clone();
    ui.change("Project paths", "list", json!({"value":"src/nested"}));
    ui.click("src/nested/file.txt · file");
    assert_eq!(ui.state["nav"]["snapshot"], snapshot);
    ui.find(|c| c["kind"] == "detail" && c["value"] == "unchanged");
    ui.click("Up one directory");
    assert_eq!(ui.state["nav"]["selected"], "src/nested");
    ui.click("Zoom in");

    let second = ui.root.with_file_name("second");
    fs::create_dir(&second).unwrap();
    ui.click("Open project");
    ui.click("Enter folder path");
    ui.change("Existing project folder", "input", json!({"value":second}));
    ui.click("Open project");
    ui.click("Project graph");
    ui.click("Capture snapshot");
    ui.control("Zoom 100%", "label");
    ui.click("Zoom out");
    ui.click("project");
    ui.click("Project graph");
    ui.control("Zoom 120%", "label");
}

#[test]
fn graph_filter_pause_zoom_comparison_and_old_snapshot_read_retained_content() {
    let mut ui = Desktop::open();
    fs::write(ui.root.join("note.md"), "Original café.").unwrap();
    fs::write(ui.root.join("other.txt"), "Other file.").unwrap();
    fs::write(ui.root.join("lib.rs"), "mod helper;\n").unwrap();
    fs::write(ui.root.join("helper.rs"), "pub fn helper() {}\n").unwrap();
    ui.click("Project graph");
    ui.control("Explore a retained source snapshot", "heading");
    ui.click("Capture snapshot");
    let old = ui.state["nav"]["snapshot"].clone();
    assert!(old.is_string());
    let snapshots = ui.control("Snapshot", "select");
    assert_eq!(snapshots["value"], old);
    assert!(
        snapshots["choices"][0]["label"]
            .as_str()
            .unwrap()
            .starts_with("Snapshot 1")
    );
    ui.change(
        "Filter paths",
        "input",
        json!({"value":"not-a-captured-path"}),
    );
    ui.find(|c| c["id"] == "graph/no-paths");
    assert!(!ui.controls().iter().any(|c| c["label"] == "Project paths"));
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
    ui.state["zoom"] = json!(3.0);
    ui.render(false);
    assert_eq!(ui.control("Zoom in", "button")["enabled"], false);
    ui.control("Zoom 300%", "label");
    ui.state["zoom"] = json!(0.5);
    ui.render(false);
    assert_eq!(ui.control("Zoom out", "button")["enabled"], false);
    ui.click("Reset zoom");
    assert_eq!(ui.state["zoom"], 1.0);
    ui.click("Resume graph");
    assert_eq!(ui.state["paused"], false);
    fs::write(ui.root.join("note.md"), "Changed café.").unwrap();
    ui.click("Capture snapshot");
    let current = ui.state["nav"]["snapshot"].clone();
    assert_ne!(old, current);
    ui.click("Relationships");
    ui.find(|c| c["id"] == "graph/inspect-prompt");
    ui.click("Inspect source relationships");
    ui.change("Filter relationships", "input", json!({"value":""}));
    assert!(
        ui.state["layers"]["edges"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["source"] == "lib.rs" && e["target"] == "helper.rs")
    );
    let target =
        ui.find(|c| c["label"] == "Open target" && c["event"]["patch"]["selected"] == "helper.rs");
    assert_eq!(target["event"]["patch"]["pane"], "Files");
    let open = ui.event(target, json!({}));
    ui.apply(open);
    assert_eq!(ui.state["nav"]["selected"], "helper.rs");
    ui.find(|c| c["kind"] == "detail" && c["value"] == "pub fn helper() {}\n");
    ui.click("Relationships");
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
    ui.change("Snapshot", "select", json!({"value":current}));
    ui.find(|c| c["kind"] == "detail" && c["value"] == "Changed café.");
    assert_eq!(ui.state["zoom"], 1.0);
}

#[test]
fn tools_fold_preserves_typed_json_and_dispatches_exact_data() {
    let mut ui = Desktop::open();
    ui.click("Tools");
    assert!(!ui.controls().iter().any(|c| c["label"] == "Tool ID"));
    let initial_matches = ui.control("Matching tools", "list");
    assert!(initial_matches["choices"].as_array().unwrap().len() > 1);
    ui.change("Search tools", "input", json!({"value":"project.info"}));
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
    ui.click("Versions");
    ui.click("Promote a version");
    let promote = ui.find(|c| c["event"]["spec"]["op"] == "tool_promote");
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
fn tool_library_keeps_arguments_and_output_with_the_selected_tool() {
    let mut ui = Desktop::open();
    fs::write(ui.root.join("garden.txt"), "Tulip receipt").unwrap();
    ui.click("Tools");
    let select = |ui: &mut Desktop, name: &str| {
        ui.change("Search tools", "input", json!({"value":name}));
        let list = ui.control("Matching tools", "list");
        let choices = list["choices"].as_array().unwrap();
        let choice = choices
            .iter()
            .find(|choice| {
                choice["label"].as_str().unwrap().split_whitespace().next() == Some(name)
            })
            .unwrap();
        let event = ui.event(list.clone(), json!({"value":choice["value"]}));
        ui.apply(event);
    };
    select(&mut ui, "fs.read");
    let input = r#"{"path":"garden.txt"}"#;
    ui.change("Arguments", "multiline", json!({"value":input}));
    ui.click("Run tool");
    assert!(
        ui.controls()
            .iter()
            .any(|c| c["id"].as_str().unwrap().ends_with("/output"))
    );
    select(&mut ui, "project.info");
    assert_ne!(ui.control("Arguments", "multiline")["value"], input);
    assert!(
        !ui.controls()
            .iter()
            .any(|c| c["id"].as_str().unwrap().ends_with("/output"))
    );
    select(&mut ui, "fs.read");
    assert_eq!(ui.control("Arguments", "multiline")["value"], input);
    assert!(ui.controls().iter().any(|c| {
        c["kind"] == "detail"
            && c["value"]
                .as_str()
                .is_some_and(|text| text.contains("relative path"))
    }));
    ui.change("Search tools", "input", json!({"value":"tool.find"}));
    assert_eq!(
        ui.control("Matching tools", "list")["choices"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    ui.change("Search tools", "input", json!({"value":"portable"}));
    ui.control("No matching tools.", "label");
    let all = ui.invoke(
        "tool_run",
        json!({"tool":"tool.find","arguments":{"query":"portable"}}),
    );
    assert!(!all["value"].as_array().unwrap().is_empty());
    let versions: Vec<Value> = [2, 3]
        .map(|n| {
            ui.invoke(
                "tool_propose",
                json!({
                    "name":"increment", "export":"run", "capability":"integer.increment",
                    "source":format!("fluxr 1\npub run 1 x int int ret {n}")
                }),
            )["version"]
                .clone()
        })
        .into();
    ui.render(true);
    ui.change("Search tools", "input", json!({"value":"increment"}));
    let matching = ui.control("Matching tools", "list");
    select(&mut ui, "increment");
    assert_eq!(
        ui.control("Run an exact registered version", "button")["enabled"],
        false
    );
    for version in versions {
        assert!(
            matching["choices"]
                .as_array()
                .unwrap()
                .iter()
                .any(|choice| choice["label"]
                    .as_str()
                    .unwrap()
                    .contains(version.as_str().unwrap()))
        );
    }
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
    assert_eq!(page[0]["caption"], true);
    assert_eq!(page[0]["label"], "You");
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
    assert!(
        ui.control("Events", "list")["choices"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["label"]
                .as_str()
                .is_some_and(|label| label.ends_with(" · Internal trace")))
    );
    ui.click("Needs attention");
    ui.find(|c| c["id"] == "activity/empty");
    ui.click("Relay");
    assert_eq!(ui.control("Message", "multiline")["value"], "Unsent draft");
}

#[test]
fn send_readiness_tracks_content_and_dismissing_an_error_keeps_the_draft() {
    let mut ui = Desktop::open();
    let send = |ui: &Desktop| ui.control("Send", "button");
    for blank in ["", " \r\n\t", "\u{a0}\u{3000}"] {
        ui.type_message(blank);
        assert_eq!(send(&ui)["enabled"], false);
        assert!(ui.event(send(&ui), json!({})).get("request").is_none());
    }
    ui.type_message("  keep this spacing  ");
    assert_eq!(send(&ui)["enabled"], true);
    ui.state["status"] = json!("Could not read a file.");
    ui.render(false);
    assert_eq!(
        ui.control("Could not read a file.", "label")["row"],
        ui.control("Dismiss message", "button")["row"]
    );
    let dismiss = ui.event(ui.control("Dismiss message", "button"), json!({}));
    assert_eq!(dismiss["focus"], ui.frame["composer"]);
    ui.apply(dismiss);
    assert!(ui.state["status"].is_null());
    assert_eq!(
        ui.control("Message", "multiline")["value"],
        "  keep this spacing  "
    );
    ui.type_message("");
    assert_eq!(send(&ui)["enabled"], false);

    let attach = ui.control("Attach files", "button");
    let effect = ui.event(attach, json!({}));
    let receipt = ui
        .app
        .invoke(
            "desktop_result",
            json!({
                "state":ui.state,"project":ui.view["project"],"after":effect["after"],
                "result":{"id":"attached","display_name":"notes.txt"}
            }),
        )
        .unwrap();
    ui.apply(receipt);
    assert_eq!(send(&ui)["enabled"], true);
    assert_eq!(
        ui.event(send(&ui), json!({}))["request"]["input"]["attachments"],
        json!(["attached"])
    );
    ui.click("Remove notes.txt · size unknown");
    assert_eq!(send(&ui)["enabled"], false);
    let field = ui.control("Attach files", "button")["event"].clone();
    let scope = field["scope"].as_str().unwrap();
    ui.state["drafts"][scope]["attachments"] =
        json!((0..8).map(|i| json!({"id":i,"name":i})).collect::<Vec<_>>());
    let capture = ui.app.invoke(
        "desktop_file",
        json!({"state":ui.state,"after":field,"project":ui.view["project"],"name":"ninth.md"}),
    );
    assert!(format!("{:#}", capture.unwrap_err()).contains("eight attachments"));
}

#[test]
fn file_only_messages_retain_the_author_size_and_exact_attachment_receipt() {
    let mut ui = Desktop::open();
    let attach = ui.control("Attach files", "button");
    let effect = ui.event(attach, json!({}));
    let capture = ui.invoke(
        "capture",
        json!({"name":"notes.txt","mime":"text/plain","base64":"YQ=="}),
    );
    let receipt = ui
        .app
        .invoke(
            "desktop_result",
            json!({
                "state":ui.state,"project":ui.view["project"],"after":effect["after"],
                "result":capture
            }),
        )
        .unwrap();
    ui.apply(receipt);
    ui.control("Remove notes.txt · 1 byte", "button");
    assert_eq!(ui.control("Send", "button")["enabled"], true);
    ui.click("Send");
    assert_eq!(
        ui.control("You", "detail")["value"],
        "Attached files\nnotes.txt"
    );
    ui.click("Attachment details");
    ui.control("notes.txt · 1 byte", "label");
    ui.click("Attachment receipts");
    let record = ui.control("Immutable sent attachment receipts", "detail");
    assert!(
        record["value"]
            .as_str()
            .unwrap()
            .contains(capture["digest"].as_str().unwrap())
    );
}

#[test]
fn changing_composer_skills_updates_the_visible_count_and_keeps_the_message() {
    let mut ui = Desktop::open();
    let db = rusqlite::Connection::open(ui._temp.path().join("state/state.sqlite3")).unwrap();
    db.execute(
        "INSERT INTO skills(id,project,name,digest,description,body,resources) VALUES('notes',?,'Notes','digest','Write notes','Body','[]')",
        [ui.view["project"].as_str().unwrap()],
    ).unwrap();
    ui.render(true);
    ui.type_message("Keep my draft");
    ui.click("Skills (0)");
    ui.change(
        "Allow skills for this submission",
        "multi",
        json!({"value":["notes"]}),
    );
    assert!(ui.controls().iter().any(|c| c["label"] == "Skills (1)"));
    let send = ui.event(ui.control("Send", "button"), json!({}));
    assert_eq!(send["request"]["input"]["skills"], json!(["notes"]));
    assert_eq!(send["request"]["input"]["text"], "Keep my draft");
}

#[test]
fn long_answer_preview_preserves_the_exact_answer_and_its_accept_action() {
    let mut ui = Desktop::open();
    let prompt = "Explain the changes.";
    let answer = (1..=50)
        .map(|n| format!("Line {n} with café."))
        .collect::<Vec<_>>()
        .join("\n");
    ui.type_message(prompt);
    ui.click("Send");
    ui.receive_intake(prompt, json!([{"type":"RESULT","summary":"Explanation","findings":answer,"result_kind":"final","evidence":[],"outcome":null}]),"complete_when");
    let preview = ui.control("RelAI · Answer ready for review", "detail");
    assert_eq!(
        preview["value"],
        format!("{}…", answer.lines().take(8).collect::<Vec<_>>().join("\n"))
    );
    let expand = ui.control("Show full answer", "button");
    assert_eq!(expand["expanded"], false);
    ui.click("Show full answer");
    let collapse = ui.control("Show less", "button");
    assert_eq!(collapse["id"], expand["id"]);
    assert_eq!(collapse["expanded"], true);
    assert!(
        ui.controls()
            .iter()
            .any(|c| c["role"] == "incoming" && c["value"] == answer)
    );
    ui.control("Request changes", "button");
    ui.click("Show less");
    ui.click("Accept answer");
    assert!(
        ui.controls()
            .iter()
            .any(|c| c["label"] == "RelAI · Accepted answer")
    );
}

#[test]
fn revising_and_accepting_an_answer_keeps_the_exchange_and_in_progress_draft() {
    let mut ui = Desktop::open();
    let question = "Which command?";
    let change = "Include the port.";
    let project = ui.view["project"].clone();
    let reply = |ui: &mut Desktop, prompt: &str, answer: &str| {
        ui.receive_intake(prompt,json!([{"type":"RESULT","summary":"Preview command.","findings":answer,"result_kind":"final","evidence":[],"outcome":null}]),"complete_when")
    };
    ui.type_message(question);
    ui.click("Send");
    ui.type_message("Unrelated draft");
    reply(&mut ui, question, "Use garden preview.");
    let untargeted = ui.event(ui.control("Send", "button"), json!({}));
    assert!(untargeted["request"]["input"]["steer"].is_null());
    let revise = ui.control("Request changes", "button");
    assert_eq!(revise["row"], ui.control("Accept answer", "button")["row"]);
    let selected = ui.event(revise.clone(), json!({}));
    assert_eq!(selected["focus"], ui.control("Message", "multiline")["id"]);
    ui.apply(selected);
    assert_eq!(
        ui.control("Message", "multiline")["value"],
        "Unrelated draft"
    );
    assert_eq!(
        ui.control("Message", "multiline")["placeholder"],
        "Describe the change…"
    );
    ui.click("New topic");
    let fresh = ui.event(ui.control("Send", "button"), json!({}));
    assert!(fresh["request"]["input"]["revise"].is_null());
    assert!(fresh["request"]["input"]["steer"].is_null());
    ui.click("Request changes");
    ui.type_message(change);
    let send = ui.event(ui.control("Send", "button"), json!({}));
    let submitted = send["request"]["input"].clone();
    let original = revise["event"]["patch"]["revise"].clone();
    assert_eq!(submitted["revise"], original);
    assert_eq!(submitted["text"], change);
    let other = ui.root.with_file_name("other");
    fs::create_dir(&other).unwrap();
    let mut foreign = submitted.clone();
    foreign["project"] = ui
        .app
        .invoke("open_project", json!({"root":other}))
        .unwrap()["id"]
        .clone();
    assert!(ui.app.invoke("submit", foreign).is_err());
    ui.apply(send);
    assert!(ui.app.invoke("submit", submitted.clone()).is_ok());
    let mut late = submitted;
    late["key"] = json!("late-change");
    assert!(ui.app.invoke("submit", late).is_err());

    let job = reply(&mut ui, change, "Use garden preview on port 4317.");
    let path = ui
        ._temp
        .path()
        .join("state/turns")
        .join(job.as_str().unwrap())
        .join("input.json");
    let sealed: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    assert_eq!(
        sealed["delivery"]["previous_exchange"],
        json!({"user_message":question,"answer":"Use garden preview.","answer_id":original})
    );
    ui.control("RelAI · Earlier answer", "detail");
    let stale = ui.event(revise, json!({}));
    ui.apply(stale);
    ui.type_message("Keep me");
    assert_eq!(ui.control("Send", "button")["enabled"], false);
    assert!(
        ui.app
            .invoke(
                "control",
                json!({"project":project,"op":"accept","action":original})
            )
            .is_err()
    );
    ui.click("Request changes");
    ui.type_message("Does it reload?");
    let accepted =
        ui.control("Accept answer", "button")["event"]["spec"]["fixed"]["action"].clone();
    let command = ui.event(ui.control("Accept answer", "button"), json!({}));
    let request = &command["request"];
    let result = ui.invoke("control", request["input"].clone());
    let reply = ui.invoke(
        "desktop_result",
        json!({"state":command["state"],"after":request["after"],"result":result}),
    );
    assert_eq!(reply["focus"], ui.control("Message", "multiline")["id"]);
    ui.apply(reply);
    ui.control("RelAI · Accepted answer", "detail");
    ui.control("Following up on “Which command?”", "label");
    assert_eq!(
        ui.control("Message", "multiline")["value"],
        "Does it reload?"
    );
    assert_eq!(
        ui.control("Message", "multiline")["placeholder"],
        "Ask a follow-up…"
    );
    let next = ui.event(ui.control("Send", "button"), json!({}));
    assert_eq!(next["request"]["input"]["follow"], accepted);
    assert!(next["request"]["input"]["revise"].is_null());
}

#[test]
fn a_newer_answer_keeps_the_draft_and_older_questions_can_be_reached_from_any_page() {
    for count in [1, 2] {
        let mut ui = Desktop::open();
        ui.type_message("Choose a style.");
        ui.click("Send");
        let questions: Vec<_> = ["Which colour?", "Which font?"][..count].iter().map(|text|
            json!({"type":"ASK","summary":"Choose a style.","work":"Style the page.","question":text,"choices":[]})
        ).collect();
        ui.receive_intake("Choose a style.", json!(questions), "wait_for");
        if count == 2 {
            ui.control("Question to answer", "select");
        } else {
            assert_eq!(
                ui.control("Message", "multiline")["placeholder"],
                "Write your reply…"
            );
        }

        // Each intervening request and stop add two real transcript entries.
        for i in 0..13 {
            let sub = ui.invoke(
                "submit",
                json!({"key":format!("old-{i}"),"text":"Intervening topic."}),
            );
            ui.invoke("control", json!({"op":"stop","request":sub["request"]}));
        }
        ui.render(true);
        ui.click("New topic");
        ui.type_message("Check the build.");
        ui.click("Send");
        ui.receive_intake("Check the build.",json!([{"type":"RESULT","summary":"Build checked.","findings":"The build passes.","result_kind":"final","outcome":null,"evidence":[]}]),"complete_when");
        ui.control("Earlier messages", "button");
        assert!(!ui.controls().iter().any(|c| c["value"] == "Which colour?"));
        let answer =
            ui.control("Accept answer", "button")["event"]["spec"]["fixed"]["action"].clone();
        ui.type_message("Keep this draft.");
        let send_target = |ui: &mut Desktop| {
            ui.event(ui.control("Send", "button"), json!({}))["request"]["input"].clone()
        };
        assert_eq!(send_target(&mut ui)["revise"], answer);
        ui.click("Other questions");
        let picker = ui.control("Question to answer", "select");
        assert_eq!(picker["choices"].as_array().unwrap().len(), count + 1);
        let old = picker["choices"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["label"] == "Which colour?")
            .unwrap()["value"]
            .clone();
        assert_eq!(send_target(&mut ui)["revise"], answer);
        ui.click("Accept answer");
        assert_eq!(send_target(&mut ui)["follow"], answer);
        if !ui
            .controls()
            .iter()
            .any(|c| c["kind"] == "select" && c["label"] == "Question to answer")
        {
            ui.click("Other questions");
        }
        ui.change("Question to answer", "select", json!({"value":old}));
        assert_eq!(send_target(&mut ui)["reply"], old);
        assert!(send_target(&mut ui)["follow"].is_null());
        assert_eq!(
            ui.control("Message", "multiline")["value"],
            "Keep this draft."
        );
        ui.click("New topic");
        let fresh = send_target(&mut ui);
        assert!(
            ["reply", "revise", "follow", "steer"]
                .iter()
                .all(|key| fresh[*key].is_null())
        );
        assert!(
            !ui.controls()
                .iter()
                .any(|c| c["label"] == "Other questions")
        );
    }
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
    ui.type_message("");
    let choose = ui.control("Two", "button");
    let effect = ui.event(choose.clone(), json!({}));
    assert!(effect.get("request").is_none());
    assert_eq!(effect["selection"]["end"], 3);
    ui.apply(effect);
    assert_eq!(ui.control("Message", "multiline")["value"], "Two");
    ui.type_message("Two, please");
    let question = ui.control("RelAI · Replying to this question", "detail");
    let body = question["value"].as_str().unwrap();
    assert_eq!(body.matches("One").count(), 1);
    assert_eq!(body.matches("Two").count(), 1);
    assert!(
        !ui.controls()
            .iter()
            .any(|c| c["label"] == "One" || c["label"] == "Two")
    );
    let stale_click = ui.event(choose, json!({}));
    assert_eq!(stale_click["refresh"], "none");
    ui.apply(stale_click);
    assert_eq!(
        ui.event(ui.control("Send", "button"), json!({}))["request"]["input"]["reply"],
        "question"
    );
    db.execute(
        "UPDATE questions SET state='answered' WHERE id='question'",
        [],
    )
    .unwrap();
    ui.render(true);
    assert_eq!(ui.control("Send", "button")["enabled"], false);
    assert!(
        ui.event(ui.control("Send", "button"), json!({}))
            .get("request")
            .is_none()
    );
    ui.click("New topic");
    assert_eq!(ui.control("Message", "multiline")["value"], "Two, please");
    assert_eq!(ui.control("Send", "button")["enabled"], true);
    db.execute("UPDATE questions SET state='open' WHERE id='question'", [])
        .unwrap();
    ui.render(true);
    ui.click("Reply");
    ui.click("Send");
    assert!(!ui.controls().iter().any(|c| c["label"] == "Reply"));
    assert!(ui.controls().iter().any(|c| {
        c["kind"] == "detail"
            && c["value"].as_str().is_some_and(|text| {
                text.contains("Which folder?") && text.contains("One") && text.contains("Two")
            })
    }));
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
fn a_draft_with_two_open_questions_waits_for_an_explicit_destination() {
    let mut ui = Desktop::open();
    let project = ui.view["project"].as_str().unwrap().to_owned();
    let request = ui.invoke(
        "submit",
        json!({"key":"start","text":"Review this project","attachments":[]}),
    );
    let db = rusqlite::Connection::open(ui._temp.path().join("state/state.sqlite3")).unwrap();
    for (id, question) in [("scope", "Which scope?"), ("build", "Which build?")] {
        let ask = json!({"question":question,"choices":["Recommended","Custom"]});
        db.execute("INSERT INTO questions(id,request,revision,session,role,body) VALUES(?,?,1,'session','planner',?)",
            (id, request["request"].as_str().unwrap(), ask.to_string())).unwrap();
        db.execute(
            "INSERT INTO events(project,kind,payload) VALUES(?,'ASK',?)",
            (
                &project,
                json!({"action":id,"request":request["request"],"payload":ask}).to_string(),
            ),
        )
        .unwrap();
    }
    ui.render(true);
    let picker = ui.control("Question to answer", "select");
    let choices = picker["choices"].as_array().unwrap();
    assert!(
        choices
            .iter()
            .any(|q| q["value"] == "scope" && q["label"] == "Which scope?")
    );
    assert!(
        choices
            .iter()
            .any(|q| q["value"] == "build" && q["label"] == "Which build?")
    );
    let question = ui
        .controls()
        .iter()
        .find(|c| c["value"] == "Which build?")
        .unwrap();
    let section = &question["section"];
    assert!(section.is_string());
    assert!(ui.controls().iter().any(|c| &c["section"] == section
        && c["label"] == "Recommended"
        && c["event"]["action"] == "choose_reply"));
    let options = ui
        .controls()
        .iter()
        .position(|c| c["label"] == "Request options")
        .unwrap();
    assert!(
        options
            > ui.controls()
                .iter()
                .position(|c| c["id"] == question["id"])
                .unwrap()
    );

    ui.type_message("The local build");
    assert_eq!(ui.control("Send", "button")["enabled"], false);
    assert!(
        ui.event(ui.control("Send", "button"), json!({}))
            .get("request")
            .is_none()
    );
    ui.change("Question to answer", "select", json!({"value":"build"}));
    assert_eq!(
        ui.control("Message", "multiline")["value"],
        "The local build"
    );
    assert_eq!(
        ui.control("Message", "multiline")["placeholder"],
        "Write your reply…"
    );
    let selected = ui.control("RelAI · Replying to this question", "detail");
    assert_eq!(selected["value"], "Which build?\n\nRecommended · Custom");
    assert_eq!(
        ui.controls()
            .iter()
            .filter(|c| c["label"] == "Reply")
            .count(),
        1
    );
    let routed = ui.event(ui.control("Send", "button"), json!({}));
    assert_eq!(routed["request"]["input"]["reply"], "build");

    ui.click("New topic");
    let fresh = ui.event(ui.control("Send", "button"), json!({}));
    assert_eq!(fresh["request"]["input"]["text"], "The local build");
    assert!(fresh["request"]["input"]["reply"].is_null());
}

#[test]
fn reopening_restores_the_selected_project_after_a_different_projects_last_message() {
    let mut ui = Desktop::empty();
    let first = ui
        .app
        .invoke("open_project", json!({"root":ui.root}))
        .unwrap()["id"]
        .clone();
    let other = ui.root.with_file_name("another-project");
    fs::create_dir(&other).unwrap();
    let second = ui
        .app
        .invoke("open_project", json!({"root":other}))
        .unwrap()["id"]
        .clone();
    ui.app
        .invoke(
            "submit",
            json!({"project":second,"key":"last","text":"Discuss this project","attachments":[]}),
        )
        .unwrap();
    ui.render(true);
    assert_eq!(ui.view["project"], second);
    ui.click("project");
    assert_eq!(ui.view["project"], first);

    let data = ui._temp.path().join("state");
    drop(ui.app);
    let mut reopened =
        App::open(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../app"), &data).unwrap();
    assert_eq!(
        reopened.invoke("dashboard", Value::Null).unwrap()["project"],
        first
    );
    assert_eq!(
        reopened
            .invoke("dashboard", json!({"project":second}))
            .unwrap()["project"],
        second
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
    ui.click("Open project");
    ui.click("Move tabs to top");
    ui.click("Move tabs to left");
    ui.change("Active project", "select", json!({"value":first}));
    let switch = ui.control("Active project", "select");
    assert_eq!(switch["choices"].as_array().unwrap().len(), 8);
    assert_eq!(switch["side"], true);
    ui.click("Tools");
    ui.click("Versions");
    ui.change("Active project", "select", json!({"value":last}));
    ui.type_message("Draft eight.");
    ui.click("Tools");
    assert_eq!(ui.control("Library", "button")["selected"], true);
    ui.change("Active project", "select", json!({"value":first}));
    assert_eq!(ui.control("Message", "multiline")["value"], "Draft one.");
    ui.click("Tools");
    assert_eq!(ui.control("Versions", "button")["selected"], true);
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
    ui.control("Results 13–24 of 512", "label");
    assert_eq!(
        ui.controls()
            .iter()
            .filter(|c| c["label"] == "Open target")
            .count(),
        12
    );
    ui.change(
        "Filter relationships",
        "input",
        json!({"value":"target-511"}),
    );
    ui.control("source → target-511 · call", "button");
    assert_eq!(
        ui.controls()
            .iter()
            .filter(|c| c["label"] == "Open target")
            .count(),
        1
    );
    ui.change(
        "Filter relationships",
        "input",
        json!({"value":"not-a-path"}),
    );
    ui.find(|c| c["id"] == "graph/no-edges");
    ui.state["layers"]["snapshot"] = json!("different-snapshot");
    ui.render(false);
    ui.find(|c| c["id"] == "graph/inspect-prompt");
    assert!(!ui.controls().iter().any(|c| c["id"] == "graph/coverage"));
    ui.state["relationshipFilter"] = json!("");
    ui.state["layers"] = json!({"snapshot":"new","coverage":{"indexed":2,"total":2},"edges":[{"source":"same","target":"target","name":"call"},{"source":"same","target":"target","name":"call"}],"changes":[]});
    ui.render(false);
    assert_eq!(
        ui.controls()
            .iter()
            .filter(|c| c["label"] == "Open target")
            .count(),
        2
    );
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
    ui.click("Open project");
    ui.click("Move tabs to top");
    ui.click("Move tabs to left");
    ui.click("project");
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
