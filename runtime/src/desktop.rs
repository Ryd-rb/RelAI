//! Channel adapter between checked application exports and the native window thread.
use super::{App, invoke, work};
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};
use tokio::runtime::Handle;

struct Desktop {
    app: Arc<Mutex<App>>,
    runtime: Handle,
    exports: Value,
    private: PathBuf,
    state: Value,
    view: Value,
    frame: Value,
    serial: u64,
}

impl Desktop {
    fn new(app: Arc<Mutex<App>>) -> Self {
        let (exports, private) = {
            let a = app.lock().unwrap();
            (a.metadata()["desktop"].clone(), a.data.clone())
        };
        Self {
            app,
            runtime: Handle::current(),
            exports,
            private,
            state: json!({}),
            view: Value::Null,
            frame: Value::Null,
            serial: 0,
        }
    }
    fn step(&self, stage: &str, input: Value) -> Result<Value> {
        let export = self.exports[stage]
            .as_str()
            .context("Missing desktop export")?;
        self.app.lock().unwrap().invoke(export, input)
    }

    fn describe(&mut self, reload: bool) -> Result<Value> {
        if reload {
            self.view = self.step("view", self.state["nav"].clone())?;
        }
        let mut frame = self.step("frame", json!({"state":self.state,"view":self.view}))?;
        frame["ack"] = json!(self.serial);
        Ok(frame)
    }

    fn render(&mut self, reload: bool, effect: &Value) -> Result<Value> {
        let frame = self.describe(reload)?;
        self.present(frame, effect)
    }

    fn present(&mut self, mut frame: Value, effect: &Value) -> Result<Value> {
        let focus = if self.frame.is_null() && effect["focus"].is_null() {
            frame["composer"].clone()
        } else {
            effect["focus"].clone()
        };
        self.frame = frame.clone();
        frame["focus"] = focus;
        if let Some(range) = effect.get("selection") {
            let control = frame["controls"]
                .as_array()
                .and_then(|cs| cs.iter().find(|c| c["id"] == range["id"]))
                .context("Missing selection target")?;
            let body = control["value"]
                .as_str()
                .context("Expected editable text")?;
            let native_offset = |key: &str| -> Result<usize> {
                let byte =
                    usize::try_from(range[key].as_u64().context("Invalid selection offset")?)?;
                let prefix = body
                    .get(..byte)
                    .context("Invalid UTF-8 selection boundary")?;
                Ok(prefix.encode_utf16().count() + prefix.bytes().filter(|b| *b == b'\n').count())
            };
            frame["selection"] = json!({"id":range["id"],"start":native_offset("start")?,"end":native_offset("end")?});
        }
        // Keep private action descriptors in the controller. Native controls receive
        // only their presentation and Windows-compatible line endings.
        for c in frame["controls"]
            .as_array_mut()
            .context("Expected native controls")?
        {
            c.as_object_mut()
                .context("Expected native control")?
                .remove("event");
            if matches!(c["kind"].as_str(), Some("multiline" | "detail"))
                && let Some(s) = c["value"].as_str()
            {
                c["value"] = json!(s.replace("\r\n", "\n").replace('\n', "\r\n"));
            }
        }
        Ok(frame)
    }

    fn error_frame(&mut self, error: anyhow::Error) -> Value {
        let message = format!("{error:#}");
        self.state["status"] = json!(message);
        match self.render(false, &Value::Null) {
            Ok(frame) => frame,
            Err(render_error) => {
                eprintln!("Desktop render: {render_error:#}");
                let detail = format!("{message}\n{render_error:#}");
                let mut fallback = json!({
                    "title": "Application error",
                    "controls": [{"id":"error", "kind":"detail", "value":detail}]
                });
                if let Some(theme) = self.frame.get("theme") {
                    fallback["theme"] = theme.clone();
                }
                fallback
            }
        }
    }

