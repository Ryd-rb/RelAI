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
    match operation {
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
