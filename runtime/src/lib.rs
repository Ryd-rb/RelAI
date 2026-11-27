//! Generic application bindings for the portable FluxR interpreter.
mod application;
mod file_effect;
mod formats;
mod git;
mod git_operations;
mod git_pack;
pub mod network;
mod resources;
mod selection;
pub mod smart_http;
mod syntax;
pub mod transport;
mod ui_values;
mod watch;
mod worktrees;
use anyhow::{Context, Result, bail, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use fluxr::structured::{
    HostServices, Invocation, Limits, Module, SourceDocument, Value as Vm, Workspace,
};
use rusqlite::{
    Connection, params_from_iter,
    types::{Value as Cell, ValueRef},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::Mutex,
};

pub const LIMIT: usize = 16 * 1024 * 1024;
fn hex(b: &[u8]) -> String {
    format!("{:x}", Sha256::digest(b))
}
pub fn read(path: &Path) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut file = fs::File::open(path).with_context(|| path.display().to_string())?;
    ensure!(file.metadata()?.is_file(), "Expected regular file");
    let identifier = same_file::Handle::from_file(file.try_clone()?)?;
    check_file_handle(&file, path)?;
    let before = file.metadata()?;
    (&mut file).take(LIMIT as u64 + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= LIMIT, "File exceeds limit");
    let after = file.metadata()?;
    ensure!(
        before.modified()? == after.modified()?
            && after.len() == bytes.len() as u64
            && identifier == same_file::Handle::from_path(path)?,
        "File changed while reading"
    );
    Ok(bytes)
}
#[cfg(windows)]
fn check_file_handle(file: &fs::File, path: &Path) -> Result<()> {
    use std::os::windows::{ffi::OsStringExt, io::AsRawHandle};
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle, GetFinalPathNameByHandleW,
    };
    let handle = file.as_raw_handle().cast();
    let mut info = std::mem::MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::uninit();
    // The kernel fills this POD on success. The File owns the handle throughout.
    ensure!(
        unsafe { GetFileInformationByHandle(handle, info.as_mut_ptr()) } != 0,
        "File identity unavailable"
    );
    ensure!(
        unsafe { info.assume_init() }.nNumberOfLinks == 1,
        "Unexpected hard-linked file"
    );
    let mut buffer = vec![0u16; 32768];
    let n =
        unsafe { GetFinalPathNameByHandleW(handle, buffer.as_mut_ptr(), buffer.len() as u32, 0) }
            as usize;
    ensure!(n > 0 && n < buffer.len(), "Final file path unavailable");
    let actual = PathBuf::from(std::ffi::OsString::from_wide(&buffer[..n]));
    ensure!(
        actual == path.canonicalize()?,
        "File path changed while opening"
    );
    Ok(())
}

#[cfg(unix)]
fn check_file_handle(file: &fs::File, _: &Path) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    ensure!(file.metadata()?.nlink() == 1, "Unexpected hard-linked file");
    Ok(())
}

/// Read a file explicitly selected by the desktop user using the same identity,
/// alias and private-directory checks as other host-path reads.
pub fn selected_file(path: &Path, private: &Path) -> Result<Value> {
    ensure!(
        fs::metadata(path)?.len() <= 8 * 1024 * 1024,
        "Attachment exceeds 8 MiB"
    );
    let selected = selection::source(
        private.to_str().context("Invalid data path")?,
        path.to_str().context("Invalid file path")?,
        true,
        private,
    )?;
    ensure!(
        selected["kind"] == "file"
            && selected["size"]
                .as_u64()
                .is_some_and(|n| n <= 8 * 1024 * 1024),
        "Select a file of at most 8 MiB"
    );
    Ok(selected["base64"].clone())
}