    fn action(&self, op: &Value, args: Value) -> Result<Value> {
        let name = op.as_str().context("Missing action name")?;
        ensure!(
            self.app.lock().unwrap().metadata()["public"]
                .as_array()
                .is_some_and(|ops| ops.contains(op)),
            "Unavailable action"
        );
        self.runtime.block_on(invoke(self.app.clone(), name, args))
    }

    fn result(&self, after: &Value, result: Value) -> Result<Value> {
        self.step("result",json!({"state":self.state,"project":self.view["project"],"after":after,"result":result}))
    }

    fn capture(&mut self, after: &Value, path: &Path) -> Result<()> {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .context("Expected a file name")?;
        let request=self.step("file",json!({"project":self.view["project"],"state":self.state,"after":after,"name":name,"extension":path.extension().and_then(|s|s.to_str()).unwrap_or("")}))?;
        let mut args = request["input"].clone();
        args["base64"] = fluxr_app_host::selected_file(path, &self.private)?;
        let receipt = self.action(&request["op"], args)?;
        self.state = self.result(&request["after"], receipt)?["state"].clone();
        Ok(())
    }

    fn process(&mut self, mut input: Value) -> Result<Option<Value>> {
        self.serial = self.serial.max(input["serial"].as_u64().unwrap_or(0));
        let kind = input["type"].as_str().unwrap_or("").to_owned();
        let id = if kind == "drop" {
            self.frame["drop"].as_str()
        } else {
            input["id"].as_str()
        };
        let event = if kind == "refresh" {
            json!({"action":"refresh"})
        } else {
            let Some(id) = id else { return Ok(None) };
            let Some(control) = self.frame["controls"]
                .as_array()
                .and_then(|cs| cs.iter().find(|c| c["id"] == id))
            else {
                return Ok(None);
            };
            if control["enabled"] == false || control["event"].is_null() {
                return Ok(None);
            }
            if matches!(control["kind"].as_str(), Some("input" | "multiline"))
                && (kind != "change" || !input["value"].is_string())
            {
                return Ok(None);
            }
            if control["kind"] == "multiline" {
                normalize_edit(&mut input);
            }
            control["event"].clone()
        };
        let mut effect = self.step(
            "event",
            json!({"state":self.state,"event":event,"project":self.view["project"],"input":input}),
        )?;
        for _ in 0..8 {
            self.state = effect["state"].clone();
            if let Some(request) = effect.get("request") {
                let result = self.action(&request["op"], request["input"].clone())?;
                effect = self.result(&request["after"], result)?;
                continue;
            }
            match effect["platform"].as_str() {
                Some("folder") => {
                    let path = rfd::FileDialog::new().pick_folder();
                    effect = self.result(&effect["after"], json!({"path":path}))?;
                    continue;
                }
                Some("files") => {
                    let paths: Vec<PathBuf> = if kind == "drop" {
                        serde_json::from_value(input["paths"].clone())?
                    } else {
                        let mut dialog = rfd::FileDialog::new();
                        if let Some(root) = self.view["subtitle"]
                            .as_str()
                            .map(Path::new)
                            .filter(|p| p.is_dir())
                        {
                            dialog = dialog.set_directory(root);
                        }
                        dialog.pick_files().unwrap_or_default()
                    };
                    let total = paths.len();
                    let mut added = 0usize;
                    let mut errors = Vec::new();
                    for path in paths {
                        if let Err(e) = self.capture(&effect["after"], &path) {
                            errors.push(format!(
                                "{}: {e:#}",
                                path.file_name().unwrap_or_default().to_string_lossy()
                            ));
                        } else {
                            added += 1;
                        }
                    }
                    if !errors.is_empty() {
                        self.state["status"] = json!(format!(
                            "Added {added} of {total} selected files; {} failed. Successful attachments remain in the draft.\n{}",
                            errors.len(),
                            errors.join("\n")
                        ));
                    }
                    return Ok(Some(self.render(false, &Value::Null)?));
                }
                Some(_) => bail!("Unsupported desktop operation"),
                None => {}
            }
            return match effect["refresh"].as_str() {
                Some("none") => Ok(None),
                Some(refresh @ ("data" | "view")) => {
                    let frame = self.describe(refresh == "data")?;
                    if kind == "refresh"
                        && frame == self.frame
                        && effect["focus"].is_null()
                        && effect["selection"].is_null()
                    {
                        return Ok(None);
                    }
                    self.present(frame, &effect).map(Some)
                }
                _ => bail!("Unknown desktop response"),
            };
        }
        bail!("Desktop continuation limit")
    }
}

