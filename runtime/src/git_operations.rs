//! Bounded local Git object/index/ref operations. No command, hook, filter,
//! credential helper, transport or executable fallback is used.
use anyhow::{Context, Result, bail, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use gix::{bstr::ByteSlice, objs::WriteTo};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path},
};
const BYTES: usize = 8 * 1024 * 1024;
const ENTRIES: usize = 4096;
fn text(value: &Value) -> Result<&str> {
    value.as_str().context("Expected text")
}
fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn bytes(path: &Path) -> Result<Option<Vec<u8>>> {
    let file = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    ensure!(
        file.metadata()?.is_file() && file.metadata()?.len() <= BYTES as u64,
        "Metadata file exceeds bound or is not regular"
    );
    let mut bytes = Vec::new();
    file.take(BYTES as u64 + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= BYTES, "Metadata file exceeds bound");
    Ok(Some(bytes))
}
fn path_name(name: &str) -> Result<()> {
    ensure!(
        !name.is_empty()
            && name.len() <= 1024
            && !name.contains(['\\', ':', '\0'])
            && Path::new(name)
                .components()
                .all(|p| matches!(p, Component::Normal(_))),
        "Expected contained Git path"
    );
    for part in name.split('/') {
        ensure!(
            !part.is_empty()
                && part != "."
                && part != ".."
                && !part.eq_ignore_ascii_case(".git")
                && !part.ends_with(['.', ' ']),
            "Reserved or ambiguous Git path"
        );
        let stem = part.split('.').next().unwrap_or("").to_ascii_uppercase();
        ensure!(
            !matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
                && !(stem.len() == 4
                    && (stem.starts_with("COM") || stem.starts_with("LPT"))
                    && matches!(stem.as_bytes()[3], b'1'..=b'9')),
            "Reserved Windows Git path"
        );
    }
    Ok(())
}
fn linked(path: &Path) -> Result<bool> {
    let m = fs::symlink_metadata(path)?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        Ok(m.file_attributes() & 0x400 != 0)
    }
    #[cfg(not(windows))]
    {
        Ok(m.is_symlink())
    }
}
fn no_links(path: &Path) -> Result<()> {
    for p in path.ancestors() {
        ensure!(!linked(p)?, "Unexpected repository link");
    }
    Ok(())
}
pub(crate) fn open(root: &Path, private: &Path) -> Result<gix::Repository> {
    let root = root.canonicalize()?;
    let private = private.canonicalize()?;
    ensure!(
        !root.starts_with(&private) && !private.starts_with(&root),
        "Private storage is outside Git scope"
    );
    no_links(&root)?;
    ensure!(
        root.join(".git").is_dir(),
        "unsupported: local Git writes require an ordinary .git directory"
    );
    no_links(&root.join(".git"))?;
    ensure!(
        !root.join(".git/commondir").exists()
            && !root.join(".git/objects/info/alternates").exists(),
        "unsupported: linked Git storage requires additional guards"
    );
    for p in ["objects", "refs", "refs/heads"] {
        let path = root.join(".git").join(p);
        if path.exists() {
            no_links(&path)?;
        }
    }
    let _metadata = metadata_guards(&root)?;
    let repo = gix::open_opts(&root, gix::open::Options::isolated())?;
    ensure!(
        repo.workdir()
            .is_some_and(|p| p.canonicalize().ok() == Some(root.clone())),
        "Repository root differs from admitted root"
    );
    Ok(repo)
}
#[cfg(windows)]
fn metadata_file(path: &Path) -> Result<(File, String)> {
    use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
    use windows_sys::Win32::{
        Foundation::HANDLE,
        Storage::FileSystem::{
            BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT,
            FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES,
            GetFileInformationByHandle,
        },
    };
    let file = OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .share_mode(1)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    ensure!(
        unsafe { GetFileInformationByHandle(file.as_raw_handle() as HANDLE, &mut info) } != 0,
        "Repository identity check failed"
    );
    ensure!(
        info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT == 0,
        "Unexpected metadata redirection"
    );
    if info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY == 0 {
        ensure!(
            info.nNumberOfLinks == 1,
            "Hard-linked Git metadata is not admitted"
        );
    }
    Ok((
        file,
        format!(
            "{:x}:{:x}:{:x}",
            info.dwVolumeSerialNumber, info.nFileIndexHigh, info.nFileIndexLow
        ),
    ))
}
#[cfg(not(windows))]
fn metadata_file(path: &Path) -> Result<(File, String)> {
    use std::os::unix::fs::MetadataExt;
    let file = File::open(path)?;
    let m = file.metadata()?;
    ensure!(!linked(path)?, "Unexpected metadata redirection");
    if m.is_file() {
        ensure!(m.nlink() == 1, "Hard-linked metadata");
    }
    Ok((file, format!("{}:{}", m.dev(), m.ino())))
}
fn metadata_guards(root: &Path) -> Result<Vec<File>> {
    let mut held = Vec::new();
    let mut pending = vec![root.join(".git")];
    let mut count = 0usize;
    for ancestor in root.ancestors().collect::<Vec<_>>().into_iter().rev() {
        held.push(metadata_file(ancestor)?.0);
    }
    while let Some(path) = pending.pop() {
        let (file, _) = metadata_file(&path)?;
        let directory = file.metadata()?.is_dir();
        if directory {
            held.push(file);
            for entry in fs::read_dir(&path)? {
                let entry = entry?;
                count += 1;
                ensure!(count <= 16384, "Git metadata inventory exceeds bound");
                ensure!(!linked(&entry.path())?, "Unexpected metadata redirection");
                if entry.file_type()?.is_dir() {
                    pending.push(entry.path());
                } else {
                    metadata_file(&entry.path())?;
                }
            }
        } else {
            bail!("Invalid metadata directory");
        }
    }
    Ok(held)
}
fn repository_identity(repo: &gix::Repository) -> Result<Value> {
    Ok(
        json!({"worktree":metadata_file(repo.workdir().context("Missing worktree")?)?.1,"gitdir":metadata_file(repo.path())?.1}),
    )
}
fn head(repo: &gix::Repository) -> Result<(Option<String>, Option<String>)> {
    let h = repo.head()?;
    Ok((
        h.id().map(|id| id.to_string()),
        h.referent_name().map(|name| name.as_bstr().to_string()),
    ))
}
fn index(repo: &gix::Repository) -> Result<(gix::index::File, Option<Vec<u8>>)> {
    let raw = bytes(&repo.index_path())?;
    let hash = repo.object_hash();
    let state = if let Some(raw) = &raw {
        ensure!(raw.len() >= 12 + hash.len_in_bytes(), "Truncated Git index");
        let count = u32::from_be_bytes(raw[8..12].try_into().unwrap());
        ensure!(count as usize <= ENTRIES, "Index entry limit");
        let mut hasher = gix::hash::hasher(hash);
        hasher.update(&raw[..raw.len() - hash.len_in_bytes()]);
        ensure!(
            hasher.try_finalize()?.as_slice() == &raw[raw.len() - hash.len_in_bytes()..],
            "Git index checksum mismatch"
        );
        gix::index::State::from_bytes(
            raw,
            gix::index::State::new(hash).timestamp(),
            hash,
            gix::index::decode::Options {
                thread_limit: Some(1),
                min_extension_block_in_bytes_for_threading: usize::MAX,
                expected_checksum: None,
            },
        )?
        .0
    } else {
        gix::index::State::new(hash)
    };
    ensure!(
        state.link().is_none()
            && state
                .entries()
                .iter()
                .all(|entry| entry.mode != gix::index::entry::Mode::DIR),
        "unsupported: split and sparse indexes need expanded guarded support"
    );
    Ok((gix::index::File::from_state(state, repo.index_path()), raw))
}
type Files = BTreeMap<String, (gix::ObjectId, u32)>;
fn tree(repo: &gix::Repository, id: gix::ObjectId, prefix: &str, out: &mut Files) -> Result<()> {
    ensure!(prefix.len() <= 1024, "Tree depth/path bound");
    ensure!(
        repo.find_header(id)?.size() <= BYTES as u64,
        "Tree object exceeds bound"
    );
    let object = repo.find_tree(id)?;
    for entry in object.iter() {
        let entry = entry?;
        let name = entry.filename().to_str()?;
        let path = format!("{prefix}{name}");
        path_name(&path)?;
        if entry.mode().is_tree() {
            tree(repo, entry.oid().to_owned(), &format!("{path}/"), out)?;
        } else {
            ensure!(out.len() < ENTRIES, "Tree entry limit");
            out.insert(path, (entry.oid().to_owned(), entry.mode().value() as u32));
        }
    }
    Ok(())
}
fn parent_files(repo: &gix::Repository) -> Result<Files> {
    let mut files = Files::new();
    if let Some(id) = repo.head()?.id() {
        let commit = repo.find_commit(id.detach())?;
        tree(repo, commit.tree_id()?.detach(), "", &mut files)?;
    }
    Ok(files)
}
fn entry_json(index: &gix::index::File) -> Result<Vec<Value>> {
    index.entries().iter().map(|entry|Ok(json!({"path":entry.path(index).to_str()?,"object":entry.id.to_string(),"mode":format!("{:o}",entry.mode.bits()),"stage":entry.stage_raw()}))).collect()
}
fn branch(repo: &gix::Repository, name: &str) -> Result<Option<String>> {
    let Some(reference) = repo.try_find_reference(name)? else {
        return Ok(None);
    };
    Ok(Some(
        reference
            .target()
            .try_id()
            .context("Symbolic branch refs are not admitted")?
            .to_string(),
    ))
}