pub fn text(v: &Value) -> Result<&str> {
    v.as_str().context("Expected text")
}
fn string(v: &Vm) -> Result<String> {
    let Vm::List(values) = v else {
        bail!("Expected UTF-8 bytes")
    };
    let bytes = values
        .iter()
        .map(|v| match v {
            Vm::U64(n) => Ok(u8::try_from(*n)?),
            _ => bail!("Invalid UTF-8 byte"),
        })
        .collect::<Result<Vec<u8>>>()?;
    Ok(String::from_utf8(bytes)?)
}
fn linked(m: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        m.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        m.is_symlink()
    }
}
fn relative(name: &str) -> Result<&Path> {
    let p = Path::new(name);
    ensure!(
        !name.contains([':', '\0']) && p.components().all(|c| matches!(c, Component::Normal(_))),
        "Expected contained relative path"
    );
    Ok(p)
}
fn join(root: &Path, name: &str) -> Result<PathBuf> {
    let mut path = root.to_owned();
    for c in relative(name)?.components() {
        path.push(c);
        match fs::symlink_metadata(&path) {
            Ok(m) => ensure!(!linked(&m), "Unexpected linked path"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(e.into()),
        }
    }
    Ok(path)
}
fn publish_file(path: &Path, data: &[u8]) -> Result<String> {
    ensure!(
        data.len() <= LIMIT,
        "Retained file exceeds publication limit"
    );
    let parent = path.parent().context("Invalid path")?;
    fs::create_dir_all(parent)?;
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    tmp.write_all(data)?;
    tmp.as_file().sync_all()?;
    tmp.persist(path)?;
    ensure!(read(path)? == data, "File verification failed");
    Ok(hex(data))
}

fn publish_checked(root: &Path, name: &str, data: &[u8], digest: &str) -> Result<()> {
    ensure!(hex(data) == digest, "Retained content digest mismatch");
    let path = join(root, name)?;
    if read(&path).is_ok_and(|bytes| bytes == data) {
        return Ok(());
    }
    ensure!(
        publish_file(&path, data)? == digest,
        "Publication digest mismatch"
    );
    Ok(())
}

pub fn fill(value: &Value, args: &[Value]) -> Result<Value> {
    Ok(match value {
        Value::String(s) if s.starts_with('$') && s[1..].parse::<usize>().is_ok() => args
            .get(s[1..].parse::<usize>()?)
            .context("Missing template argument")?
            .clone(),
        Value::Array(v) => Value::Array(v.iter().map(|v| fill(v, args)).collect::<Result<_>>()?),
        Value::Object(v) => Value::Object(
            v.iter()
                .map(|(k, v)| Ok((k.clone(), fill(v, args)?)))
                .collect::<Result<_>>()?,
        ),
        _ => value.clone(),
    })
}
/// Check the bounded inline schema vocabulary used by protocol and connector contracts.
pub fn validate(schema: &Value, value: &Value) -> Result<()> {
    let fields = schema.as_object().context("Expected an inline schema")?;
    for keyword in fields.keys() {
        ensure!(
            matches!(
                keyword.as_str(),
                "type"
                    | "enum"
                    | "anyOf"
                    | "properties"
                    | "required"
                    | "additionalProperties"
                    | "items"
                    | "minItems"
                    | "maxItems"
                    | "minLength"
                    | "maxLength"
                    | "description"
                    | "title"
            ),
            "Unsupported schema keyword {keyword}"
        );
    }
    if let Some(options) = fields.get("anyOf") {
        ensure!(
            options
                .as_array()
                .context("Expected schema alternatives")?
                .iter()
                .any(|s| validate(s, value).is_ok()),
            "No matching schema variant"
        );
    }
    if let Some(allowed) = fields.get("enum") {
        ensure!(
            allowed
                .as_array()
                .context("Expected enum values")?
                .contains(value),
            "Invalid enumerated value"
        );
    }
    let matches_type = |name: &Value| match name.as_str() {
        Some("null") => value.is_null(),
        Some("integer") => value.is_i64() || value.is_u64(),
        Some("number") => value.is_number(),
        Some("string") => value.is_string(),
        Some("boolean") => value.is_boolean(),
        Some("object") => value.is_object(),
        Some("array") => value.is_array(),
        _ => false,
    };
    if let Some(ty) = fields.get("type") {
        ensure!(
            matches_type(ty)
                || ty
                    .as_array()
                    .is_some_and(|options| options.iter().any(matches_type)),
            "Value does not match type {ty}"
        );
    }
    let bounds = |length: usize, min: &str, max: &str| -> Result<()> {
        if let Some(n) = fields.get(min) {
            ensure!(
                length as u64 >= n.as_u64().context("Invalid minimum length")?,
                "Value falls below {min}"
            )
        }
        if let Some(n) = fields.get(max) {
            ensure!(
                length as u64 <= n.as_u64().context("Invalid maximum length")?,
                "Value exceeds {max}"
            )
        }
        Ok(())
    };
    if let Some(content) = value.as_str() {
        bounds(content.chars().count(), "minLength", "maxLength")?
    }
    if let Some(items) = value.as_array() {
        bounds(items.len(), "minItems", "maxItems")?;
        if let Some(item) = fields.get("items") {
            for v in items {
                validate(item, v)?
            }
        }
    }
    if let Some(obj) = value.as_object() {
        if let Some(required) = fields.get("required") {
            for key in required
                .as_array()
                .context("Expected required property names")?
            {
                let key = text(key)?;
                ensure!(obj.contains_key(key), "Missing {key}");
            }
        }
        for (key, v) in obj {
            if let Some(rule) = schema["properties"].get(key) {
                validate(rule, v)?
            } else if let Some(extra) = fields.get("additionalProperties") {
                if let Some(allow) = extra.as_bool() {
                    ensure!(allow, "Unexpected property {key}")
                } else {
                    validate(extra, v)?
                }
            }
        }
    }
    Ok(())
}
fn evaluate(r: &Value) -> Result<Value> {
    let started = std::time::Instant::now();
    let src = text(&r["source"])?;
    ensure!(src.len() <= 65536, "Draft source too large");
    let doc = SourceDocument::parse(src)?;
    ensure!(
        !doc.has_native_items(),
        "Native code is not permitted in a checked module"
    );
    let m = doc.compile()?;
    ensure!(
        m.host_requirements()?.is_empty(),
        "Guest host imports are not permitted"
    );
    let export = text(&r["export"])?;
    let signature = m
        .export_signatures()
        .get(export)
        .context("Missing export")?
        .clone();
    let mut out = json!({"source_digest":hex(src.as_bytes()),"route":"portable","signature":signature,"canonical_source":m.unified_text()?,"parameters":signature.0,"result_type":signature.1,"input_types":m.input_bindings().iter().map(|b|&b.ty).collect::<Vec<_>>()});
    if r["compile_only"] != true {
        let args: Vec<Vm> = serde_json::from_value(r["arguments"].clone())?;
        let inputs: Vec<Vm> = serde_json::from_value(r["inputs"].clone())?;
        let result = m.run_portable_with(
            Invocation {
                export,
                inputs: &inputs,
                arguments: &args,
                limits: Limits {
                    instructions: 100_000,
                    cells: 262144,
                    frames: 64,
                },
            },
            &mut Workspace::default(),
        )?;
        out["value"] = serde_json::to_value(result)?;
    }
    out["elapsed_micros"] = json!(started.elapsed().as_micros() as u64);
    Ok(out)
}
pub struct App {
    modules: std::collections::BTreeMap<String, Module>,
    config: Value,
    assets: application::Assets,
    db: Connection,
    pub data: PathBuf,
    pub bundle: PathBuf,
    live: Mutex<watch::LiveSources>,
    _lock: fs::File,
}
impl App {
    pub fn open(bundle: &Path, data: &Path) -> Result<Self> {
        fs::create_dir_all(data)?;
        let data = data.canonicalize()?;
        let bundle = bundle.canonicalize()?;
        ensure!(
            !data.starts_with(&bundle) && !bundle.starts_with(&data),
            "Use separate bundle and state directories"
        );
        let lock = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(data.join("app.lock"))?;
        fs2::FileExt::try_lock_exclusive(&lock)
            .context("Another daemon owns the data directory")?;
        let config: Value = serde_json::from_slice(&read(&bundle.join("app.json"))?)?;
        let modules = application::link(&bundle, &config)?;
        let assets = application::Assets::load(&bundle, &config)?;
        let db = Connection::open(data.join("state.sqlite3"))?;
        db.create_scalar_function(
            "single_line",
            1,
            rusqlite::functions::FunctionFlags::SQLITE_UTF8
                | rusqlite::functions::FunctionFlags::SQLITE_DETERMINISTIC
                | rusqlite::functions::FunctionFlags::SQLITE_INNOCUOUS,
            |ctx| {
                Ok(ctx
                    .get::<String>(0)?
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" "))
            },
        )?;
        db.create_scalar_function(
            "unicode_fold",
            1,
            rusqlite::functions::FunctionFlags::SQLITE_UTF8
                | rusqlite::functions::FunctionFlags::SQLITE_DETERMINISTIC
                | rusqlite::functions::FunctionFlags::SQLITE_INNOCUOUS,
            |ctx| Ok(resources::fold(&ctx.get::<String>(0)?)),
        )?;
        db.execute_batch(
            "PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;",
        )?;
        let mut app = Self {
            modules,
            assets,
            config,
            db,
            bundle,
            data,
            live: Default::default(),
            _lock: lock,
        };
        app.invoke("init", Value::Null)?;
        for hook in app.config["startup_recovery"]
            .as_array()
            .cloned()
            .unwrap_or_default()
        {
            let mut cursor = Value::Null;
            loop {
                let page = app.invoke(text(&hook["page"])?, cursor.clone())?;
                let items = page.as_array().context("Invalid startup recovery page")?;
                if items.is_empty() {
                    break;
                }
                for item in items {
                    let id = text(&item["cursor"])?;
                    ensure!(
                        cursor.as_str().is_none_or(|previous| previous < id),
                        "Recovery cursor did not advance"
                    );
                    app.invoke(text(&hook["apply"])?, item.clone())?;
                    cursor = json!(id);
                }
            }
        }
        Ok(app)
    }
    pub fn change_count(&self) -> u64 {
        self.db.total_changes()
    }
    pub fn metadata(&self) -> &Value {
        &self.config
    }
    pub fn invoke(&mut self, name: &str, input: Value) -> Result<Value> {
        let mut value = self.enter(name, input)?;
        let mut origin = name.to_owned();
        for _ in 0..16 {
            if let Some(call) = value.get("$call") {
                let target = text(call)?;
                ensure!(
                    self.config["continuations"][&origin]["$call"]
                        .as_array()
                        .is_some_and(|allowed| allowed.contains(call)),
                    "Invalid application call"
                );
                origin = target.to_owned();
                value = self.enter(target, value["input"].clone())?;
                continue;
            }
            let kind = if value.get("$worktree").is_some() {
                "$worktree"
            } else if value.get("$git").is_some() {
                "$git"
            } else if value.get("$filesystem").is_some() {
                "$filesystem"
            } else if value.get("$evaluate").is_some() {
                "$evaluate"
            } else {
                return Ok(value);
            };
            let next = text(&value["resume"])?.to_owned();
            let allowed = &self.config["continuations"][&origin][kind];
            ensure!(
                allowed
                    .as_array()
                    .is_some_and(|set| set.contains(&json!(next)))
                    && self.modules.contains_key(&next),
                "Invalid continuation for {origin}"
            );
            let result = match kind {
                "$worktree" => worktrees::execute(&value[kind], &self.data),
                "$git" => git_operations::execute(&value[kind], &self.data),
                "$filesystem" => file_effect::execute(&value[kind], &self.data),
                _ => evaluate(&value[kind]),
            };
            origin.clone_from(&next);
            let reply = match result {
                Ok(v) => json!({"state":value["state"],"result":v,"error":null}),
                Err(e) => json!({"state":value["state"],"result":null,"error":format!("{e:#}")}),
            };
            value = self.enter(&next, reply)?;
        }
        bail!("Continuation limit reached")
    }
    fn enter(&mut self, name: &str, input: Value) -> Result<Value> {
        let mut arena = Arena {
            values: vec![Value::Null],
            bytes: 0,
        };
        let arg = arena.add(input)?;
        self.db.execute_batch("BEGIN IMMEDIATE")?;
        let executed = (|| -> Result<Value> {
            let host = Bindings {
                live: &self.live,
                arena: Mutex::new(arena),
                controls: Mutex::new(Vec::new()),
                db: Mutex::new(&mut self.db),
                data: &self.data,
                bundle: &self.bundle,
                assets: &self.assets,
                error: Mutex::new(None),
            };
            let result = self
                .modules
                .get(name)
                .context("Unknown application export")?
                .run_with_host(
                    Invocation {
                        export: name,
                        inputs: &[],
                        arguments: &[arg],
                        limits: Limits {
                            instructions: 2_000_000,
                            cells: 2_000_000,
                            frames: 64,
                        },
                    },
                    &mut Workspace::default(),
                    &host,
                );
            if let Some(err) = host.error.lock().unwrap().take() {
                bail!("{err}")
            }
            let result = host.arena.lock().unwrap().get(&result?)?.clone();
            Ok(result)
        })();
        match executed {
            Ok(v) => {
                self.db.execute_batch("COMMIT")?;
                Ok(v)
            }
            Err(e) => {
                self.db.execute_batch("ROLLBACK")?;
                Err(e)
            }
        }
    }
}
struct Arena {
    values: Vec<Value>,
    bytes: usize,
}
impl Arena {
    fn add(&mut self, value: Value) -> Result<Vm> {
        self.bytes += serde_json::to_vec(&value)?.len();
        ensure!(
            self.bytes <= 96 * 1024 * 1024 && self.values.len() < 32000,
            "Call arena limit"
        );
        let id = self.values.len();
        self.values.push(value);
        Ok(Vm::U64(id as u64))
    }
    fn get(&self, v: &Vm) -> Result<&Value> {
        let Vm::U64(id) = v else {
            bail!("Expected handle")
        };
        self.values.get(*id as usize).context("Unknown handle")
    }
}
struct Bindings<'a> {
    live: &'a Mutex<watch::LiveSources>,
    arena: Mutex<Arena>,
    controls: Mutex<Vec<Value>>,
    db: Mutex<&'a mut Connection>,
    data: &'a Path,
    bundle: &'a Path,
    assets: &'a application::Assets,
    error: Mutex<Option<String>>,
}
impl HostServices for Bindings<'_> {
    fn call(&self, name: &str, args: &[Vm]) -> std::result::Result<Vm, fluxr::Error> {
        match self.apply(name, args) {
            Ok(v) => Ok(v),
            Err(e) => {
                *self.error.lock().unwrap() = Some(format!("{name}: {e:#}"));
                Err(fluxr::Error::host(e.to_string()))
            }
        }
    }
}
fn json_string_bytes(text: &str) -> usize {
    2 + text
        .bytes()
        .map(|byte| match byte {
            b'"' | b'\\' | 8 | 9 | 10 | 12 | 13 => 2,
            0..=31 => 6,
            _ => 1,
        })
        .sum::<usize>()
}

