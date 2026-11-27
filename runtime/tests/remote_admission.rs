use anyhow::Result;
use fluxr_app_host::App;
use serde_json::json;
use std::{fs, path::Path};

#[test]
fn remote_admission_and_fetch_retirement_are_local() -> Result<()> {
    let tmp = tempfile::tempdir()?;
    let root = tmp.path().join("repo");
    fs::create_dir(&root)?;
    let data = tmp.path().join("state");
    let bundle = Path::new(env!("CARGO_MANIFEST_DIR")).join("../app");
    let mut app = App::open(&bundle, &data)?;
    let pid = app.invoke("open_project", json!({"root":root}))?["id"].clone();
    for (url, branch) in [
        ("http://example.invalid/repo", "refs/heads/main"),
        ("https://example.invalid/repo?q=x", "refs/heads/main"),
        ("https://user@example.invalid/repo", "refs/heads/main"),
        ("https://example.invalid/repo#x", "refs/heads/main"),
        ("https://example.invalid/repo", "refs/heads/a..b"),
        ("https://example.invalid/repo", "main"),
    ] {
        assert!(
            app.invoke(
                "remote",
                json!({"project":pid,"op":"register","url":url,"branch":branch})
            )
            .is_err()
        );
    }
    assert_eq!(
        app.invoke("remote", json!({"project":pid,"op":"list"}))?,
        json!([])
    );
    let remote = app.invoke("remote", json!({"project":pid,"op":"register","url":"https://example.invalid/repo","branch":"refs/heads/main","credential":"RELAI_GIT_UNUSED_FIXTURE_TOKEN"}))?["id"].clone();
    let valid = "A".repeat(40);
    let invalid = "z".repeat(40);
    for (target, expected) in [(&invalid, &valid), (&valid, &invalid)] {
        assert!(app.invoke("remote", json!({"project":pid,"op":"authorize","id":remote,"kind":"fetch","target":target,"expected":expected})).is_err());
    }
    let db = rusqlite::Connection::open(data.join("state.sqlite3"))?;
    assert_eq!(
        db.query_row("SELECT count(*) FROM git_transfers", [], |r| r
            .get::<_, i64>(0))?,
        0
    );
    let op = app.invoke("remote", json!({"project":pid,"op":"authorize","id":remote,"kind":"fetch","target":valid,"expected":valid}))?["operation"].clone();
    let id = op.as_str().unwrap();
    let receipt = r#"{"state":"uncertain","error":"fixture: missing result"}"#;
    db.execute(
        "UPDATE git_transfers SET state='uncertain', receipt=? WHERE id=?",
        [receipt, id],
    )?;
    app.invoke("remote", json!({"project":pid,"op":"revoke","id":remote}))?;
    let retired = app.invoke("remote", json!({"project":pid,"op":"retire_fetch","id":op}))?;
    assert_eq!(retired["state"], "retired");
    assert_eq!(
        retired["receipt"],
        serde_json::from_str::<serde_json::Value>(receipt)?
    );
    let grant = db.query_row(
        "SELECT expected,target,receipt FROM git_transfers WHERE id=?",
        [id],
        |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        },
    )?;
    assert_eq!(
        grant,
        (
            valid.to_ascii_lowercase(),
            valid.to_ascii_lowercase(),
            receipt.into()
        )
    );
    assert!(
        app.invoke("remote", json!({"project":pid,"op":"retire_fetch","id":op}))
            .is_err()
    );
    Ok(())
}
