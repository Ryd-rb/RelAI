use crate::text;
use anyhow::{Result, bail, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::path::Path;
pub fn read(root: &Path, private: &Path, operation: &str, args: &Value) -> Result<Value> {
    let path = root.canonicalize()?;
    ensure!(!path.starts_with(private), "Private source");
    let repo = gix::open_opts(path, gix::open::Options::isolated())?;
    ensure!(
        !repo.path().canonicalize()?.starts_with(private),
        "Private repository"
    );
    let common = repo.common_dir().canonicalize()?;
    ensure!(
        !common.starts_with(private) && !common.join("objects/info/alternates").exists(),
        "Private or redirected Git storage"
    );
    crate::join(&common, "objects/pack")?;
    match operation {
        "package" => package(&repo, args),
        "blame" => {
            let commit = text(&args["object"])?.parse::<gix::ObjectId>()?;
            let name = text(&args["path"])?;
            ensure!(
                !name.is_empty() && !name.contains('\\'),
                "Expected a repository-relative Git path"
            );
            crate::relative(name)?;
            // A private bare object store has no checkout filters, diff drivers or user hooks.
            let packed = crate::git_pack::reachable(&repo, commit, None)?;
            let scratch = tempfile::tempdir_in(private)?;
            gix::init_bare(scratch.path())?;
            crate::git_pack::install(&packed, &scratch.path().join("objects/pack"))?;
            let view = gix::open_opts(scratch.path(), gix::open::Options::isolated())?;
            let output = view.blame_file(gix::bstr::BStr::new(name), commit, Default::default())?;
            ensure!(output.entries.len() <= 8192, "Blame hunk limit");
            let hunks=output.entries.into_iter().map(|entry|json!({"commit":entry.commit_id.to_string(),"start":entry.start_in_blamed_file,"source_start":entry.start_in_source_file,"count":entry.len.get(),"source":entry.source_file_name.map(|n|n.to_string())})).collect::<Vec<_>>();
            Ok(
                json!({"object":commit.to_string(),"path":name,"digest":crate::hex(&output.blob),"line_origin":0,"hunks":hunks}),
            )
        }
        "refs" => {
            let mut refs = vec![];
            let platform = repo.references()?;
            for entry in platform.all()? {
                ensure!(refs.len() < 2048, "Ref limit");
                let entry = entry.map_err(|e| anyhow::anyhow!(e.to_string()))?;
                refs.push(json!({"name":entry.name().as_bstr().to_string(),"target":format!("{:?}",entry.target())}))
            }
            Ok(json!(refs))
        }
        "head" => Ok(json!(repo.head_id()?.to_string())),
        "commit" | "object" | "tree" => {
            let id = text(&args["object"])?.parse::<gix::ObjectId>()?;
            ensure!(
                repo.find_header(id)?.size() <= 2 * 1024 * 1024,
                "Git object exceeds limit"
            );
            let object = repo.find_object(id)?;
            match operation {
                "commit" => {
                    let c = object.try_into_commit()?;
                    let d = c.decode()?;
                    Ok(
                        json!({"object":id.to_string(),"tree":d.tree().to_string(),"parents":d.parents().map(|p|p.to_string()).collect::<Vec<_>>(),"message":d.message.to_string()}),
                    )
                }
                "tree" => {
                    let tree = object.try_into_tree()?;
                    let mut entries = Vec::new();
                    for e in tree.iter() {
                        ensure!(entries.len() < 4096, "Tree entry limit");
                        let e = e?;
                        entries.push(json!({"name":e.filename().to_string(),"id":e.oid().to_string(),"directory":e.mode().is_tree(),"mode":format!("{:?}",e.mode())}));
                    }
                    Ok(json!(entries))
                }
                _ => Ok(
                    json!({"object":id.to_string(),"kind":object.kind.to_string(),"size":object.data.len(),"base64":STANDARD.encode(&object.data),"text":std::str::from_utf8(&object.data).ok()}),
                ),
            }
        }
        _ => bail!("Unknown Git object operation"),
    }
}
fn package(repo: &gix::Repository, args: &Value) -> Result<Value> {
    use gix::{bstr::ByteSlice, objs::tree::EntryKind};
    let id = text(&args["object"])?.parse::<gix::ObjectId>()?;
    let prefix = text(&args["path"])?;
    let mut base = repo.find_commit(id)?.tree()?.id;
    if !prefix.is_empty() {
        ensure!(!prefix.contains('\\'), "Use Git paths");
        crate::relative(prefix)?;
        for name in prefix.split('/') {
            let tree = repo.find_tree(base)?;
            let e = tree
                .find_entry(name)
                .ok_or_else(|| anyhow::anyhow!("Missing Git package path"))?;
            ensure!(e.mode().is_tree(), "Expected an ordinary package directory");
            base = e.id().detach();
        }
    }
    let mut files = std::collections::BTreeMap::new();
    let mut names = std::collections::BTreeSet::new();
    let mut dirs = vec![(String::new(), base)];
    let mut total = 0usize;
    let mut count = 0usize;
    while let Some((parent, oid)) = dirs.pop() {
        let tree = repo.find_tree(oid)?;
        for entry in tree.iter() {
            let entry = entry?;
            count += 1;
            ensure!(count <= 128, "Package entry limit");
            let name = entry.filename().to_str()?;
            ensure!(
                !name.is_empty() && !name.contains(['/', '\\', ':']) && name != "." && name != "..",
                "Invalid package entry"
            );
            let path = if parent.is_empty() {
                name.to_owned()
            } else {
                format!("{parent}/{name}")
            };
            ensure!(
                names.insert(path.to_lowercase()),
                "Ambiguous Git package path"
            );
            match entry.mode().kind() {
                EntryKind::Tree => dirs.push((path, entry.oid().to_owned())),
                EntryKind::Blob | EntryKind::BlobExecutable => {
                    let size = repo.find_header(entry.oid())?.size();
                    ensure!(
                        size <= 2 * 1024 * 1024 && size as usize <= 2 * 1024 * 1024 - total,
                        "Git package exceeds bound"
                    );
                    let blob = repo.find_blob(entry.oid())?;
                    total += blob.data.len();
                    ensure!(total <= 2 * 1024 * 1024, "Git package grew");
                    files.insert(path, std::str::from_utf8(&blob.data)?.to_owned());
                }
                _ => bail!("Links and submodules are outside package scope"),
            }
        }
    }
    Ok(json!(files))
}