fn seal_pressure(encoded: &str, is_text: bool, weight: usize) -> Result<usize> {
    let bytes = STANDARD.decode(encoded)?;
    let escaped = std::str::from_utf8(&bytes).map_or(0, json_string_bytes);
    let base = encoded.len();
    Ok(if is_text {
        4 * base + (18 + weight) * escaped
    } else {
        (20 + weight) * base + 2 * escaped
    })
}

impl Bindings<'_> {
    fn apply(&self, name: &str, args: &[Vm]) -> Result<Vm> {
        let mut arena = self.arena.lock().unwrap();
        let get = |i: usize| arena.get(args.get(i).context("Missing argument")?);
        let v = match name {
            "ui_emit" => {
                let widget = get(0)?;
                let mut controls = self.controls.lock().unwrap();
                ensure!(controls.len() < 2048, "Native control limit");
                controls.push(widget.clone());
                return Ok(Vm::U64(0));
            }
            "ui_collect" => Value::Array(std::mem::take(&mut *self.controls.lock().unwrap())),
            "merge_records" => ui_values::merged(get(0)?, get(1)?)?,
            "display_value" => match get(0)? {
                Value::String(s) => Value::String(s.clone()),
                v => json!(serde_json::to_string_pretty(v)?),
            },
            "edit_span" => ui_values::edit_span(text(get(0)?)?, text(get(1)?)?, get(2)?)?,
            "text" => json!(string(&args[0])?),
            "parse" => serde_json::from_str(text(get(0)?)?)?,
            "stringify" => json!(serde_json::to_string(get(0)?)?),
            "bytes" => {
                let b = text(get(0)?)?.as_bytes();
                ensure!(b.len() <= 131072, "Byte conversion limit");
                return Ok(Vm::List(std::sync::Arc::new(
                    b.iter().map(|v| Vm::U64(*v as u64)).collect(),
                )));
            }
            "source_path" => {
                selection::source(text(get(0)?)?, text(get(1)?)?, get(2)? == true, self.data)?
            }
            "byte_range" => selection::range(text(get(0)?)?, get(1)?)?,
            "regex_spans" => selection::regex_spans(text(get(0)?)?, text(get(1)?)?)?,
            "worktree_prepare" => {
                worktrees::prepare(Path::new(text(get(0)?)?), self.data, get(1)?)?
            }
            "worktree_inspect" => worktrees::inspect(Path::new(text(get(0)?)?), self.data)?,
            "validate_git_remote" => {
                smart_http::validate_remote(text(get(0)?)?, text(get(1)?)?)?;
                Value::Null
            }
            "git_sha1" => {
                let id = text(get(0)?)?;
                ensure!(
                    id.len() == 40 && id.bytes().all(|b| b.is_ascii_hexdigit()),
                    "Expected exactly 40 hexadecimal SHA-1 digits"
                );
                json!(id.to_ascii_lowercase())
            }
            "git_inspect" => git_operations::inspect(Path::new(text(get(0)?)?), self.data)?,
            "git_prepare" => {
                git_operations::prepare(Path::new(text(get(0)?)?), self.data, get(1)?)?
            }
            "effect_inspect" => file_effect::inspect(get(0)?, self.data)?,
            "patch_transform" => file_effect::patch(get(0)?)?,
            "prepare_chat_patch" => {
                file_effect::prepare_chat_patch(Path::new(text(get(0)?)?), get(1)?, self.data)?
            }
            "decode_format" => formats::decode(text(get(0)?)?, &STANDARD.decode(text(get(1)?)?)?)?,
            "encode_format" => {
                let format = text(get(0)?)?;
                let bytes = formats::encode(format, get(1)?)?;
                json!({"format":format,"base64":STANDARD.encode(&bytes),"digest":hex(&bytes),"size":bytes.len()})
            }
            "git_read" => git::read(
                Path::new(text(get(0)?)?),
                self.data,
                text(get(1)?)?,
                get(2)?,
            )?,
            "stage" => {
                let scope = text(get(0)?)?;
                ensure!(
                    scope.len() == 32 && scope.bytes().all(|b| b.is_ascii_hexdigit()),
                    "Invalid staging scope"
                );
                let base = join(self.data, &format!("staging/{scope}"))?;
                fs::create_dir_all(&base)?;
                let op = text(get(1)?)?;
                let arg = get(2)?;
                let name = text(&arg["path"])?;
                ensure!(!name.is_empty(), "Expected stage path");
                let src = join(&base, name)?;
                match op {
                    "mkdir" => fs::create_dir(&src)?,
                    "remove" => {
                        if src.is_dir() {
                            fs::remove_dir(&src)?
                        } else {
                            fs::remove_file(&src)?
                        }
                    }
                    "copy" | "move" => {
                        let dest = join(&base, text(&arg["destination"])?)?;
                        ensure!(src.is_file(), "Expected regular file");
                        let mut out = fs::OpenOptions::new()
                            .create_new(true)
                            .write(true)
                            .open(&dest)?;
                        out.write_all(&read(&src)?)?;
                        out.sync_all()?;
                        if op == "move" {
                            fs::remove_file(src)?
                        }
                    }
                    "create" => {
                        let bytes = STANDARD.decode(text(&arg["base64"])?)?;
                        ensure!(bytes.len() <= LIMIT, "Output too large");
                        let mut temp = tempfile::NamedTempFile::new_in(
                            src.parent().context("Missing parent")?,
                        )?;
                        temp.write_all(&bytes)?;
                        temp.as_file().sync_all()?;
                        temp.persist_noclobber(&src)?;
                    }
                    _ => bail!("Unknown staging operation"),
                };
                json!({"operation":op,"path":name})
            }
            "bundle" => resources::package(Path::new(text(get(0)?)?), self.data)?,
            "frontmatter" => resources::frontmatter(text(get(0)?)?)?,
            "matches" => resources::matches(text(get(0)?)?, get(1)?)?,
            "keys" => json!(
                get(0)?
                    .as_object()
                    .context("Expected object")?
                    .keys()
                    .collect::<Vec<_>>()
            ),
            "lower" => json!(text(get(0)?)?.to_lowercase()),
            "contains" => return Ok(Vm::Bool(text(get(0)?)?.contains(text(get(1)?)?))),
            "slice" => {
                let (Vm::I64(a), Vm::I64(b)) = (&args[1], &args[2]) else {
                    bail!("Expected byte offsets")
                };
                json!(
                    text(get(0)?)?
                        .get(usize::try_from(*a)?..usize::try_from(*b)?)
                        .context("Invalid byte span")?
                )
            }
            "pathjoin" => {
                let a = text(get(0)?)?;
                let b = text(get(1)?)?;
                relative(b)?;
                json!(if a.is_empty() {
                    b.into()
                } else {
                    format!("{a}/{b}")
                })
            }
            "validate_connector" => {
                network::validate_definition(get(0)?)?;
                Value::Null
            }
            "validate" => {
                validate(get(0)?, get(1)?)?;
                Value::Null
            }
            "base64" => json!(STANDARD.encode(text(get(0)?)?.as_bytes())),
            "int" => {
                let Vm::I64(n) = args[0] else {
                    bail!("Expected integer")
                };
                json!(n)
            }
            "number" => return Ok(Vm::I64(get(0)?.as_i64().context("Expected integer")?)),
            "same" => return Ok(Vm::Bool(get(0)? == get(1)?)),
            "truth" => return Ok(Vm::Bool(get(0)?.as_bool().context("Expected boolean")?)),
            "size" => {
                return Ok(Vm::I64(match get(0)? {
                    Value::String(s) => s.len(),
                    Value::Array(a) => a.len(),
                    Value::Object(m) => m.len(),
                    Value::Null => 0,
                    _ => bail!("Expected collection"),
                } as i64));
            }
            "get" => get(0)?.get(text(get(1)?)?).cloned().unwrap_or_default(),
            "at" => {
                let Vm::I64(n) = args[1] else {
                    bail!("Expected index")
                };
                get(0)?
                    .get(usize::try_from(n)?)
                    .cloned()
                    .unwrap_or_default()
            }
            "put" => {
                let mut v = get(0)?.clone();
                v.as_object_mut()
                    .context("Expected object")?
                    .insert(text(get(1)?)?.into(), get(2)?.clone());
                v
            }
            "append" => {
                let mut a = get(0)?.as_array().context("Expected list")?.clone();
                a.push(get(1)?.clone());
                json!(a)
            }
            "template" => fill(
                &serde_json::from_str(text(get(0)?)?)?,
                get(1)?.as_array().context("Expected arguments")?,
            )?,
            "concat" => json!(format!("{}{}", text(get(0)?)?, text(get(1)?)?)),
            "uuid" => json!(uuid::Uuid::new_v4().simple().to_string()),
            "hash" => json!(hex(text(get(0)?)?.as_bytes())),
            "require" => {
                ensure!(args[0] == Vm::Bool(true), "{}", text(get(1)?)?);
                Value::Null
            }
            "asset" => json!(self.assets.get(self.bundle, text(get(0)?)?)?),
            "sql" => {
                let conn = self.db.lock().unwrap();
                let sql = text(get(0)?)?;
                let params = get(1)?
                    .as_array()
                    .context("Expected SQL parameters")?
                    .iter()
                    .map(|v| {
                        Ok(match v {
                            Value::Null => Cell::Null,
                            Value::Number(n) => {
                                Cell::Integer(n.as_i64().context("Expected integer")?)
                            }
                            Value::Bool(b) => Cell::Integer(i64::from(*b)),
                            Value::String(s) => Cell::Text(s.clone()),
                            _ => Cell::Text(serde_json::to_string(v)?),
                        })
                    })
                    .collect::<Result<Vec<_>>>()?;
                let mut stmt = conn.prepare(sql)?;
                if stmt.column_count() == 0 {
                    json!(stmt.execute(params_from_iter(params))?)
                } else {
                    let names = stmt
                        .column_names()
                        .iter()
                        .map(|n| n.to_string())
                        .collect::<Vec<_>>();
                    let mut cursor = stmt.query(params_from_iter(params))?;
                    let mut out = Vec::new();
                    let mut size = 0;
                    while let Some(row) = cursor.next()? {
                        let mut item = json!({});
                        for (i, n) in names.iter().enumerate() {
                            let v = match row.get_ref(i)? {
                                ValueRef::Null => Value::Null,
                                ValueRef::Integer(n) => json!(n),
                                ValueRef::Real(n) => json!(n),
                                ValueRef::Text(b) => json!(std::str::from_utf8(b)?),
                                ValueRef::Blob(b) => json!(STANDARD.encode(b)),
                            };
                            if let Some(key) = n.strip_suffix(":json") {
                                item[key] = if v.is_null() {
                                    v
                                } else {
                                    serde_json::from_str(text(&v)?)?
                                }
                            } else {
                                item[n] = v
                            }
                        }
                        size += serde_json::to_vec(&item)?.len();
                        ensure!(
                            size <= LIMIT && out.len() < 10000,
                            "Query result exceeds limit"
                        );
                        out.push(item)
                    }
                    json!(out)
                }
            }
            "schema" => {
                self.db.lock().unwrap().execute_batch(text(get(0)?)?)?;
                Value::Null
            }
            "observe" => self
                .live
                .lock()
                .unwrap()
                .observe(Path::new(text(get(0)?)?))?,
            "cache_get" => self.live.lock().unwrap().get(text(get(0)?)?),
            "cache_put" => {
                self.live
                    .lock()
                    .unwrap()
                    .put(text(get(0)?)?.into(), get(1)?.clone())?;
                Value::Null
            }
            "syntax" => syntax::index(text(get(0)?)?, text(get(1)?)?)?,
            "graph_file_metadata" => syntax::graph_file_metadata(text(get(0)?)?, text(get(1)?)?),
            "directory_available" => {
                let path = Path::new(text(get(0)?)?);
                let available = path.canonicalize().is_ok_and(|actual| {
                    actual == path
                        && !actual.starts_with(self.data)
                        && !self.data.starts_with(&actual)
                        && fs::read_dir(&actual).is_ok_and(|mut entries| {
                            entries.next().is_none_or(|entry| entry.is_ok())
                        })
                });
                return Ok(Vm::Bool(available));
            }
            "check_attachment_pressure" => {
                let project = text(get(0)?)?;
                let files = get(1)?.as_array().context("Expected attachments")?;
                let conn = self.db.lock().unwrap();
                let mut statement =
                    conn.prepare("SELECT encoded FROM artifacts WHERE project=? AND id=?")?;
                let mut total = 0usize;
                for (index, file) in files.iter().enumerate() {
                    let mut rows = statement.query([project, text(&file["id"])?])?;
                    let row = rows.next()?.context("Unavailable attachment")?;
                    let encoded = row.get_ref(0)?.as_str()?;
                    total += seal_pressure(encoded, file["text"] == true, files.len() - index)?;
                    // Leave 32 MiB of the call arena for other request data. This
                    // estimates attachment pressure, not total sealing cost.
                    ensure!(
                        total <= 64 * 1024 * 1024,
                        "Attachments exceed the combined model delivery budget. Remove one or more files and try again."
                    );
                }
                Value::Null
            }
            "canonical" => {
                let p = Path::new(text(get(0)?)?).canonicalize()?;
                ensure!(
                    p.is_dir() && !p.starts_with(self.data) && !self.data.starts_with(&p),
                    "Select a directory separate from private storage"
                );
                json!(p)
            }
            "directory" | "read_file" | "read_base64" => {
                let root = Path::new(text(get(0)?)?).canonicalize()?;
                let path = join(&root, text(get(1)?)?)?.canonicalize()?;
                ensure!(
                    path.starts_with(&root) && !path.starts_with(self.data),
                    "Private or escaped path"
                );
                if name == "directory" {
                    let mut entries = Vec::new();
                    for entry in fs::read_dir(path)? {
                        let entry = entry?;
                        ensure!(entries.len() < 10000, "Directory limit");
                        let m = fs::symlink_metadata(entry.path())?;
                        entries.push(json!({"name":entry.file_name().to_str().context("Invalid filename")?,"directory":m.is_dir(),"link":linked(&m),"size":m.len()}))
                    }
                    entries.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
                    json!(entries)
                } else {
                    let before = fs::metadata(&path)?;
                    let bytes = read(&path)?;
                    let after = fs::metadata(&path)?;
                    ensure!(
                        before.modified()? == after.modified()?
                            && before.len() == after.len()
                            && before.len() == bytes.len() as u64,
                        "File changed while reading"
                    );
                    if name == "read_base64" {
                        json!(STANDARD.encode(bytes))
                    } else {
                        json!(String::from_utf8(bytes)?)
                    }
                }
            }
            "ancestors" => {
                let root = Path::new(text(get(0)?)?).canonicalize()?;
                let mut dirs = std::collections::BTreeSet::from([(0, String::new())]);
                for target in get(1)?.as_array().into_iter().flatten() {
                    let rel = text(target)?;
                    let path = join(&root, rel)?;
                    let dir = if path.is_dir() {
                        path.as_path()
                    } else {
                        path.parent().context("Invalid target")?
                    };
                    for prefix in dir.strip_prefix(&root)?.ancestors() {
                        if join(&root, prefix.to_str().context("Non-Unicode path")?)?.is_dir() {
                            dirs.insert((
                                prefix.components().count(),
                                prefix.to_string_lossy().replace('\\', "/"),
                            ));
                        }
                    }
                }
                json!(dirs.into_iter().map(|(_, s)| s).collect::<Vec<_>>())
            }
            "find_names" => {
                let root = Path::new(text(get(0)?)?).canonicalize()?;
                let dir = join(&root, text(get(1)?)?)?;
                ensure!(!dir.starts_with(self.data), "Private directory");
                let mut items = Vec::new();
                for n in get(2)?.as_array().context("Expected names")? {
                    let mut found = None;
                    for entry in fs::read_dir(&dir)? {
                        let entry = entry?;
                        if entry.file_name().eq_ignore_ascii_case(text(n)?) {
                            ensure!(found.is_none(), "Ambiguous file name");
                            let path = entry.path();
                            ensure!(!linked(&fs::symlink_metadata(&path)?), "Unexpected link");
                            let data = read(&path)?;
                            ensure!(data.len() <= 256 * 1024, "Instruction exceeds limit");
                            found = Some(
                                json!({"path":path.strip_prefix(&root)?,"directory_scope":dir.strip_prefix(&root)?,"digest":hex(&data),"text":String::from_utf8(data)?}),
                            );
                        }
                    }
                    if let Some(item) = found {
                        items.push(item)
                    }
                }
                json!(items)
            }
            "file_valid" => {
                let p = join(self.data, text(get(0)?)?)?;
                return Ok(Vm::Bool(match read(&p) {
                    Ok(b) => hex(&b) == text(get(1)?)?,
                    Err(_) => false,
                }));
            }
            "publish" => {
                let p = join(self.data, text(get(0)?)?)?;
                json!(publish_file(&p, text(get(1)?)?.as_bytes())?)
            }
            "publish_checked" => {
                match publish_checked(
                    self.data,
                    text(get(0)?)?,
                    text(get(1)?)?.as_bytes(),
                    text(get(2)?)?,
                ) {
                    Ok(()) => Value::Null,
                    Err(error) => json!(format!("{error:#}").chars().take(512).collect::<String>()),
                }
            }
            "base64_info" => {
                let encoded = text(get(0)?)?;
                let bytes = STANDARD.decode(encoded)?;
                ensure!(bytes.len() <= 8 * 1024 * 1024, "Attachment exceeds limit");
                json!({"size":bytes.len(),"digest":hex(&bytes),"text":std::str::from_utf8(&bytes).ok()})
            }
            _ => bail!("Unknown host service {name}"),
        };
        arena.add(v)
    }
}