/// Native edit controls count CRLF as two UTF-16 units. FluxR stores LF text.
fn normalize_edit(input: &mut Value) {
    let Some(raw) = input["value"].as_str() else {
        return;
    };
    if !raw.contains("\r\n") {
        return;
    }
    let raw = raw.to_owned();
    for key in ["start", "end"] {
        if let Some(pos) = input[key].as_u64() {
            let mut at = 0;
            let mut removed = 0;
            let mut previous_cr = false;
            for c in raw.chars() {
                if at >= pos {
                    break;
                }
                at += c.len_utf16() as u64;
                if c == '\n' && previous_cr {
                    removed += 1;
                }
                previous_cr = c == '\r';
            }
            input[key] = json!(pos.saturating_sub(removed));
        }
    }
    input["value"] = json!(raw.replace("\r\n", "\n"));
}

pub(super) fn run(app: Arc<Mutex<App>>, view: Option<&str>) -> Result<()> {
    let mut desktop = Desktop::new(app.clone());
    if let Some(view) = view {
        anyhow::ensure!(
            desktop.exports["views"]
                .as_array()
                .is_some_and(|views| views.iter().any(|value| value == view)),
            "Unknown native view {view}"
        );
        desktop.state = json!({"nav":{"view":view}});
    }
    let initial = desktop.render(true, &Value::Null)?;
    let title = initial["title"].as_str().unwrap_or("FluxR").to_owned();
    let (events, rx) = mpsc::channel::<Value>();
    let (frames, rx_frames) = mpsc::channel();
    let ending = events.clone();
    let worker = tokio::spawn(work(app, Some(events.clone())));
    let closed = Arc::new(AtomicBool::new(false));
    let closing = closed.clone();
    std::thread::Builder::new()
        .name("FluxR desktop controller".into())
        .spawn(move || {
            for event in rx {
                if closing.load(Ordering::Acquire) || event["type"] == "shutdown" {
                    break;
                }
                let frame = match desktop.process(event) {
                    Ok(Some(v)) => v,
                    Ok(None) => continue,
                    Err(error) => desktop.error_frame(error),
                };
                if frames.send(frame).is_err() {
                    break;
                }
            }
        })?;
    let result = fluxr_win32::run(&title, initial, events, rx_frames);
    closed.store(true, Ordering::Release);
    worker.abort();
    let _ = ending.send(json!({"type":"shutdown"}));
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn normalized_caret_counts_utf16_units_with_newlines_and_emoji() {
        let mut e = json!({"value":"🙂\r\na\r\nb","start":4,"end":8});
        normalize_edit(&mut e);
        assert_eq!(e, json!({"value":"🙂\na\nb","start":3,"end":6}));
    }

    #[test]
    fn background_activity_only_sends_a_frame_when_the_chat_changes() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let project = temp.path().join("project");
        std::fs::create_dir(&project)?;
        let data = temp.path().join("data");
        let mut app = App::open(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../app"), &data)?;
        let pid = app.invoke("open_project", json!({"root":project}))?["id"].clone();
        let runtime = tokio::runtime::Runtime::new()?;
        let _entered = runtime.enter();
        let mut desktop = Desktop::new(Arc::new(Mutex::new(app)));
        let initial = desktop.render(true, &Value::Null)?;
        assert!(initial["composer"].is_string());
        assert_eq!(initial["focus"], initial["composer"]);
        let db = rusqlite::Connection::open(data.join("state.sqlite3"))?;
        db.execute(
            "INSERT INTO events(project,kind,payload) VALUES(?,'Delivery','{}')",
            [pid.as_str().unwrap()],
        )?;
        let refresh = json!({"type":"refresh"});
        assert!(
            desktop.process(refresh.clone())?.is_none(),
            "Internal activity sent an unchanged native frame"
        );
        desktop.app.lock().unwrap().invoke(
            "submit",
            json!({"project":pid,"key":"visible","text":"New message","attachments":[]}),
        )?;
        let frame = desktop
            .process(refresh.clone())?
            .expect("A new message needs a native frame");
        assert!(
            frame["controls"]
                .as_array()
                .unwrap()
                .iter()
                .any(|c| c["role"] == "outgoing" && c["value"] == "New message")
        );
        assert!(desktop.process(refresh)?.is_none());
        Ok(())
    }

    #[test]
    fn native_drop_captures_only_selected_bytes_and_acknowledges_input() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let project = temp.path().join("project");
        std::fs::create_dir(&project)?;
        let data = temp.path().join("data");
        let mut app = App::open(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../app"), &data)?;
        app.invoke("open_project", json!({"root":project}))?;
        let runtime = tokio::runtime::Runtime::new()?;
        let _entered = runtime.enter();
        let mut desktop = Desktop::new(Arc::new(Mutex::new(app)));
        let frame = desktop.render(true, &Value::Null)?;
        let composer = frame["controls"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["kind"] == "multiline" && c["label"] == "Message")
            .unwrap()["id"]
            .clone();
        desktop.process(json!({"id":composer,"type":"change","value":"🙂\r\nhello.","start":10,"end":10,"serial":4}))?;
        let typed = desktop.state.clone();
        for notification in [
            json!({"id":composer,"type":"focus"}),
            json!({"id":composer,"type":"click","value":"unrelated caption"}),
            json!({"id":composer,"type":"change"}),
        ] {
            assert!(desktop.process(notification)?.is_none());
            assert_eq!(
                desktop.state, typed,
                "Only text changes may replace the draft"
            );
        }
        let attachment = project.join("manual.md");
        std::fs::write(&attachment, "captured once")?;
        let private = data.join("private.txt");
        std::fs::write(&private, "private")?;
        let frame = desktop
            .process(json!({"type":"drop","paths":[attachment,private]}))?
            .unwrap();
        assert_eq!(frame["ack"], 4);
        let controls = frame["controls"].as_array().unwrap();
        assert!(
            controls
                .iter()
                .any(|c| c["id"] == composer && c["value"] == "🙂\r\nhello.")
        );
        assert!(
            controls
                .iter()
                .any(|c| c["id"] == "status" && c["area"] == "composer")
        );
        assert!(
            controls
                .iter()
                .any(|c| c["label"] == "Remove manual.md · 13 bytes")
        );
        assert!(
            desktop.state["status"]
                .as_str()
                .unwrap()
                .contains("Added 1 of 2")
        );
        assert!(!controls.iter().any(|c| c["label"] == "Remove private.txt"));
        assert!(
            desktop.state["status"]
                .as_str()
                .unwrap()
                .contains("private.txt")
        );
        fluxr_win32::validate_frame(&frame)?;
        let submit = desktop.frame["default"].clone();
        desktop.process(json!({"id":submit,"type":"click","serial":5}))?;
        let transcript = desktop.frame["controls"].as_array().unwrap();
        assert!(
            transcript.iter().any(|c| c["role"] == "outgoing"
                && c["value"] == "🙂\nhello.\n\nAttached files\nmanual.md")
        );
        assert!(!desktop.frame.to_string().contains("Remove manual.md"));

        // Enter can arrive before the native window sees the first edit's enabled button.
        desktop.process(
            json!({"id":composer,"type":"change","value":"X","start":1,"end":1,"serial":6}),
        )?;
        let enter = json!({"id":submit,"type":"click","pending_edit":6,"serial":7});
        let sent = desktop
            .process(enter.clone())?
            .expect("Pending Enter should send");
        assert_eq!(
            sent["controls"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|c| c["role"] == "outgoing" && c["value"] == "X")
                .count(),
            1
        );
        assert!(desktop.process(enter)?.is_none());
        Ok(())
    }
}
