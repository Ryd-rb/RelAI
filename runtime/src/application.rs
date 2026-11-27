use crate::{join, read, text};
use anyhow::{Context, Result, ensure};
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

/// A manifest can pin its text resources to the same launch as the linked code.
pub struct Assets(Option<BTreeMap<String, String>>);

impl Assets {
    pub fn load(bundle: &Path, config: &Value) -> Result<Self> {
        let Some(list) = config.get("assets") else {
            return Ok(Self(None));
        };
        let mut entries = BTreeMap::new();
        let mut bytes = 0usize;
        for item in list
            .as_array()
            .context("Expected an application asset list")?
        {
            let name = text(item)?;
            let value = String::from_utf8(read(&join(bundle, name)?)?)?;
            bytes = bytes
                .checked_add(value.len())
                .context("Application asset size overflow")?;
            ensure!(
                bytes <= crate::LIMIT,
                "Application assets exceed the bundle byte limit"
            );
            ensure!(
                entries.insert(name.to_owned(), value).is_none(),
                "Duplicate application asset {name}"
            );
        }
        Ok(Self(Some(entries)))
    }

    pub fn get(&self, bundle: &Path, name: &str) -> Result<String> {
        if let Some(entries) = &self.0 {
            return entries
                .get(name)
                .cloned()
                .with_context(|| format!("Unknown application asset {name}"));
        }
        Ok(String::from_utf8(read(&join(bundle, name)?)?)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::App;
    use serde_json::json;
    use std::fs;

    #[test]
    fn asset_version_stays_with_the_open_application_until_restart() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let bundle = temp.path().join("bundle");
        let data = temp.path().join("data");
        fs::create_dir(&bundle)?;
        fs::write(
            bundle.join("app.json"),
            r#"{"modules":["sample.fluxr"],"assets":["message.txt"]}"#,
        )?;
        fs::write(
            bundle.join("sample.fluxr"),
            r#"fluxr 1
pub init 1 arg uint uint ret 0u
pub message 1 arg uint uint ret host asset uint 1 uint host text uint 1 list uint utf8 "message.txt"
"#,
        )?;
        fs::write(bundle.join("message.txt"), "first")?;
        let mut application = App::open(&bundle, &data)?;
        fs::write(bundle.join("message.txt"), "second")?;
        assert_eq!(application.invoke("message", Value::Null)?, json!("first"));
        drop(application);
        assert_eq!(
            App::open(&bundle, &data)?.invoke("message", Value::Null)?,
            json!("second")
        );
        Ok(())
    }
}
