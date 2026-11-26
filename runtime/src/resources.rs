use super::{join, linked, read, relative, text};
use anyhow::{Context, Result, ensure};
use serde_json::{Map, Value, json};
use std::{collections::BTreeMap, fs, io::Read, path::Path};
use unicode_normalization::UnicodeNormalization;
use unicode_segmentation::UnicodeSegmentation;
const CAP: usize = 2 * 1024 * 1024;
pub fn package(path: &Path, private: &Path) -> Result<Value> {
    let root = path.canonicalize()?;
    ensure!(
        !root.starts_with(private) && !private.starts_with(&root),
        "Private storage unavailable"
    );
    let mut files = BTreeMap::new();
    let mut budget = CAP;
    let mut count = 0;
    if root.is_dir() {
        let mut dirs = vec![root.clone()];
        while let Some(dir) = dirs.pop() {
            for entry in fs::read_dir(dir)? {
                let p = entry?.path();
                count += 1;
                ensure!(count <= 128, "Package entry limit");
                let meta = fs::symlink_metadata(&p)?;
                ensure!(!linked(&meta), "Unexpected package link");
                if meta.is_dir() {
                    dirs.push(p);
                    continue;
                }
                let key = p
                    .strip_prefix(&root)?
                    .to_str()
                    .context("Invalid filename")?
                    .replace('\\', "/");
                ensure!(meta.len() <= budget as u64, "Package exceeds byte budget");
                let bytes = read(&join(&root, &key)?)?;
                budget = budget.checked_sub(bytes.len()).context("Package grew")?;
                files.insert(key, String::from_utf8(bytes)?);
            }
        }
    } else {
        let mut zip = zip::ZipArchive::new(fs::File::open(root)?)?;
        ensure!(zip.len() <= 128, "Package entry limit");
        for i in 0..zip.len() {
            let mut file = zip.by_index(i)?;
            let name = file.name().replace('\\', "/");
            let path = name.trim_end_matches('/');
            relative(path)?;
            ensure!(
                !path.is_empty() && file.enclosed_name().is_some() && !file.is_symlink(),
                "Invalid archive member"
            );
            if file.is_dir() {
                continue;
            }
            ensure!(file.size() <= budget as u64, "Package exceeds byte budget");
            let mut bytes = Vec::new();
            (&mut file)
                .take(budget as u64 + 1)
                .read_to_end(&mut bytes)?;
            budget = budget
                .checked_sub(bytes.len())
                .context("Package expands beyond limit")?;
            ensure!(
                files.insert(name, String::from_utf8(bytes)?).is_none(),
                "Duplicate package member"
            );
        }
    }
    Ok(json!(files))
}
pub fn frontmatter(input: &str) -> Result<Value> {
    let s = input.replace("\r\n", "\n");
    let (meta, body) = s
        .strip_prefix("---\n")
        .and_then(|r| r.split_once("\n---\n"))
        .context("Missing YAML front matter")?;
    let attributes: Map<String, Value> = serde_yaml::from_str(meta)?;
    Ok(json!({"attributes":attributes,"body":body}))
}
/// Return exact, whole-word byte ranges for nonambiguous names in a bounded vocabulary.
pub fn matches(input: &str, names: &Value) -> Result<Value> {
    ensure!(input.len() <= 262144, "Text exceeds span limit");
    let mut terms = BTreeMap::<String, Vec<&Value>>::new();
    let norm = |s: &str| s.nfc().flat_map(char::to_lowercase).collect::<String>();
    for row in names.as_array().context("Expected vocabulary")? {
        let name = text(&row["name"])?;
        ensure!(name.len() <= 256, "Term too long");
        terms.entry(norm(name)).or_default().push(row);
    }
    let mut out = Vec::new();
    let segments = input.split_word_bound_indices().collect::<Vec<_>>();
    let mut i = 0;
    let mut fence = false;
    let mut inline = false;
    while i < segments.len() {
        let (start, word) = segments[i];
        let tail = &input[start..];
        if tail.starts_with("```") {
            fence = !fence;
            i += 3;
            continue;
        }
        if word == "`" && !fence {
            inline = !inline;
            i += 1;
            continue;
        }
        if tail.starts_with("https://") || tail.starts_with("http://") {
            let end = start + tail.find(char::is_whitespace).unwrap_or(tail.len());
            while i < segments.len() && segments[i].0 < end {
                i += 1
            }
            continue;
        }
        if fence || inline || !word.chars().any(char::is_alphanumeric) {
            i += 1;
            continue;
        }
        let mut best = None;
        for (j, (offset, word)) in segments.iter().enumerate().skip(i) {
            let end = offset + word.len();
            if end - start > 512 {
                break;
            }
            let normalized = norm(&input[start..end]);
            if let Some(hits) = terms.get(&normalized) {
                best = Some((j, end, hits))
            }
            if !terms
                .range(normalized.clone()..)
                .next()
                .is_some_and(|(key, _)| key.starts_with(&normalized))
            {
                break;
            }
        }
        if let Some((j, end, hits)) = best {
            if hits.len() == 1 {
                out.push(json!({"start":start,"end":end,"ref":hits[0]}));
            }
            i = j + 1
        } else {
            i += 1
        }
    }
    Ok(json!(out))
}
