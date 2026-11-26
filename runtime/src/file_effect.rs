//! Generic guarded filesystem transactions. Application grants, project scope,
//! effect journals and recovery decisions live in checked FluxR.
use anyhow::{Context, Result, bail, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Seek, SeekFrom, Write},
    path::{Component, Path, PathBuf},
};

const LIMIT: usize = 8 * 1024 * 1024;
fn text(value: &Value) -> Result<&str> {
    value.as_str().context("Expected text")
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub fn patch(request: &Value) -> Result<Value> {
    let base = STANDARD.decode(text(&request["base64"])?)?;
    ensure!(base.len() <= LIMIT, "Patch base exceeds limit");
    let patch = text(&request["patch"])?;
    ensure!(patch.len() <= 65536, "Patch source exceeds limit");
    let old = std::str::from_utf8(&base)?;
    let parsed = diffy::Patch::from_str(patch)?;
    let updated = diffy::apply(old, &parsed)?;
    ensure!(updated.len() <= LIMIT, "Patch output exceeds limit");
    let preview = diffy::create_patch(old, &updated).to_string();
    ensure!(preview.len() <= LIMIT, "Patch preview exceeds limit");
    Ok(
        json!({"base64":STANDARD.encode(updated.as_bytes()),"digest":digest(updated.as_bytes()),"base_digest":digest(&base),"size":updated.len(),"preview":preview}),
    )
}

fn compatible(actual: &Value, expected: &Value) -> bool {
    if actual["kind"] != expected["kind"] {
        return false;
    }
    ["digest", "file_id", "empty"].iter().all(|key| {
        expected
            .get(*key)
            .is_none_or(|v| actual.get(*key) == Some(v))
    })
}
fn operations(request: &Value) -> Result<Vec<&Value>> {
    if request["operation"] == "batch" {
        let operations = request["operations"]
            .as_array()
            .context("Missing batch operations")?;
        ensure!(
            !operations.is_empty() && operations.len() <= 64,
            "Batch size limit"
        );
        ensure!(
            operations.iter().all(|op| op["operation"] == "write"),
            "Batches admit guarded writes only"
        );
        Ok(operations.iter().collect())
    } else {
        Ok(vec![request])
    }
}

#[cfg(not(windows))]
pub fn inspect(_request: &Value, _private: &Path) -> Result<Value> {
    bail!("Unsupported platform: protected file effects require Windows TxF")
}
#[cfg(not(windows))]
pub fn execute(_request: &Value, _private: &Path) -> Result<Value> {
    Ok(json!({"state":"unsupported","error":"Protected file effects require Windows TxF"}))
}

#[cfg(windows)]
mod windows {
    use super::*;
    use std::{
        fs::{File, OpenOptions},
        os::windows::{
            ffi::OsStrExt,
            fs::OpenOptionsExt,
            io::{AsRawHandle, FromRawHandle},
        },
        ptr,
    };
    use windows_sys::Win32::{
        Foundation::{CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE},
        Storage::FileSystem::*,
    };

    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().chain(Some(0)).collect()
    }
    fn os_error(label: &str) -> anyhow::Error {
        anyhow::anyhow!("{label}: {}", std::io::Error::last_os_error())
    }
    fn info(file: &File) -> Result<BY_HANDLE_FILE_INFORMATION> {
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        ensure!(
            unsafe { GetFileInformationByHandle(file.as_raw_handle() as HANDLE, &mut info) } != 0,
            "{}",
            os_error("File identity")
        );
        ensure!(
            info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT == 0,
            "Unexpected link or reparse point"
        );
        if info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY == 0 {
            ensure!(
                info.nNumberOfLinks == 1,
                "Multiple hard links are not admitted for live effects"
            );
        }
        Ok(info)
    }
    struct Guarded {
        path: PathBuf,
        _parents: Vec<File>,
    }
    fn guarded(path: &Path, private: &Path) -> Result<Guarded> {
        ensure!(path.is_absolute(), "Effect paths must be absolute");
        for part in path.components() {
            match part {
                Component::Prefix(prefix) => ensure!(
                    matches!(
                        prefix.kind(),
                        std::path::Prefix::Disk(_) | std::path::Prefix::VerbatimDisk(_)
                    ),
                    "Only local drive paths are supported"
                ),
                Component::RootDir => {}
                Component::Normal(value) => {
                    let value = value.to_str().context("Expected Unicode path")?;
                    ensure!(
                        !value.contains([':', '\0']) && !value.ends_with(['.', ' ']),
                        "Ambiguous path component"
                    );
                    let stem = value.split('.').next().unwrap_or("").to_ascii_uppercase();
                    ensure!(
                        !matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
                            && !(stem.len() == 4
                                && (stem.starts_with("COM") || stem.starts_with("LPT"))
                                && matches!(stem.as_bytes()[3], b'1'..=b'9')),
                        "Reserved path component"
                    );
                }
                _ => bail!("Relative path components are forbidden"),
            }
        }
        let parent = path
            .parent()
            .context("Volume roots are not effect targets")?;
        let name = path.file_name().context("Missing final path component")?;
        let ancestors: Vec<_> = parent.ancestors().collect();
        ensure!(ancestors.len() <= 128, "Path depth limit");
        let mut parents = Vec::new();
        for ancestor in ancestors.into_iter().rev() {
            let file = OpenOptions::new()
                .access_mode(FILE_READ_ATTRIBUTES)
                .share_mode(FILE_SHARE_READ)
                .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
                .open(ancestor)
                .with_context(|| format!("Cannot protect ancestor {}", ancestor.display()))?;
            let metadata = info(&file)?;
            ensure!(
                metadata.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0,
                "Ancestor is not a directory"
            );
            parents.push(file);
        }
        let parent = parent.canonicalize()?;
        let private = private.canonicalize()?;
        let path = parent.join(name);
        ensure!(
            !path.starts_with(&private),
            "Private daemon storage is excluded from generic effects"
        );
        Ok(Guarded {
            path,
            _parents: parents,
        })
    }
    fn checkout(path: &Path) -> Option<PathBuf> {
        path.parent()?
            .ancestors()
            .find(|p| fs::symlink_metadata(p.join(".git")).is_ok())
            .map(Path::to_path_buf)
    }
    fn plain_open(path: &Path) -> Result<Option<File>> {
        match OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)
        {
            Ok(file) => Ok(Some(file)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
    fn state(path: &Path, file: Option<&mut File>) -> Result<Value> {
        let Some(file) = file else {
            return Ok(json!({"kind":"absent"}));
        };
        let identity = info(file)?;
        let id = format!(
            "{:08x}:{:08x}{:08x}",
            identity.dwVolumeSerialNumber, identity.nFileIndexHigh, identity.nFileIndexLow
        );
        if identity.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
            return Ok(
                json!({"kind":"directory","empty":fs::read_dir(path)?.next().is_none(),"file_id":id}),
            );
        }
        ensure!(
            file.metadata()?.len() <= LIMIT as u64,
            "File exceeds effect byte limit"
        );
        file.seek(SeekFrom::Start(0))?;
        let mut bytes = Vec::new();
        file.take(LIMIT as u64 + 1).read_to_end(&mut bytes)?;
        ensure!(bytes.len() <= LIMIT, "File exceeds effect byte limit");
        Ok(json!({"kind":"file","digest":digest(&bytes),"bytes":bytes.len(),"file_id":id}))
    }
    pub(super) fn inspect(request: &Value, private: &Path) -> Result<Value> {
        let mut output = Vec::new();
        for op in operations(request)? {
            let target = guarded(Path::new(text(&op["path"])?), private)?;
            let mut file = plain_open(&target.path)?;
            let observed = state(&target.path, file.as_mut())?;
            let mut item =
                json!({"path":target.path,"state":observed,"checkout":checkout(&target.path)});
            if matches!(op["operation"].as_str(), Some("copy" | "move")) {
                let source = guarded(Path::new(text(&op["source"])?), private)?;
                let mut file = plain_open(&source.path)?;
                item["source"] = json!(source.path);
                item["source_state"] = state(&source.path, file.as_mut())?;
                item["source_checkout"] = json!(checkout(&source.path));
            }
            output.push(item);
        }
        if request["operation"] == "batch" {
            Ok(json!({"operations":output}))
        } else {
            Ok(output.remove(0))
        }
    }
    struct Transaction {
        handle: HANDLE,
        committed: bool,
    }
    impl Transaction {
        fn new() -> Result<Self> {
            let description: Vec<u16> = "Guarded filesystem effect\0".encode_utf16().collect();
            let handle = unsafe {
                CreateTransaction(
                    ptr::null_mut(),
                    ptr::null_mut(),
                    0,
                    0,
                    0,
                    15000,
                    description.as_ptr(),
                )
            };
            ensure!(
                handle != INVALID_HANDLE_VALUE && !handle.is_null(),
                "unsupported: {}",
                os_error("CreateTransaction")
            );
            Ok(Self {
                handle,
                committed: false,
            })
        }
        fn open(&self, path: &Path, access: u32, disposition: u32) -> Result<Option<File>> {
            let name = wide(path);
            let handle = unsafe {
                CreateFileTransactedW(
                    name.as_ptr(),
                    access,
                    FILE_SHARE_READ,
                    ptr::null(),
                    disposition,
                    FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS,
                    ptr::null_mut(),
                    self.handle,
                    ptr::null(),
                    ptr::null(),
                )
            };
            if handle == INVALID_HANDLE_VALUE {
                let e = std::io::Error::last_os_error();
                if disposition == OPEN_EXISTING && e.kind() == std::io::ErrorKind::NotFound {
                    return Ok(None);
                }
                return Err(anyhow::anyhow!("Transacted open: {e}"));
            }
            Ok(Some(unsafe { File::from_raw_handle(handle) }))
        }
        fn commit(&mut self) -> Result<()> {
            ensure!(
                unsafe { CommitTransaction(self.handle) } != 0,
                "uncertain: {}",
                os_error("CommitTransaction")
            );
            self.committed = true;
            Ok(())
        }
    }
    impl Drop for Transaction {
        fn drop(&mut self) {
            unsafe {
                if !self.committed {
                    RollbackTransaction(self.handle);
                }
                CloseHandle(self.handle);
            }
        }
    }
    fn protected_head(request: &Value, private: &Path) -> Result<Vec<File>> {
        let checkout = &request["checkout"];
        if checkout.is_null() {
            return Ok(Vec::new());
        }
        let root = Path::new(text(&checkout["root"])?).canonicalize()?;
        let revision = text(&checkout["revision"])?;
        ensure!(
            matches!(revision.len(), 40 | 64) && revision.bytes().all(|b| b.is_ascii_hexdigit()),
            "Invalid expected Git revision"
        );
        ensure!(
            root.join(".git").is_dir(),
            "unsupported: protected patches require a .git directory"
        );
        ensure!(
            !root.join(".git/commondir").exists(),
            "unsupported: linked Git metadata needs additional protected guards"
        );
        let mut held = Vec::new();
        let head = guarded(&root.join(".git/HEAD"), private)?;
        let mut file = plain_open(&head.path)?.context("Missing Git HEAD")?;
        info(&file)?;
        let mut value = String::new();
        (&mut file).take(1025).read_to_string(&mut value)?;
        ensure!(value.len() <= 1024, "Git HEAD exceeds bound");
        held.push(file);
        held.extend(head._parents);
        let actual = if let Some(reference) = value.trim().strip_prefix("ref: ") {
            ensure!(
                reference.starts_with("refs/")
                    && Path::new(reference)
                        .components()
                        .all(|c| matches!(c, Component::Normal(_))),
                "Invalid symbolic HEAD"
            );
            let path = guarded(&root.join(".git").join(reference), private)?;
            let mut file = plain_open(&path.path)?
                .context("unsupported: packed-only refs require protected loose-ref support")?;
            info(&file)?;
            let mut contents = String::new();
            (&mut file).take(129).read_to_string(&mut contents)?;
            ensure!(contents.len() <= 128, "Git ref exceeds bound");
            held.push(file);
            held.extend(path._parents);
            contents
        } else {
            value
        };
        ensure!(
            actual.trim().eq_ignore_ascii_case(revision),
            "conflict: Git revision changed"
        );
        Ok(held)
    }
    fn dispose(file: &File) -> Result<()> {
        let value = FILE_DISPOSITION_INFO { DeleteFile: true };
        ensure!(
            unsafe {
                SetFileInformationByHandle(
                    file.as_raw_handle() as HANDLE,
                    FileDispositionInfo,
                    &value as *const _ as *const _,
                    std::mem::size_of_val(&value) as u32,
                )
            } != 0,
            "{}",
            os_error("Transacted disposition")
        );
        Ok(())
    }
    fn rename(file: &File, destination: &Path) -> Result<()> {
        let name: Vec<u16> = destination.as_os_str().encode_wide().collect();
        let length = std::mem::offset_of!(FILE_RENAME_INFO, FileName) + name.len() * 2;
        let mut buffer = vec![0u64; length.div_ceil(8)];
        let pointer = buffer.as_mut_ptr() as *mut FILE_RENAME_INFO;
        unsafe {
            (*pointer).Anonymous.ReplaceIfExists = false;
            (*pointer).RootDirectory = ptr::null_mut();
            (*pointer).FileNameLength = (name.len() * 2) as u32;
            ptr::copy_nonoverlapping(
                name.as_ptr(),
                ptr::addr_of_mut!((*pointer).FileName) as *mut u16,
                name.len(),
            );
            ensure!(
                SetFileInformationByHandle(
                    file.as_raw_handle() as HANDLE,
                    FileRenameInfo,
                    pointer as *const _,
                    length as u32
                ) != 0,
                "{}",
                os_error("Transacted rename")
            );
        }
        Ok(())
    }
    fn expected(observed: &Value, expected: &Value) -> Result<()> {
        ensure!(expected["kind"].is_string(), "Expected state is required");
        if expected["kind"] == "file" {
            ensure!(
                expected["digest"].as_str().is_some_and(|s| s.len() == 64),
                "Expected SHA-256 is required"
            );
        }
        ensure!(
            compatible(observed, expected),
            "conflict: Expected file state changed"
        );
        Ok(())
    }
    fn execute_inner(request: &Value, private: &Path) -> Result<Value> {
        ensure!(
            serde_json::to_vec(request)?.len() <= LIMIT * 2,
            "Effect request exceeds bound"
        );
        let _head = protected_head(request, private)?;
        let mut transaction = Transaction::new()?;
        let mut held = Vec::new();
        let mut guards = Vec::new();
        let mut before = Vec::new();
        let mut intended = Vec::new();
        let mut total = 0usize;
        let mut destinations = std::collections::BTreeSet::new();
        for op in operations(request)? {
            let target = guarded(Path::new(text(&op["path"])?), private)?;
            ensure!(
                destinations.insert(target.path.clone()),
                "Repeated transaction destination"
            );
            if let Some(root) = checkout(&target.path) {
                ensure!(
                    !request["checkout"].is_null()
                        && root == Path::new(text(&request["checkout"]["root"])?).canonicalize()?,
                    "Checkout writes require a protected staged patch"
                );
            }
            if !request["checkout"].is_null() {
                ensure!(
                    target.path.starts_with(
                        Path::new(text(&request["checkout"]["root"])?).canonicalize()?
                    ),
                    "Patch destination escaped checkout"
                );
            }
            let operation = text(&op["operation"])?;
            let access = GENERIC_READ | GENERIC_WRITE | DELETE;
            let mut target_file = transaction.open(&target.path, access, OPEN_EXISTING)?;
            let observed = state(&target.path, target_file.as_mut())?;
            expected(&observed, &op["expected"])?;
            before.push(json!({"path":target.path,"state":observed}));
            match operation {
                "write" | "copy" => {
                    let bytes = if operation == "write" {
                        STANDARD.decode(text(&op["base64"])?)?
                    } else {
                        let source = guarded(Path::new(text(&op["source"])?), private)?;
                        ensure!(source.path != target.path, "Copy endpoints must differ");
                        let mut file = transaction
                            .open(&source.path, GENERIC_READ, OPEN_EXISTING)?
                            .context("conflict: Copy source is absent")?;
                        let source_state = state(&source.path, Some(&mut file))?;
                        ensure!(
                            source_state["kind"] == "file",
                            "Copy source must be a regular file"
                        );
                        expected(&source_state, &op["source_expected"])?;
                        file.seek(SeekFrom::Start(0))?;
                        let mut bytes = Vec::new();
                        (&mut file).take(LIMIT as u64 + 1).read_to_end(&mut bytes)?;
                        before.push(json!({"path":source.path,"state":source_state}));
                        held.push(file);
                        guards.push(source);
                        bytes
                    };
                    total += bytes.len();
                    ensure!(total <= LIMIT, "Aggregate write byte limit");
                    ensure!(
                        observed["kind"] == "absent" || observed["kind"] == "file",
                        "Only regular files can be written"
                    );
                    let actual = json!({"kind":"file","digest":digest(&bytes)});
                    ensure!(
                        compatible(&actual, &op["intended"]),
                        "Intended bytes differ from the recorded effect"
                    );
                    let mut file = match target_file.take() {
                        Some(file) => file,
                        None => transaction
                            .open(&target.path, access, CREATE_NEW)?
                            .context("New file handle missing")?,
                    };
                    info(&file)?;
                    file.seek(SeekFrom::Start(0))?;
                    file.set_len(0)?;
                    file.write_all(&bytes)?;
                    file.sync_all()?;
                    held.push(file);
                    intended.push(json!({"path":target.path,"state":actual}));
                }
                "mkdir" => {
                    ensure!(
                        observed["kind"] == "absent",
                        "Directory destination must be absent"
                    );
                    let name = wide(&target.path);
                    ensure!(
                        unsafe {
                            CreateDirectoryTransactedW(
                                ptr::null(),
                                name.as_ptr(),
                                ptr::null(),
                                transaction.handle,
                            )
                        } != 0,
                        "{}",
                        os_error("Transacted mkdir")
                    );
                    intended.push(json!({"path":target.path,"state":{"kind":"directory"}}));
                }
                "remove" => {
                    let file = target_file
                        .take()
                        .context("conflict: Remove target is absent")?;
                    ensure!(
                        observed["kind"] == "file" || observed["empty"] == true,
                        "Only a file or empty directory can be removed"
                    );
                    dispose(&file)?;
                    // Close the transacted delete-on-close handle while the
                    // transaction remains active; its namespace mutation stays
                    // isolated until the subsequent transaction commit.
                    drop(file);
                    intended.push(json!({"path":target.path,"state":{"kind":"absent"}}));
                }
                "move" => {
                    ensure!(
                        observed["kind"] == "absent",
                        "Move requires an absent destination"
                    );
                    let source = guarded(Path::new(text(&op["source"])?), private)?;
                    ensure!(source.path != target.path, "Move endpoints must differ");
                    ensure!(
                        checkout(&source.path).is_none(),
                        "Checkout source moves require a protected patch"
                    );
                    let mut file = transaction
                        .open(&source.path, access, OPEN_EXISTING)?
                        .context("conflict: Move source is absent")?;
                    let old = state(&source.path, Some(&mut file))?;
                    ensure!(old["kind"] == "file", "Move source must be a regular file");
                    expected(&old, &op["source_expected"])?;
                    rename(&file, &target.path)?;
                    before.push(json!({"path":source.path,"state":old}));
                    intended.push(
                        json!({"path":target.path,"state":{"kind":"file","digest":old["digest"]}}),
                    );
                    intended.push(json!({"path":source.path,"state":{"kind":"absent"}}));
                    held.push(file);
                    guards.push(source);
                }
                _ => bail!("Unsupported filesystem operation {operation}"),
            }
            if let Some(file) = target_file {
                held.push(file);
            }
            guards.push(target);
        }
        transaction.commit()?;
        drop(held);
        Ok(json!({"state":"completed","route":"windows-txf","before":before,"after":intended}))
    }
    fn reconcile(request: &Value, private: &Path) -> Result<Value> {
        let observed = inspect(request, private)?;
        let rows = if request["operation"] == "batch" {
            observed["operations"].as_array().unwrap().clone()
        } else {
            vec![observed]
        };
        let mut old = true;
        let mut intended = true;
        for (op, current) in operations(request)?.into_iter().zip(rows.iter()) {
            old &= compatible(&current["state"], &op["expected"]);
            intended &= compatible(&current["state"], &op["intended"]);
            if matches!(op["operation"].as_str(), Some("move" | "copy")) {
                old &= compatible(&current["source_state"], &op["source_expected"]);
                intended &= if op["operation"] == "move" {
                    current["source_state"]["kind"] == "absent"
                } else {
                    compatible(&current["source_state"], &op["source_expected"])
                };
            }
        }
        Ok(
            json!({"state":if intended{"completed"}else if old{"old"}else{"conflict"},"observed":rows,"route":"windows-guarded-reconciliation"}),
        )
    }
    pub(super) fn execute(request: &Value, private: &Path) -> Result<Value> {
        if request["operation"] == "reconcile" {
            return reconcile(&request["request"], private);
        }
        match execute_inner(request, private) {
            Ok(value) => Ok(value),
            Err(e) => {
                let error = format!("{e:#}");
                let state = if error.contains("uncertain:") {
                    "uncertain"
                } else if error.contains("conflict:") {
                    "conflict"
                } else if error.contains("unsupported:") || error.contains("Unsupported") {
                    "unsupported"
                } else {
                    "failed"
                };
                Ok(json!({"state":state,"error":error,"route":"windows-txf"}))
            }
        }
    }
}

#[cfg(windows)]
pub fn inspect(request: &Value, private: &Path) -> Result<Value> {
    windows::inspect(request, private)
}
#[cfg(windows)]
pub fn execute(request: &Value, private: &Path) -> Result<Value> {
    windows::execute(request, private)
}