pub fn inspect(root: &Path, private: &Path) -> Result<Value> {
    let repo = open(root, private)?;
    let _metadata = metadata_guards(repo.workdir().unwrap())?;
    let (head, head_ref) = head(&repo)?;
    let (index, raw) = index(&repo)?;
    let parent = parent_files(&repo)?;
    let root = repo.workdir().unwrap();
    let mut work = Files::new();
    let mut special = BTreeSet::new();
    let mut pending = vec![root.to_owned()];
    let mut visited = 0;
    let mut total = 0;
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            let relative = path
                .strip_prefix(root)?
                .to_str()
                .context("Non-Unicode worktree path")?
                .replace('\\', "/");
            if relative == ".git" {
                continue;
            }
            visited += 1;
            ensure!(visited <= ENTRIES, "Worktree inventory limit");
            path_name(&relative)?;
            if linked(&path)? {
                special.insert(relative);
                continue;
            }
            let m = entry.metadata()?;
            if m.is_dir() {
                pending.push(path);
            } else if m.is_file() {
                ensure!(m.len() <= BYTES as u64, "Worktree file exceeds bound");
                let raw = bytes(&path)?.context("Source disappeared")?;
                total += raw.len();
                ensure!(total <= BYTES, "Aggregate worktree byte limit");
                let id = gix::objs::compute_hash(repo.object_hash(), gix::objs::Kind::Blob, &raw)?;
                work.insert(relative, (id, 0o100644));
            } else {
                special.insert(relative);
            }
        }
    }
    let mut staged = Files::new();
    let mut conflicts = BTreeSet::new();
    for entry in index.entries() {
        let path = entry.path(&index).to_str()?.to_owned();
        if entry.stage() != gix::index::entry::Stage::Unconflicted {
            conflicts.insert(path);
        } else {
            staged.insert(path, (entry.id, entry.mode.bits()));
        }
    }
    let mut names: BTreeSet<_> = parent
        .keys()
        .chain(staged.keys())
        .chain(work.keys())
        .cloned()
        .collect();
    names.extend(special.iter().cloned());
    names.extend(conflicts.iter().cloned());
    let status = names
        .into_iter()
        .filter_map(|path| {
            let a = parent.get(&path);
            let b = staged.get(&path);
            let w = work.get(&path);
            let index_status = if conflicts.contains(&path) {
                "conflicted"
            } else if a == b {
                "clean"
            } else if a.is_none() {
                "added"
            } else if b.is_none() {
                "deleted"
            } else {
                "modified"
            };
            let worktree = if special.contains(&path) {
                "unsupported_file_kind"
            } else if b.is_none() && w.is_some() {
                "untracked"
            } else if w.is_none() && b.is_some() {
                "deleted"
            } else if b.zip(w).is_some_and(|(a, b)| a.0 != b.0) {
                "modified_raw_bytes"
            } else {
                "clean"
            };
            (index_status != "clean" || worktree != "clean")
                .then(|| json!({"path":path,"index":index_status,"worktree":worktree}))
        })
        .collect::<Vec<_>>();
    let mut branches = Vec::new();
    let platform = repo.references()?;
    for reference in platform.prefixed("refs/heads/")? {
        let reference = reference.map_err(|error| anyhow::anyhow!(error.to_string()))?;
        ensure!(branches.len() < 2048, "Branch count limit");
        branches.push(json!({"name":reference.name().as_bstr().to_string(),"object":reference.target().try_id().map(|id|id.to_string())}));
    }
    Ok(
        json!({"root":root,"repository_identity":repository_identity(&repo)?,"head":head,"head_ref":head_ref,"index_digest":raw.as_deref().map(sha),"entries":entry_json(&index)?,"status":status,"branches":branches,"normalization":"raw bytes; no filters or executable helpers"}),
    )
}
struct Raw<'a> {
    kind: gix::objs::Kind,
    data: &'a [u8],
}
impl WriteTo for Raw<'_> {
    fn write_to(&self, out: &mut dyn Write) -> std::io::Result<()> {
        out.write_all(self.data)
    }
    fn kind(&self) -> gix::objs::Kind {
        self.kind
    }
    fn size(&self) -> u64 {
        self.data.len() as u64
    }
}
fn object(
    objects: &mut Vec<Value>,
    hash: gix::hash::Kind,
    kind: gix::objs::Kind,
    bytes: Vec<u8>,
) -> Result<gix::ObjectId> {
    ensure!(
        bytes.len() <= BYTES && objects.len() < ENTRIES,
        "Object plan limit"
    );
    let id = gix::objs::compute_hash(hash, kind, &bytes)?;
    objects
        .push(json!({"id":id.to_string(),"kind":kind.to_string(),"base64":STANDARD.encode(bytes)}));
    Ok(id)
}
fn object_value<T: WriteTo>(
    objects: &mut Vec<Value>,
    hash: gix::hash::Kind,
    value: &T,
) -> Result<gix::ObjectId> {
    let mut bytes = Vec::new();
    value.write_to(&mut bytes)?;
    object(objects, hash, value.kind(), bytes)
}
fn build_tree(
    files: &Files,
    hash: gix::hash::Kind,
    objects: &mut Vec<Value>,
) -> Result<gix::ObjectId> {
    let mut direct = Vec::new();
    let mut subtrees: BTreeMap<String, Files> = BTreeMap::new();
    for (path, (id, mode)) in files {
        if let Some((dir, rest)) = path.split_once('/') {
            subtrees
                .entry(dir.into())
                .or_default()
                .insert(rest.into(), (*id, *mode));
        } else {
            direct.push(gix::objs::tree::Entry {
                mode: gix::objs::tree::EntryMode::from_bytes(format!("{mode:o} ").as_bytes())
                    .context("Invalid tree mode")?,
                filename: path.clone().into(),
                oid: *id,
            });
        }
    }
    for (name, files) in subtrees {
        ensure!(
            !direct
                .iter()
                .any(|entry| entry.filename.as_slice() == name.as_bytes()),
            "File/directory tree conflict"
        );
        let id = build_tree(&files, hash, objects)?;
        direct.push(gix::objs::tree::Entry {
            mode: gix::objs::tree::EntryKind::Tree.into(),
            filename: name.into(),
            oid: id,
        });
    }
    direct.sort();
    object_value(objects, hash, &gix::objs::Tree { entries: direct })
}
fn actor(value: &Value) -> Result<gix::actor::Signature> {
    let name = value["name"].as_str().unwrap_or("RelAI");
    let email = value["email"].as_str().unwrap_or("relai@localhost");
    ensure!(
        !name.is_empty()
            && !email.is_empty()
            && name.len() <= 255
            && email.len() <= 255
            && !name.contains(['\0', '\n', '\r', '<', '>'])
            && !email.contains(['\0', '\n', '\r', '<', '>']),
        "Invalid Git actor"
    );
    let seconds = value["timestamp"].as_i64().unwrap_or_else(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64
    });
    let offset = value["offset"].as_i64().unwrap_or(0);
    ensure!(
        (-86340..=86340).contains(&offset) && offset % 60 == 0,
        "Invalid Git time offset"
    );
    Ok(gix::actor::Signature {
        name: name.into(),
        email: email.into(),
        time: gix::date::Time::new(seconds, offset as i32),
    })
}

