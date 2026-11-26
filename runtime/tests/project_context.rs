use fluxr_app_host::App;
use serde_json::{Value, json};
use std::{fs, path::Path};
fn fixture() -> (tempfile::TempDir, App) {
    let t = tempfile::tempdir().unwrap();
    let a = App::open(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../app"),
        &t.path().join("state"),
    )
    .unwrap();
    (t, a)
}
fn open(a: &mut App, path: &Path) -> Value {
    fs::create_dir(path).unwrap();
    a.invoke("open_project", json!({"root":path})).unwrap()["id"].clone()
}
#[test]
fn linked_lookup_preserves_revisions_and_unique_unicode_spans() {
    let (t, mut a) = fixture();
    let p = open(&mut a, &t.path().join("one"));
    let other = open(&mut a, &t.path().join("two"));
    let original=a.invoke("dictionary",json!({"project":p,"op":"save","name":"Café","kind":"knowledge","definition":"First wording."})).unwrap();
    a.invoke(
        "dictionary",
        json!({"project":other,"op":"link","dictionary":p}),
    )
    .unwrap();
    let refs = json!([original.clone(), original.clone()]);
    let get = |a: &mut App| {
        a.invoke(
            "dictionary",
            json!({"project":other,"op":"expand","refs":refs}),
        )
        .unwrap()
    };
    let result = get(&mut a);
    assert_eq!(result.as_array().unwrap().len(), 1);
    assert_eq!(result[0]["definition"]["definition"], "First wording.");
    let arg =
        json!({"project":other,"text":"Cafe\u{301} ✦ and Café.","attachments":[],"key":"mark"});
    a.invoke("submit", arg).unwrap();
    let job = a.invoke("poll", Value::Null).unwrap();
    a.invoke("seal", job.clone()).unwrap();
    a.invoke("poll", Value::Null).unwrap();
    let input = t
        .path()
        .join("state/turns")
        .join(job["job"].as_str().unwrap())
        .join("input.json");
    let data: Value = serde_json::from_slice(&fs::read(input).unwrap()).unwrap();
    let parts = data["parts"].as_array().unwrap();
    assert_eq!(
        parts
            .iter()
            .map(|v| v["text"].as_str().unwrap())
            .collect::<String>(),
        "Cafe\u{301} ✦ and Café."
    );
    assert_eq!(parts.iter().filter(|v| v.get("term").is_some()).count(), 2);
    a.invoke("dictionary",json!({"project":p,"op":"save","id":original["id"],"revision":1,"name":"Café","definition":"Revised.","kind":"knowledge"})).unwrap();
    assert_eq!(get(&mut a)[0]["definition"]["revision"], 1);
    a.invoke(
        "dictionary",
        json!({"project":other,"op":"unlink","dictionary":p}),
    )
    .unwrap();
    assert!(get(&mut a)[0]["definition"].is_null());
}
#[test]
fn snapshot_inspection_reads_retained_bytes_after_source_removal() {
    let (t, mut a) = fixture();
    let root = t.path().join("work");
    let p = open(&mut a, &root);
    fs::write(root.join("draft.txt"), "original").unwrap();
    let snap = a.invoke("capture_graph", json!({"project":p})).unwrap();
    fs::remove_file(root.join("draft.txt")).unwrap();
    let v = a
        .invoke(
            "dashboard",
            json!({"project":p,"view":"graph","snapshot":snap["snapshot"],"selected":"draft.txt"}),
        )
        .unwrap();
    assert!(v.to_string().contains("original"));
}
#[test]
fn skill_uses_pinned_content_and_reports_removed_revision() {
    let (t, mut a) = fixture();
    let p = open(&mut a, &t.path().join("work"));
    let folder = t.path().join("style");
    fs::create_dir(&folder).unwrap();
    let file = folder.join("SKILL.md");
    fs::write(
        &file,
        "---\nname: style\ndescription: Writing terms\n---\nUse short words.",
    )
    .unwrap();
    let installed = a
        .invoke("skill", json!({"project":p,"op":"install","path":folder}))
        .unwrap();
    fs::write(file, "updated outside RelAI").unwrap();
    let data = a
        .invoke(
            "skill",
            json!({"project":p,"op":"inspect","id":installed["id"]}),
        )
        .unwrap();
    assert_eq!(data["body"], "Use short words.");
    a.invoke(
        "skill",
        json!({"project":p,"op":"remove","id":installed["id"]}),
    )
    .unwrap();
    assert!(a.invoke("submit",json!({"project":p,"text":"Use style","key":"a","attachments":[],"skills":[installed["id"]]})).is_err());
}
#[test]
fn graph_does_not_read_a_different_projects_snapshot() {
    let (t, mut a) = fixture();
    let p = open(&mut a, &t.path().join("one"));
    let q = open(&mut a, &t.path().join("two"));
    let snap = a.invoke("capture_graph", json!({"project":p})).unwrap();
    assert!(
        a.invoke(
            "dashboard",
            json!({"project":q,"view":"graph","snapshot":snap["snapshot"]})
        )
        .is_err()
    );
}
#[test]
fn explicit_selection_disambiguates_two_dictionaries_and_validates_exact_utf8_spans() {
    let (t, mut a) = fixture();
    let p = open(&mut a, &t.path().join("one"));
    let other = open(&mut a, &t.path().join("two"));
    for project in [&p, &other] {
        a.invoke("dictionary",json!({"project":project,"op":"save","name":"Café","kind":"knowledge","definition":"A local definition."})).unwrap();
    }
    a.invoke(
        "dictionary",
        json!({"project":p,"op":"link","dictionary":other}),
    )
    .unwrap();
    let matches = a
        .invoke("suggest_terms", json!({"project":p,"prefix":"Caf"}))
        .unwrap();
    assert_eq!(matches.as_array().unwrap().len(), 2);
    let chosen = matches
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["dictionary_id"] == other)
        .unwrap();
    let marker = json!({"start":5,"end":10,"ref":chosen});
    let body = "🍵 Café!";
    let message = |key: &str, markers: Value| json!({"project":p,"text":body,"attachments":[],"key":key,"markers":markers});
    assert!(
        a.invoke("submit", message("overlap", json!([marker, marker])))
            .is_err()
    );
    let mut broken = marker.clone();
    broken["start"] = json!(1);
    assert!(
        a.invoke("submit", message("invalid", json!([broken])))
            .is_err()
    );
    a.invoke("submit", message("valid", json!([marker])))
        .unwrap();
    let job = a.invoke("poll", Value::Null).unwrap();
    a.invoke("seal", job.clone()).unwrap();
    a.invoke("poll", Value::Null).unwrap();
    let frame: Value = serde_json::from_slice(
        &fs::read(
            a.data
                .join("turns")
                .join(job["job"].as_str().unwrap())
                .join("input.json"),
        )
        .unwrap(),
    )
    .unwrap();
    let parts = frame["parts"].as_array().unwrap();
    assert_eq!(
        parts
            .iter()
            .filter_map(|i| i["text"].as_str())
            .collect::<String>(),
        body
    );
    assert!(
        parts
            .iter()
            .any(|v| v["term"]["dictionary_id"] == other && v["term"]["revision"] == 1)
    );
}
