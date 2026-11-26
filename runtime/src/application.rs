use crate::{join, read, text};
use anyhow::{Context, Result, ensure};
use fluxr::structured::{Module, SourceDocument};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

/// Link each exported call graph separately. Large applications keep the VM's
/// module limits, and each entry has only the functions it can reach.
pub fn link(bundle: &Path, config: &Value) -> Result<BTreeMap<String, Module>> {
    let mut source = SourceDocument::parse("fluxr 1\n")?.source;
    let mut graph = BTreeMap::new();
    for file in config["modules"]
        .as_array()
        .context("Missing module list")?
    {
        let path = join(bundle, text(file)?)?;
        let content = String::from_utf8(read(&path)?)?;
        let doc = SourceDocument::parse(&content).with_context(|| path.display().to_string())?;
        ensure!(!doc.has_native_items(), "Bundle contains native code");
        let parsed = doc.source;
        ensure!(
            parsed.inputs.is_empty(),
            "Application exports use explicit arguments"
        );
        source.records.extend(parsed.records);
        source.enums.extend(parsed.enums);
        for f in parsed.functions {
            ensure!(
                f.receiver.is_none(),
                "Method declarations require a receiver-aware linker"
            );
            let mut calls = BTreeSet::new();
            dependencies(&serde_json::to_value(&f.body)?, &mut calls);
            ensure!(
                graph.insert(f.name.clone(), calls).is_none(),
                "Duplicate function {}",
                f.name
            );
            source.functions.push(f);
        }
    }
    let mut modules = BTreeMap::new();
    for export in source.functions.iter().filter(|f| f.export) {
        let mut pending = vec![export.name.clone()];
        let mut needed = BTreeSet::new();
        while let Some(name) = pending.pop() {
            if needed.insert(name.clone()) {
                pending.extend(
                    graph
                        .get(&name)
                        .with_context(|| format!("Unknown FluxR function {name}"))?
                        .iter()
                        .cloned(),
                )
            }
        }
        let mut linked = source.clone();
        linked.functions.retain(|f| needed.contains(&f.name));
        for f in &mut linked.functions {
            f.export = f.name == export.name
        }
        let mut doc = SourceDocument::parse("fluxr 1\n")?;
        doc.source = linked;
        let m = doc
            .compile()
            .with_context(|| format!("Linking export {}", export.name))?;
        modules.insert(export.name.clone(), m);
    }
    Ok(modules)
}
fn dependencies(node: &Value, out: &mut BTreeSet<String>) {
    match node {
        Value::Object(obj) => {
            if obj.get("op").is_some_and(|v| v == "call")
                && let Some(name) = obj["name"].as_str()
            {
                out.insert(name.into());
            }
            for value in obj.values() {
                dependencies(value, out)
            }
        }
        Value::Array(children) => {
            for child in children {
                dependencies(child, out)
            }
        }
        _ => (),
    }
}