pub fn prepare(root: &Path, private: &Path, request: &Value) -> Result<Value> {
    let repo = open(root, private)?;
    let _metadata = metadata_guards(repo.workdir().unwrap())?;
    let (root_head, head_ref) = head(&repo)?;
    let (mut index, raw) = index(&repo)?;
    let operation = text(&request["operation"])?;
    let index_digest = raw.as_deref().map(sha);
    let mut objects = Vec::new();
    let mut plan = json!({"format":"relai-git-local-v1","root":repo.workdir().unwrap(),"repository_identity":repository_identity(&repo)?,"operation":operation,"expected_head":root_head,"head_ref":head_ref,"expected_index":index_digest,"objects":[]});
    match operation {
        "stage" => {
            ensure!(
                root_head.is_some() && head_ref.is_some(),
                "unsupported: staging milestone requires a born branch"
            );
            ensure!(
                request["expected_head"] == json!(root_head)
                    && request["expected_index"] == json!(index_digest),
                "conflict: Head or index changed"
            );
            ensure!(
                index.resolve_undo().is_none(),
                "unsupported: resolve-undo index metadata requires scoped preservation"
            );
            let files = request["files"]
                .as_array()
                .context("Selected files are required")?;
            ensure!(
                !files.is_empty() && files.len() <= 64,
                "Selected path limit"
            );
            let mut selected = BTreeSet::new();
            let mut total = 0;
            for file in files {
                let path = text(&file["path"])?;
                path_name(path)?;
                ensure!(selected.insert(path.to_owned()), "Duplicate selected path");
                index.remove_entries(|_, name, _| name.as_bytes() == path.as_bytes());
                if file["base64"].is_null() {
                    continue;
                }
                let bytes = STANDARD.decode(text(&file["base64"])?)?;
                total += bytes.len();
                ensure!(total <= BYTES, "Selected byte limit");
                let len = bytes.len();
                let id = object(
                    &mut objects,
                    repo.object_hash(),
                    gix::objs::Kind::Blob,
                    bytes,
                )?;
                let mode = match file["mode"].as_str().unwrap_or("100644") {
                    "100644" => gix::index::entry::Mode::FILE,
                    "100755" => gix::index::entry::Mode::FILE_EXECUTABLE,
                    _ => bail!("Only regular file modes are admitted"),
                };
                index.dangerously_push_entry(
                    gix::index::entry::Stat {
                        size: len as u32,
                        ..Default::default()
                    },
                    id,
                    gix::index::entry::Flags::empty(),
                    mode,
                    path.as_bytes().as_bstr(),
                );
            }
            index.sort_entries();
            index.remove_tree();
            let mut bytes = Vec::new();
            index.write_to(&mut bytes, Default::default())?;
            ensure!(bytes.len() <= BYTES, "Index output limit");
            plan["index_base64"] = json!(STANDARD.encode(&bytes));
            plan["intended_index"] = json!(sha(&bytes));
            plan["paths"] = json!(selected);
        }
        "commit" => {
            ensure!(
                root_head.is_some() && head_ref.is_some(),
                "unsupported: selected-file commits require a born branch"
            );
            ensure!(
                request["expected_head"] == json!(root_head)
                    && request["expected_index"] == json!(index_digest),
                "conflict: Head or index changed"
            );
            for marker in [
                "MERGE_HEAD",
                "CHERRY_PICK_HEAD",
                "REVERT_HEAD",
                "rebase-apply",
                "rebase-merge",
            ] {
                ensure!(
                    !repo.path().join(marker).exists(),
                    "Unfinished repository operation is preserved"
                );
            }
            let paths = request["paths"]
                .as_array()
                .context("Selected paths are required")?;
            ensure!(
                !paths.is_empty() && paths.len() <= 64,
                "Selected path limit"
            );
            let mut selected = BTreeSet::new();
            let mut files = parent_files(&repo)?;
            for path in paths {
                let path = text(path)?;
                path_name(path)?;
                ensure!(selected.insert(path.to_owned()), "Duplicate selected path");
                if let Some(entry) = index.entry_by_path(path.as_bytes().as_bstr()) {
                    ensure!(
                        entry.stage() == gix::index::entry::Stage::Unconflicted,
                        "Selected path has an unresolved conflict"
                    );
                    ensure!(
                        matches!(
                            entry.mode,
                            gix::index::entry::Mode::FILE
                                | gix::index::entry::Mode::FILE_EXECUTABLE
                        ),
                        "Selected entries must be regular files"
                    );
                    files.insert(path.into(), (entry.id, entry.mode.bits()));
                } else {
                    ensure!(
                        files.remove(path).is_some(),
                        "Selected path exists in neither index nor parent"
                    );
                }
            }
            let tree = build_tree(&files, repo.object_hash(), &mut objects)?;
            let actor = actor(&request["author"])?;
            let message = text(&request["message"])?;
            ensure!(
                !message.is_empty() && message.len() <= 65536 && !message.contains('\0'),
                "Invalid commit message"
            );
            let parent = root_head.as_deref().unwrap().parse::<gix::ObjectId>()?;
            let commit = gix::objs::Commit {
                tree,
                parents: vec![parent].into(),
                author: actor.clone(),
                committer: actor.clone(),
                encoding: None,
                message: message.into(),
                extra_headers: Default::default(),
            };
            let id = object_value(&mut objects, repo.object_hash(), &commit)?;
            plan["reference"] = json!(head_ref);
            plan["expected_ref"] = json!(root_head);
            plan["intended_ref"] = json!(id.to_string());
            plan["paths"] = json!(selected);
            plan["actor"] = json!({"name":actor.name.to_string(),"email":actor.email.to_string(),"timestamp":actor.time.seconds,"offset":actor.time.offset});
        }
        "branch" => {
            let short = text(&request["name"])?;
            ensure!(
                !short.is_empty() && !short.starts_with('-'),
                "Invalid branch name"
            );
            let name = format!("refs/heads/{short}");
            let _: gix::refs::FullName = name.clone().try_into()?;
            let current = branch(&repo, &name)?;
            ensure!(
                request.get("expected").is_some() && request["expected"] == json!(current),
                "conflict: Expected branch value changed"
            );
            let action = text(&request["action"])?;
            let target = match action {
                "create" => {
                    ensure!(current.is_none(), "Branch creation requires absence");
                    let id = text(&request["target"])?.parse::<gix::ObjectId>()?;
                    repo.find_commit(id)?;
                    Some(id.to_string())
                }
                "delete" => {
                    ensure!(
                        current.is_some() && head_ref.as_deref() != Some(name.as_str()),
                        "Checked-out or absent branch cannot be deleted"
                    );
                    ensure!(
                        !repo.path().join("worktrees").exists(),
                        "unsupported: auxiliary worktrees need branch-occupancy guards"
                    );
                    None
                }
                _ => bail!("Only branch create and delete are admitted"),
            };
            plan["reference"] = json!(name);
            plan["expected_ref"] = json!(current);
            plan["intended_ref"] = json!(target);
            let actor = actor(&request["author"])?;
            plan["actor"] = json!({"name":actor.name.to_string(),"email":actor.email.to_string(),"timestamp":actor.time.seconds,"offset":actor.time.offset});
        }
        _ => bail!("Unsupported local Git mutation"),
    }
    plan["objects"] = json!(objects);
    ensure!(
        serde_json::to_vec(&plan)?.len() <= BYTES * 3,
        "Prepared mutation exceeds bound"
    );
    Ok(plan)
}

