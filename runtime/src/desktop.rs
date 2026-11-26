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

    fn render(&mut self, reload: bool, effect: &Value) -> Result<Value> {
        if reload {
            self.view = self.step("view", self.state["nav"].clone())?;
        }
        let mut frame = self.step("frame", json!({"state":self.state,"view":self.view}))?;
        frame["ack"] = json!(self.serial);
        frame["focus"] = effect["focus"].clone();
        self.frame = frame.clone();
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
        let request=self.step("file",json!({"project":self.view["project"],"after":after,"name":name,"extension":path.extension().and_then(|s|s.to_str()).unwrap_or("")}))?;
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
                        rfd::FileDialog::new().pick_files().unwrap_or_default()
                    };
                    let mut errors = Vec::new();
                    for path in paths {
                        if let Err(e) = self.capture(&effect["after"], &path) {
                            errors.push(format!(
                                "{}: {e:#}",
                                path.file_name().unwrap_or_default().to_string_lossy()
                            ));
                        }
                    }
                    if !errors.is_empty() {
                        self.state["status"] = json!(errors.join("\n"));
                    }
                    return Ok(Some(self.render(false, &Value::Null)?));
                }
                Some(_) => bail!("Unsupported desktop operation"),
                None => {}
            }
            return match effect["refresh"].as_str() {
                Some("none") => Ok(None),
                Some("data") => Ok(Some(self.render(true, &effect)?)),
                Some("view") => Ok(Some(self.render(false, &effect)?)),
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

pub(super) fn run(app: Arc<Mutex<App>>) -> Result<()> {
    let mut desktop = Desktop::new(app.clone());
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
        assert!(controls.iter().any(|c| c["label"] == "Remove manual.md"));
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
        let event = desktop.step("view", json!({}))?;
        assert!(event.to_string().contains("Submission"));
        assert!(!desktop.frame.to_string().contains("Remove manual.md"));
        Ok(())
    }
}
