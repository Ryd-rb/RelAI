//! Private, detached linked worktrees with retained anchors and atomic Windows publication.
//! No executable, hook, filter, owner-selected removal path, or recursive delete is used.
use anyhow::{Context, Result, bail, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use gix::bstr::ByteSlice;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
};
const LIMIT: usize = 8 * 1024 * 1024;
const ENTRIES: usize = 4096;
fn text(v: &Value) -> Result<&str> {
    v.as_str().context("Expected text")
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn id(value: &Value) -> Result<String> {
    let id = text(value)?;
    ensure!(
        uuid::Uuid::parse_str(id).is_ok_and(|u| u.to_string() == id),
        "Expected canonical managed worktree ID"
    );
    Ok(id.to_owned())
}
fn portable(path: &Path) -> Result<String> {
    Ok(path
        .to_str()
        .context("Non-Unicode path")?
        .trim_start_matches("\\\\?\\")
        .replace('\\', "/"))
}
fn relative(path: &str) -> Result<()> {
    ensure!(
        !path.is_empty()
            && path.len() <= 1024
            && path.is_ascii()
            && !path.contains(['\\', ':', '\0']),
        "unsupported: portable ASCII checkout paths required"
    );
    for part in path.split('/') {
        let upper = part.split('.').next().unwrap_or("").to_ascii_uppercase();
        ensure!(
            !part.is_empty()
                && part != "."
                && part != ".."
                && !part.eq_ignore_ascii_case(".git")
                && !part.ends_with(['.', ' '])
                && !matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
                && !(upper.len() == 4
                    && (upper.starts_with("COM") || upper.starts_with("LPT"))
                    && matches!(upper.as_bytes()[3], b'1'..=b'9')),
            "Unsafe checkout path"
        );
    }
    ensure!(
        !path
            .split('/')
            .any(|part| part.eq_ignore_ascii_case(".gitattributes")),
        "unsupported: filter/attribute checkout requires separate qualification"
    );
    Ok(())
}
fn bounded(path: &Path) -> Result<Vec<u8>> {
    bounded_limit(path, LIMIT)
}
fn bounded_limit(path: &Path, limit: usize) -> Result<Vec<u8>> {
    let mut file = guard(path)?.0;
    ensure!(
        file.metadata()?.is_file() && file.metadata()?.len() <= limit as u64,
        "Bounded regular file required"
    );
    let mut data = Vec::new();
    (&mut file).take(limit as u64 + 1).read_to_end(&mut data)?;
    ensure!(data.len() <= limit, "File byte limit");
    Ok(data)
}
#[cfg(windows)]
fn guard(path: &Path) -> Result<(File, String)> {
    use std::{
        fs::OpenOptions,
        os::windows::{fs::OpenOptionsExt, io::AsRawHandle},
    };
    use windows_sys::Win32::{Foundation::HANDLE, Storage::FileSystem::*};
    let file = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)?;
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    ensure!(
        unsafe { GetFileInformationByHandle(file.as_raw_handle() as HANDLE, &mut info) } != 0,
        "Cannot identify worktree file"
    );
    ensure!(
        info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT == 0,
        "Unexpected worktree redirection"
    );
    ensure!(
        info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0 || info.nNumberOfLinks == 1,
        "Hard-linked worktree files are excluded"
    );
    Ok((
        file,
        format!(
            "{:x}:{:x}:{:x}",
            info.dwVolumeSerialNumber, info.nFileIndexHigh, info.nFileIndexLow
        ),
    ))
}
#[cfg(not(windows))]
fn guard(path: &Path) -> Result<(File, String)> {
    use std::os::unix::fs::MetadataExt;
    let m = fs::symlink_metadata(path)?;
    ensure!(
        !m.is_symlink() && (!m.is_file() || m.nlink() == 1),
        "Unexpected linked worktree file"
    );
    Ok((File::open(path)?, format!("{}:{}", m.dev(), m.ino())))
}
fn parents(path: &Path) -> Result<Vec<File>> {
    path.ancestors().map(|p| guard(p).map(|x| x.0)).collect()
}
fn identity(repo: &gix::Repository) -> Result<Value> {
    Ok(
        json!({"worktree":guard(repo.workdir().context("Missing worktree")?)?.1,"gitdir":guard(repo.path())?.1}),
    )
}
fn head(repo: &gix::Repository) -> Result<String> {
    Ok(repo.head_id()?.to_string())
}
fn paths(
    repo: &gix::Repository,
    private: &Path,
    wid: &str,
) -> (PathBuf, PathBuf, PathBuf, PathBuf) {
    (
        private.join("managed-worktrees").join(wid),
        repo.path().join("worktrees").join(format!("relai-{wid}")),
        private
            .join("worktree-evidence")
            .join(format!("{wid}.json")),
        repo.path().join("refs/relai/anchors").join(wid),
    )
}
fn walk_tree(
    repo: &gix::Repository,
    oid: gix::ObjectId,
    prefix: &str,
    files: &mut BTreeMap<String, Value>,
    total: &mut usize,
) -> Result<()> {
    ensure!(prefix.len() <= 1024, "Tree depth limit");
    let tree = repo.find_tree(oid)?;
    for entry in tree.iter() {
        let entry = entry?;
        let name = entry.filename().to_str()?;
        let path = format!("{prefix}{name}");
        relative(&path)?;
        if entry.mode().is_tree() {
            walk_tree(
                repo,
                entry.oid().to_owned(),
                &format!("{path}/"),
                files,
                total,
            )?;
        } else {
            ensure!(
                entry.mode().value() == 0o100644 || entry.mode().value() == 0o100755,
                "unsupported: only regular checkout files are admitted"
            );
            ensure!(files.len() < ENTRIES, "Checkout entry limit");
            let blob = repo.find_blob(entry.oid().to_owned())?;
            *total += blob.data.len();
            ensure!(*total <= LIMIT, "Checkout byte limit");
            files.insert(path,json!({"object":entry.oid().to_string(),"mode":format!("{:o}",entry.mode().value()),"digest":digest(&blob.data),"base64":STANDARD.encode(&blob.data)}));
        }
    }
    Ok(())
}
fn checkout(repo: &gix::Repository, oid: gix::ObjectId) -> Result<(Value, String)> {
    let commit = repo.find_commit(oid)?;
    let tree = commit.tree_id()?.detach();
    let mut files = BTreeMap::new();
    walk_tree(repo, tree, "", &mut files, &mut 0)?;
    let mut names = BTreeSet::new();
    for path in files.keys() {
        let lower = path.to_ascii_lowercase();
        ensure!(
            names.insert(lower),
            "Case-alias checkout paths are excluded"
        );
    }
    for path in &names {
        for (at, _) in path.match_indices('/') {
            ensure!(
                !names.contains(&path[..at]),
                "Checkout file/directory collision"
            );
        }
    }
    let index = repo.index_from_tree(&tree)?;
    let mut bytes = Vec::new();
    index.write_to(&mut bytes, Default::default())?;
    ensure!(bytes.len() <= LIMIT, "Checkout index limit");
    Ok((json!(files), STANDARD.encode(bytes)))
}
fn evidence(root: &Path, private: &Path, wid: &str) -> Result<Value> {
    let path = private
        .join("worktree-evidence")
        .join(format!("{wid}.json"));
    let _parents = parents(path.parent().unwrap())?;
    let value: Value = serde_json::from_slice(&bounded_limit(&path, LIMIT * 2)?)?;
    ensure!(
        value["id"] == wid && Path::new(text(&value["root"])?) == root,
        "Managed ownership evidence differs from project"
    );
    Ok(value)
}
fn merged(repo: &gix::Repository, ancestor: &str, current: &str) -> Result<bool> {
    let mut pending = vec![current.parse::<gix::ObjectId>()?];
    let target = ancestor.parse::<gix::ObjectId>()?;
    let mut seen = BTreeSet::new();
    while let Some(id) = pending.pop() {
        if id == target {
            return Ok(true);
        }
        if seen.insert(id) {
            ensure!(seen.len() <= 16384, "Merge reachability bound");
            pending.extend(repo.find_commit(id)?.parent_ids().map(|id| id.detach()));
        }
    }
    Ok(false)
}
fn expected_files(
    repo: &gix::Repository,
    private: &Path,
    e: &Value,
) -> Result<BTreeMap<PathBuf, Vec<u8>>> {
    let wid = id(&e["id"])?;
    let (root, meta, _, _) = paths(repo, private, &wid);
    let mut files = BTreeMap::new();
    for (path, value) in e["files"]
        .as_object()
        .context("Missing checkout manifest")?
    {
        relative(path)?;
        files.insert(root.join(path), STANDARD.decode(text(&value["base64"])?)?);
    }
    files.insert(
        root.join(".git"),
        format!("gitdir: {}\n", portable(&meta)?).into_bytes(),
    );
    files.insert(
        meta.join("HEAD"),
        format!("{}\n", text(&e["target"])?).into_bytes(),
    );
    files.insert(meta.join("commondir"), b"../..\n".to_vec());
    files.insert(
        meta.join("gitdir"),
        format!("{}\n", portable(&root.join(".git"))?).into_bytes(),
    );
    files.insert(
        meta.join("locked"),
        b"RelAI managed ownership; use exact approved worktree removal\n".to_vec(),
    );
    files.insert(
        meta.join("index"),
        STANDARD.decode(text(&e["index_base64"])?)?,
    );
    Ok(files)
}
fn inventory(root: &Path) -> Result<(BTreeSet<PathBuf>, BTreeSet<PathBuf>)> {
    let mut files = BTreeSet::new();
    let mut directories = BTreeSet::new();
    let mut pending = vec![root.to_owned()];
    while let Some(dir) = pending.pop() {
        guard(&dir)?;
        directories.insert(dir.clone());
        for entry in fs::read_dir(&dir)? {
            let path = entry?.path();
            let file = guard(&path)?.0;
            ensure!(
                files.len() + directories.len() < ENTRIES + 128,
                "Managed inventory limit"
            );
            if file.metadata()?.is_dir() {
                pending.push(path);
            } else {
                ensure!(file.metadata()?.is_file(), "Unsupported managed file kind");
                files.insert(path);
            }
        }
    }
    Ok((files, directories))
}
fn clean(repo: &gix::Repository, private: &Path, e: &Value) -> Result<Value> {
    let wid = id(&e["id"])?;
    let (root, meta, _, anchor) = paths(repo, private, &wid);
    let expected = expected_files(repo, private, e)?;
    let (mut files, mut dirs) = inventory(&root)?;
    let (more, otherdirs) = inventory(&meta)?;
    files.extend(more);
    dirs.extend(otherdirs);
    let mut owned_dirs = BTreeSet::from([root.clone(), meta.clone()]);
    for file in expected.keys() {
        for parent in file.parent().unwrap().ancestors() {
            if parent == root || parent == meta {
                break;
            }
            if parent.starts_with(&root) || parent.starts_with(&meta) {
                owned_dirs.insert(parent.to_owned());
            }
        }
    }
    ensure!(
        dirs == owned_dirs,
        "conflict: Unowned directories are preserved"
    );
    ensure!(
        files == expected.keys().cloned().collect(),
        "conflict: Untracked or missing worktree files are preserved"
    );
    for (path, bytes) in &expected {
        ensure!(
            bounded(path)? == *bytes,
            "conflict: Modified checkout or index is preserved"
        );
    }
    let linked = gix::open_opts(&root, gix::open::Options::isolated())?;
    ensure!(
        linked.common_dir().canonicalize()? == repo.path().canonicalize()?
            && head(&linked)? == text(&e["target"])?,
        "conflict: Linked worktree identity changed"
    );
    ensure!(
        String::from_utf8(bounded(&anchor)?)?.trim() == text(&e["target"])?,
        "conflict: Retained anchor changed"
    );
    let identities: BTreeMap<String, String> = dirs
        .iter()
        .map(|p| Ok((portable(p)?, guard(p)?.1)))
        .collect::<Result<_>>()?;
    Ok(
        json!({"files":files,"directories":dirs,"identities":identities,"head":head(&linked)?,"clean":true}),
    )
}
pub fn prepare(root: &Path, private: &Path, request: &Value) -> Result<Value> {
    let private = private.canonicalize()?;
    let _private = parents(&private)?;
    let repo = crate::git_operations::open(root, &private)?;
    let root = repo.workdir().unwrap();
    let current = head(&repo)?;
    let operation = text(&request["operation"])?;
    if operation == "create" {
        let target = request["target"].as_str().unwrap_or(&current);
        let oid = target.parse::<gix::ObjectId>()?;
        let (files, index) = checkout(&repo, oid)?;
        let wid = uuid::Uuid::new_v4().to_string();
        Ok(
            json!({"format":"relai-managed-worktree-v1","operation":operation,"id":wid,"root":root,"repository_identity":identity(&repo)?,"expected_head":current,"target":oid.to_string(),"source_branch":repo.head()?.referent_name().map(|n|n.as_bstr().to_string()),"files":files,"index_base64":index}),
        )
    } else if operation == "remove" {
        let wid = id(&request["worktree"])?;
        let e = evidence(root, &private, &wid)?;
        ensure!(
            e["repository_identity"] == identity(&repo)?,
            "conflict: Owner repository identity changed"
        );
        ensure!(
            merged(&repo, text(&e["target"])?, &current)?,
            "Unmerged managed checkout is retained"
        );
        let observed = clean(&repo, &private, &e)?;
        Ok(
            json!({"format":"relai-managed-worktree-v1","operation":operation,"id":wid,"root":root,"repository_identity":identity(&repo)?,"expected_head":current,"target":e["target"],"evidence_digest":digest(&serde_json::to_vec(&e)?),"observed":observed}),
        )
    } else {
        bail!("Unsupported managed worktree operation")
    }
}
pub fn inspect(root: &Path, private: &Path) -> Result<Value> {
    let private = private.canonicalize()?;
    let _private = parents(&private)?;
    let repo = crate::git_operations::open(root, &private)?;
    let mut out = Vec::new();
    let folder = private.join("worktree-evidence");
    if folder.exists() {
        let _folder = parents(&folder)?;
        for entry in fs::read_dir(&folder)? {
            ensure!(out.len() < 1024, "Managed listing bound");
            let entry = entry?;
            let stem = entry
                .path()
                .file_stem()
                .context("Invalid evidence name")?
                .to_string_lossy()
                .to_string();
            let wid = id(&json!(stem))?;
            let e = evidence(repo.workdir().unwrap(), &private, &wid);
            let Ok(e) = e else { continue };
            let (path, meta, _, anchor) = paths(&repo, &private, &wid);
            let present = path.exists() || meta.exists();
            let check = if present {
                clean(&repo, &private, &e)
                    .map(|_| "clean")
                    .unwrap_or("conflict")
            } else {
                "removed"
            };
            out.push(json!({"id":wid,"path":path,"metadata":meta,"commit":e["target"],"branch":e["source_branch"],"detached":true,"state":check,"retained_anchor":anchor,"evidence_digest":digest(&serde_json::to_vec(&e)?)}));
        }
    }
    Ok(json!({"worktrees":out,"scope":"RelAI-owned private detached linked worktrees only"}))
}