#[cfg(windows)]
fn locked_read(path: &Path) -> Result<Option<File>> {
    use std::os::windows::fs::OpenOptionsExt;
    match OpenOptions::new().read(true).share_mode(1).open(path) {
        Ok(file) => Ok(Some(file)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}
#[cfg(not(windows))]
fn locked_read(path: &Path) -> Result<Option<File>> {
    match File::open(path) {
        Ok(file) => Ok(Some(file)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}
fn publish_objects(repo: &gix::Repository, request: &Value) -> Result<()> {
    let objects = request["objects"]
        .as_array()
        .context("Missing exact object plan")?;
    ensure!(objects.len() <= ENTRIES, "Object count limit");
    let mut total = 0;
    for object in objects {
        let bytes = STANDARD.decode(text(&object["base64"])?)?;
        total += bytes.len();
        ensure!(total <= BYTES * 2, "Object byte limit");
        let kind = match text(&object["kind"])? {
            "blob" => gix::objs::Kind::Blob,
            "tree" => gix::objs::Kind::Tree,
            "commit" => gix::objs::Kind::Commit,
            _ => bail!("Unsupported planned object kind"),
        };
        let id = gix::objs::compute_hash(repo.object_hash(), kind, &bytes)?;
        ensure!(
            json!(id.to_string()) == object["id"],
            "Object payload changed after approval"
        );
        let hex = id.to_string();
        let directory = repo.path().join("objects").join(&hex[..2]);
        match fs::create_dir(&directory) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
        let _directory = metadata_file(&directory)?;
        let destination = directory.join(&hex[2..]);
        let _existing = if destination.exists() {
            Some(metadata_file(&destination)?)
        } else {
            None
        };
        ensure!(
            repo.write_object(Raw { kind, data: &bytes })?.detach() == id,
            "Object store identity mismatch"
        );
        let actual = repo.find_object(id)?;
        ensure!(
            actual.kind == kind && actual.data == bytes,
            "Stored Git object differs from approved bytes"
        );
    }
    Ok(())
}
fn apply(request: &Value, private: &Path) -> Result<Value> {
    let root = Path::new(text(&request["root"])?);
    let repo = open(root, private)?;
    let _metadata = metadata_guards(repo.workdir().unwrap())?;
    ensure!(
        request["format"] == "relai-git-local-v1"
            && request["repository_identity"] == repository_identity(&repo)?,
        "conflict: Repository identity or implementation changed"
    );
    let _head_lock = gix::lock::Marker::acquire_to_hold_resource(
        repo.path().join("HEAD"),
        gix::lock::acquire::Fail::Immediately,
        None,
    )?;
    let _head_handle = locked_read(&repo.path().join("HEAD"))?.context("Missing HEAD")?;
    let (current, head_ref) = head(&repo)?;
    ensure!(
        request["head_ref"] == json!(head_ref),
        "conflict: HEAD changed branches"
    );
    let operation = text(&request["operation"])?;
    if operation == "stage" {
        let _index_lock = gix::lock::Marker::acquire_to_hold_resource(
            repo.index_path(),
            gix::lock::acquire::Fail::Immediately,
            None,
        )?;
        ensure!(
            request["expected_head"] == json!(current)
                && request["expected_index"]
                    == json!(bytes(&repo.index_path())?.as_deref().map(sha)),
            "conflict: Head or index changed"
        );
        publish_objects(&repo, request)?;
        let expected = if request["expected_index"].is_null() {
            json!({"kind":"absent"})
        } else {
            json!({"kind":"file","digest":request["expected_index"]})
        };
        let raw = json!({"operation":"batch","checkout":{"root":root,"revision":current},"operations":[{"operation":"write","path":repo.index_path(),"expected":expected,"intended":{"kind":"file","digest":request["intended_index"]},"base64":request["index_base64"]}]});
        let result = crate::file_effect::execute(&raw, private)?;
        return Ok(
            json!({"state":result["state"],"native":result,"index_digest":request["intended_index"],"route":"gix-index/windows-txf"}),
        );
    }
    ensure!(
        matches!(operation, "commit" | "branch"),
        "Unsupported Git mutation"
    );
    let _index_lock = if operation == "commit" {
        Some(gix::lock::Marker::acquire_to_hold_resource(
            repo.index_path(),
            gix::lock::acquire::Fail::Immediately,
            None,
        )?)
    } else {
        None
    };
    let _index_handle = if operation == "commit" {
        locked_read(&repo.index_path())?
    } else {
        None
    };
    if operation == "commit" {
        ensure!(
            request["expected_head"] == json!(current)
                && request["expected_index"]
                    == json!(bytes(&repo.index_path())?.as_deref().map(sha)),
            "conflict: Head or index changed"
        );
    }
    let name = text(&request["reference"])?;
    ensure!(
        request["expected_ref"] == json!(branch(&repo, name)?),
        "conflict: Branch changed"
    );
    if operation == "branch" && request["intended_ref"].is_null() {
        ensure!(
            head_ref.as_deref() != Some(name) && !repo.path().join("worktrees").exists(),
            "Checked-out branches are preserved"
        );
    }
    publish_objects(&repo, request)?;
    use gix::refs::{
        Target,
        transaction::{Change, LogChange, PreviousValue, RefEdit, RefLog},
    };
    let previous = if request["expected_ref"].is_null() {
        PreviousValue::MustNotExist
    } else {
        PreviousValue::MustExistAndMatch(Target::Object(text(&request["expected_ref"])?.parse()?))
    };
    let change = if request["intended_ref"].is_null() {
        Change::Delete {
            expected: previous,
            log: RefLog::AndReference,
        }
    } else {
        Change::Update {
            expected: previous,
            new: Target::Object(text(&request["intended_ref"])?.parse()?),
            log: LogChange {
                mode: RefLog::AndReference,
                force_create_reflog: true,
                message: "RelAI approved local mutation".into(),
            },
        }
    };
    let actor = actor(&request["actor"])?;
    let mut time_buffer = gix::date::parse::TimeBuf::default();
    repo.edit_references_as(
        [RefEdit {
            name: name.try_into()?,
            deref: false,
            change,
        }],
        Some(actor.to_ref(&mut time_buffer)),
    )
    .context("uncertain: Ref transaction outcome needs reconciliation")?;
    Ok(
        json!({"state":"completed","reference":name,"old":request["expected_ref"],"new":request["intended_ref"],"index_preserved":operation=="commit","route":"gix-ref-transaction"}),
    )
}
fn reconcile(request: &Value, private: &Path) -> Result<Value> {
    let repo = open(Path::new(text(&request["root"])?), private)?;
    let _metadata = metadata_guards(repo.workdir().unwrap())?;
    ensure!(
        request["repository_identity"] == repository_identity(&repo)?,
        "Repository identity changed; recovery is held"
    );
    let (current, reference) = head(&repo)?;
    let index = bytes(&repo.index_path())?.as_deref().map(sha);
    let state = if request["operation"] == "stage" {
        if json!(index) == request["intended_index"] {
            "completed"
        } else if json!(index) == request["expected_index"]
            && json!(current) == request["expected_head"]
            && json!(reference) == request["head_ref"]
        {
            "old"
        } else {
            "conflict"
        }
    } else {
        let actual = branch(&repo, text(&request["reference"])?)?;
        if json!(actual) == request["intended_ref"] {
            "completed"
        } else if json!(actual) == request["expected_ref"]
            && (request["operation"] == "branch"
                || (json!(index) == request["expected_index"]
                    && json!(current) == request["expected_head"]))
            && json!(reference) == request["head_ref"]
        {
            "old"
        } else {
            "conflict"
        }
    };
    Ok(
        json!({"state":state,"observed_head":current,"observed_index":index,"route":"gix-local-reconciliation"}),
    )
}
pub fn execute(request: &Value, private: &Path) -> Result<Value> {
    if request["operation"] == "reconcile" {
        return reconcile(&request["request"], private);
    }
    match apply(request, private) {
        Ok(value) => Ok(value),
        Err(error) => {
            let message = format!("{error:#}");
            Ok(
                json!({"state":if message.contains("uncertain:"){"uncertain"}else if message.contains("conflict:"){"conflict"}else if message.contains("unsupported:"){"unsupported"}else{"failed"},"error":message,"route":"gix-local"}),
            )
        }
    }
}
