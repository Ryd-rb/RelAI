use crate::{hex, join, linked, read};
use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::{fs, path::Path};

pub(super) fn source(root: &str, name: &str, host: bool, private: &Path) -> Result<Value> {
    let root = Path::new(root).canonicalize()?;
    let candidate = if host {
        let path = Path::new(name);
        ensure!(path.is_absolute(), "Expected an absolute host path");
        ensure!(
            !path
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir)),
            "Unexpected parent in host path"
        );
        for a in path.ancestors() {
            ensure!(!linked(&fs::symlink_metadata(a)?), "Unexpected linked path");
        }
        path.to_path_buf()
    } else {
        join(&root, name)?
    };
    let path = candidate.canonicalize()?;
    ensure!(
        (host || path.starts_with(&root))
            && !path.starts_with(private)
            && !private.starts_with(&path),
        "Private or escaped source"
    );
    let meta = fs::symlink_metadata(&path)?;
    let common =
        json!({"path":name,"kind":if meta.is_dir(){"directory"}else{"file"},"host_path":host});
    if meta.is_dir() {
        return Ok(common);
    }
    let bytes = read(&path)?;
    let mut out = common;
    out["size"] = json!(bytes.len());
    out["digest"] = json!(hex(&bytes));
    out["base64"] = json!(STANDARD.encode(bytes));
    Ok(out)
}

pub(super) fn regex_spans(body: &str, pattern: &str) -> Result<Value> {
    ensure!(
        !pattern.is_empty() && pattern.len() <= 256 && body.len() <= 65536,
        "Regex input exceeds limits"
    );
    let pattern = regex::RegexBuilder::new(pattern)
        .size_limit(262144)
        .dfa_size_limit(524288)
        .nest_limit(24)
        .build()?;
    let mut starts = vec![0];
    starts.extend(body.match_indices('\n').map(|(index, _)| index + 1));
    let mut matches = Vec::new();
    for found in pattern.find_iter(body).take(257) {
        ensure!(matches.len() < 256, "Regex result exceeds 256 matches");
        let line = starts.partition_point(|pos| *pos <= found.start());
        matches.push(json!({"byte_start":found.start(),"byte_end":found.end(),"line":line,"column":found.start()-starts[line-1]}));
    }
    Ok(Value::Array(matches))
}

pub(super) fn range(encoded: &str, spec: &Value) -> Result<Value> {
    let data = STANDARD.decode(encoded)?;
    if !spec.is_null() {
        ensure!(spec.is_object(), "Expected an object for the range");
    }
    let unit = match spec.get("unit") {
        None => "bytes",
        Some(value) => value.as_str().context("Expected a string range unit")?,
    };
    let index = |key: &str, default: usize| -> Result<usize> {
        match spec.get(key) {
            None => Ok(default),
            Some(n) => Ok(usize::try_from(
                n.as_u64().context("Expected a nonnegative range bound")?,
            )?),
        }
    };
    let (start, end, selected) = if unit == "lines" {
        let content = std::str::from_utf8(&data)?;
        let mut starts = vec![0];
        for (i, b) in data.iter().enumerate() {
            if *b == b'\n' && i + 1 < data.len() {
                starts.push(i + 1)
            }
        }
        let count = if data.is_empty() { 0 } else { starts.len() };
        starts.push(data.len());
        let start = index("start", 0)?;
        let end = index("end", count)?.min(count);
        ensure!(start <= end, "Invalid line range");
        (start, end, &content.as_bytes()[starts[start]..starts[end]])
    } else {
        ensure!(unit == "bytes", "Expected line or byte range");
        let start = index("start", 0)?;
        let end = index("end", data.len())?.min(data.len());
        ensure!(start <= end, "Invalid byte range");
        (start, end, &data[start..end])
    };
    ensure!(
        selected.len() <= 128 * 1024,
        "Selected range exceeds 128 KiB"
    );
    Ok(
        json!({"unit":unit,"start":start,"end":end,"size":data.len(),"digest":hex(&data),"encoding":if std::str::from_utf8(selected).is_ok(){"utf-8"}else{"binary"},"text":std::str::from_utf8(selected).ok(),"base64":STANDARD.encode(selected)}),
    )
}