#[cfg(windows)]
mod windows {
    use super::*;
    use std::{
        fs::OpenOptions,
        io::{Seek, SeekFrom, Write},
        os::windows::{
            ffi::OsStrExt,
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
    struct Tx(HANDLE, bool);
    impl Tx {
        fn new() -> Result<Self> {
            let handle = unsafe {
                CreateTransaction(ptr::null_mut(), ptr::null_mut(), 0, 0, 0, 0, ptr::null())
            };
            ensure!(
                !handle.is_null() && handle != INVALID_HANDLE_VALUE,
                "unsupported: Windows TxF unavailable: {}",
                std::io::Error::last_os_error()
            );
            Ok(Self(handle, false))
        }
        fn open(&self, path: &Path, access: u32, disposition: u32) -> Result<File> {
            let h = unsafe {
                CreateFileTransactedW(
                    wide(path).as_ptr(),
                    access,
                    FILE_SHARE_READ,
                    ptr::null(),
                    disposition,
                    FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
                    ptr::null_mut(),
                    self.0,
                    ptr::null(),
                    ptr::null(),
                )
            };
            ensure!(
                h != INVALID_HANDLE_VALUE,
                "conflict: Transacted file access: {}",
                std::io::Error::last_os_error()
            );
            Ok(unsafe { File::from_raw_handle(h) })
        }
        fn mkdir(&self, path: &Path) -> Result<()> {
            ensure!(
                unsafe {
                    CreateDirectoryTransactedW(
                        ptr::null(),
                        wide(path).as_ptr(),
                        ptr::null(),
                        self.0,
                    )
                } != 0,
                "Transacted directory creation: {}",
                std::io::Error::last_os_error()
            );
            Ok(())
        }
        fn commit(&mut self) -> Result<()> {
            ensure!(
                unsafe { CommitTransaction(self.0) } != 0,
                "uncertain: Worktree transaction commit: {}",
                std::io::Error::last_os_error()
            );
            self.1 = true;
            Ok(())
        }
    }
    impl Drop for Tx {
        fn drop(&mut self) {
            unsafe {
                if !self.1 {
                    RollbackTransaction(self.0);
                }
                CloseHandle(self.0);
            }
        }
    }
    fn main_guards(repo: &gix::Repository, request: &Value) -> Result<Vec<File>> {
        let mut held = parents(repo.path())?;
        held.push(guard(&repo.path().join("HEAD"))?.0);
        let h = repo.head()?;
        if let Some(name) = h.referent_name() {
            let path = repo.path().join(name.as_bstr().to_str()?);
            held.extend(parents(path.parent().unwrap())?);
            held.push(guard(&path)?.0);
        }
        ensure!(
            request["repository_identity"] == identity(repo)?
                && request["expected_head"] == head(repo)?,
            "conflict: Main repository head or identity changed"
        );
        Ok(held)
    }
    fn container(path: &Path) -> Result<Vec<File>> {
        match fs::create_dir(path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e.into()),
        }
        parents(path)
    }
    fn same_volume(a: &Path, b: &Path) -> Result<()> {
        let aa = guard(a)?.1;
        let bb = guard(b)?.1;
        ensure!(
            aa.split(':').next() == bb.split(':').next(),
            "unsupported: Worktree transaction requires one local volume"
        );
        Ok(())
    }
    fn verify(file: &File, expected: &[u8]) -> Result<()> {
        let mut file = file.try_clone()?;
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        ensure!(
            unsafe { GetFileInformationByHandle(file.as_raw_handle() as HANDLE, &mut info) } != 0
                && info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT == 0
                && info.nNumberOfLinks == 1,
            "conflict: Linked file is not admitted"
        );
        file.seek(SeekFrom::Start(0))?;
        let mut data = Vec::new();
        file.take(LIMIT as u64 + 1).read_to_end(&mut data)?;
        ensure!(data == expected, "conflict: Managed bytes changed");
        Ok(())
    }
    pub(super) fn apply(request: &Value, private: &Path) -> Result<Value> {
        let private = private.canonicalize()?;
        let _private = parents(&private)?;
        let root = Path::new(text(&request["root"])?);
        let repo = crate::git_operations::open(root, &private)?;
        ensure!(
            request["format"] == "relai-managed-worktree-v1",
            "Unknown worktree format"
        );
        let _main = main_guards(&repo, request)?;
        same_volume(repo.path(), &private)?;
        let wid = id(&request["id"])?;
        let (path, meta, evidencepath, anchor) = paths(&repo, &private, &wid);
        let operation = text(&request["operation"])?;
        if operation == "create" {
            ensure!(
                !path.exists() && !meta.exists(),
                "conflict: Managed destination already exists"
            );
            let (files, index) = checkout(&repo, text(&request["target"])?.parse()?)?;
            ensure!(
                files == request["files"] && index == text(&request["index_base64"])?,
                "Checkout object bytes differ from exact plan"
            );
            let mut containers = Vec::new();
            for p in [
                private.join("managed-worktrees"),
                private.join("worktree-evidence"),
                repo.path().join("worktrees"),
                repo.path().join("refs/relai"),
                repo.path().join("refs/relai/anchors"),
            ] {
                containers.extend(container(&p)?);
            }
            let encoded = serde_json::to_vec(request)?;
            ensure!(encoded.len() <= LIMIT * 2, "Evidence limit");
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&evidencepath)
            {
                Ok(mut file) => {
                    file.write_all(&encoded)?;
                    file.sync_all()?
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => ensure!(
                    bounded_limit(&evidencepath, LIMIT * 2)? == encoded,
                    "conflict: Ownership evidence already differs"
                ),
                Err(e) => return Err(e.into()),
            }
            let _evidence = guard(&evidencepath)?;
            let mut files = expected_files(&repo, &private, request)?;
            if anchor.exists() {
                ensure!(
                    bounded(&anchor)? == format!("{}\n", text(&request["target"])?).as_bytes(),
                    "conflict: Anchor differs"
                );
            } else {
                files.insert(
                    anchor.clone(),
                    format!("{}\n", text(&request["target"])?).into_bytes(),
                );
            }
            let mut dirs = BTreeSet::from([path.clone(), meta.clone()]);
            for file in files.keys() {
                for parent in file.parent().unwrap().ancestors() {
                    if parent == path || parent == meta {
                        break;
                    }
                    if parent.starts_with(&path) || parent.starts_with(&meta) {
                        dirs.insert(parent.to_owned());
                    }
                }
            }
            let mut dirs = dirs.into_iter().collect::<Vec<_>>();
            dirs.sort_by_key(|p| p.components().count());
            let mut tx = Tx::new()?;
            for dir in dirs {
                tx.mkdir(&dir)?;
            }
            let mut held = Vec::new();
            for (path, bytes) in files {
                let mut file = tx.open(&path, GENERIC_READ | GENERIC_WRITE, CREATE_NEW)?;
                file.write_all(&bytes)?;
                file.sync_all()?;
                held.push(file);
            }
            tx.commit()?;
            drop(held);
            drop(containers);
        } else if operation == "remove" {
            let e = evidence(root, &private, &wid)?;
            ensure!(
                digest(&serde_json::to_vec(&e)?) == text(&request["evidence_digest"])?,
                "Ownership evidence changed"
            );
            ensure!(
                merged(&repo, text(&e["target"])?, &head(&repo)?)?,
                "Unmerged managed checkout is retained"
            );
            let observed = clean(&repo, &private, &e)?;
            ensure!(
                observed == request["observed"],
                "conflict: Managed inventory changed"
            );
            let expected = expected_files(&repo, &private, &e)?;
            let mut tx = Tx::new()?;
            let _checkout_parent = parents(path.parent().unwrap())?;
            let _metadata_parent = parents(meta.parent().unwrap())?;
            // Acquire all directory handles before resolving any child file. They deny
            // concurrent rename/removal while TxF protects the full namespace change.
            let mut dirs: Vec<PathBuf> = serde_json::from_value(observed["directories"].clone())?;
            dirs.sort_by_key(|p| p.components().count());
            let mut directories = Vec::new();
            for dir in dirs {
                let file = tx.open(&dir, GENERIC_READ | DELETE, OPEN_EXISTING)?;
                let mut info = BY_HANDLE_FILE_INFORMATION::default();
                ensure!(
                    unsafe {
                        GetFileInformationByHandle(file.as_raw_handle() as HANDLE, &mut info)
                    } != 0
                        && info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT == 0
                        && info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0,
                    "conflict: Managed directory redirection"
                );
                let identity = format!(
                    "{:x}:{:x}:{:x}",
                    info.dwVolumeSerialNumber, info.nFileIndexHigh, info.nFileIndexLow
                );
                ensure!(
                    observed["identities"][portable(&dir)?] == identity,
                    "conflict: Managed directory identity changed"
                );
                directories.push((dir, file));
            }
            for (path, bytes) in expected {
                let file = tx.open(&path, GENERIC_READ | GENERIC_WRITE | DELETE, OPEN_EXISTING)?;
                verify(&file, &bytes)?;
                let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
                ensure!(
                    unsafe {
                        SetFileInformationByHandle(
                            file.as_raw_handle() as HANDLE,
                            FileDispositionInfo,
                            &disposition as *const _ as *const _,
                            std::mem::size_of_val(&disposition) as u32,
                        )
                    } != 0,
                    "Transacted file removal failed"
                );
                drop(file);
            }
            for (_dir, file) in directories.into_iter().rev() {
                let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
                ensure!(
                    unsafe {
                        SetFileInformationByHandle(
                            file.as_raw_handle() as HANDLE,
                            FileDispositionInfo,
                            &disposition as *const _ as *const _,
                            std::mem::size_of_val(&disposition) as u32,
                        )
                    } != 0,
                    "conflict: Nonempty or changed managed directory is preserved"
                );
                drop(file);
            }
            // The retained anchor and immutable ownership evidence are deliberately kept.
            tx.commit()?;
        } else {
            bail!("Unsupported worktree mutation")
        }
        Ok(
            json!({"state":"completed","id":wid,"path":path,"metadata":meta,"commit":request["target"],"detached":true,"retained_anchor":anchor,"route":"gix-linked-worktree/windows-txf"}),
        )
    }
}
fn reconcile(request: &Value, private: &Path) -> Result<Value> {
    let private = private.canonicalize()?;
    let _private = parents(&private)?;
    let repo = crate::git_operations::open(Path::new(text(&request["root"])?), &private)?;
    ensure!(
        request["repository_identity"] == identity(&repo)?,
        "Owner repository identity changed"
    );
    let wid = id(&request["id"])?;
    let (path, meta, _, anchor) = paths(&repo, &private, &wid);
    let absent = !path.exists() && !meta.exists();
    let state = if request["operation"] == "create" {
        if absent {
            "old"
        } else if evidence(repo.workdir().unwrap(), &private, &wid)
            .is_ok_and(|e| e == *request && clean(&repo, &private, &e).is_ok())
        {
            "completed"
        } else {
            "conflict"
        }
    } else if absent {
        let e = evidence(repo.workdir().unwrap(), &private, &wid)?;
        ensure!(
            digest(&serde_json::to_vec(&e)?) == text(&request["evidence_digest"])?
                && bounded(&anchor)? == format!("{}\n", text(&e["target"])?).as_bytes(),
            "Retained ownership evidence or anchor changed"
        );
        "completed"
    } else if evidence(repo.workdir().unwrap(), &private, &wid)
        .is_ok_and(|e| clean(&repo, &private, &e).is_ok())
        && request["expected_head"] == head(&repo)?
    {
        "old"
    } else {
        "conflict"
    };
    Ok(
        json!({"state":state,"id":wid,"path":path,"metadata":meta,"commit":request["target"],"retained_anchor":anchor,"route":"managed-worktree-reconciliation"}),
    )
}
pub fn execute(request: &Value, private: &Path) -> Result<Value> {
    if request["operation"] == "reconcile" {
        return reconcile(&request["request"], private);
    }
    #[cfg(windows)]
    let result = windows::apply(request, private);
    #[cfg(not(windows))]
    let result: Result<Value> = Err(anyhow::anyhow!(
        "unsupported: Managed worktree publication requires Windows TxF"
    ));
    match result {
        Ok(v) => Ok(v),
        Err(error) => {
            let message = format!("{error:#}");
            Ok(
                json!({"state":if message.contains("uncertain:"){"uncertain"}else if message.contains("conflict:"){"conflict"}else if message.contains("unsupported:"){"unsupported"}else{"failed"},"error":message,"route":"managed-worktree"}),
            )
        }
    }
}
