use crate::{join, read, text};
use anyhow::{Context, Result};
use fluxr::structured::{Module, SourceDocument};
use serde_json::Value;
use std::{collections::BTreeMap, path::Path};

pub fn link(bundle: &Path, config: &Value) -> Result<BTreeMap<String, Module>> {
    let modules = config["modules"]
        .as_array()
        .context("Missing module list")?;
    let mut sources = Vec::new();
    for module in modules {
        let path = join(bundle, text(module)?)?;
        sources.push(String::from_utf8(read(&path)?)?);
    }
    Ok(SourceDocument::compile_entry_modules(
        sources.iter().map(String::as_str),
    )?)
}
