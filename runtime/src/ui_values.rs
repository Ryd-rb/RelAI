use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};

/// Byte boundaries for a Unicode edit and the identifier preceding a UTF-16 caret.
pub(crate) fn edit_span(old: &str, new: &str, caret: &Value) -> Result<Value> {
    ensure!(
        old.len().max(new.len()) <= 1024 * 1024,
        "Text field exceeds 1 MiB"
    );
    let caret_byte = caret.as_u64().and_then(|end| {
        let mut units = 0;
        for (pos, ch) in new.char_indices() {
            if units == end {
                return Some(pos);
            }
            units += ch.len_utf16() as u64;
            if units > end {
                return None;
            }
        }
        (units == end).then_some(new.len())
    });
    // When repeated spans match equally well, the insertion point distinguishes
    // removal of the first occurrence from removal of the last occurrence.
    let prefix: usize = old
        .chars()
        .zip(new[..caret_byte.unwrap_or(new.len())].chars())
        .take_while(|(a, b)| a == b)
        .map(|(c, _)| c.len_utf8())
        .sum();
    let suffix: usize = old[prefix..]
        .chars()
        .rev()
        .zip(new[prefix..].chars().rev())
        .take_while(|(a, b)| a == b)
        .map(|(c, _)| c.len_utf8())
        .sum();
    let mut result = json!({"start":prefix,"old_end":old.len()-suffix,"new_end":new.len()-suffix});
    if let Some(pos) = caret_byte {
        let n: usize = new[..pos]
            .chars()
            .rev()
            .take_while(|c| {
                c.is_alphanumeric()
                    || *c == '_'
                    || *c == '-'
                    || unicode_normalization::char::is_combining_mark(*c)
            })
            .map(char::len_utf8)
            .sum();
        if (1..=100).contains(&n) {
            result["word_start"] = json!(pos - n);
            result["word_end"] = json!(pos);
            result["prefix"] = json!(&new[pos - n..pos]);
        }
    }
    Ok(result)
}
pub(crate) fn merged(base: &Value, overlay: &Value) -> Result<Value> {
    let mut record = base.as_object().cloned().unwrap_or_default();
    if !overlay.is_null() {
        for (k, v) in overlay.as_object().context("Expected a record")? {
            record.insert(k.clone(), v.clone());
        }
    }
    Ok(Value::Object(record))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn repeated_terms_use_the_real_caret_to_identify_the_deleted_occurrence() {
        let left = edit_span("étéété", "été", &json!(0)).unwrap();
        let right = edit_span("étéété", "été", &json!(3)).unwrap();
        assert_eq!(
            (left["start"].as_u64(), left["old_end"].as_u64()),
            (Some(0), Some(5))
        );
        assert_eq!(
            (right["start"].as_u64(), right["old_end"].as_u64()),
            (Some(5), Some(10))
        );
    }
}
