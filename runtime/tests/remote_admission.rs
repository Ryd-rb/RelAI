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

#[test]
fn foreground_push_pins_registered_remote_and_replays_without_resending() -> Result<()> {
    let tmp = tempfile::tempdir()?;
    let root = tmp.path().join("repo");
    fs::create_dir(&root)?;
    let data = tmp.path().join("state");
    let bundle = Path::new(env!("CARGO_MANIFEST_DIR")).join("../app");
    let mut app = App::open(&bundle, &data)?;
    let pid = app.invoke("open_project", json!({"root":root}))?["id"].clone();
    let remote = app.invoke("remote", json!({"project":pid,"op":"register","url":"https://example.invalid/repo","branch":"refs/heads/main","credential":"RELAI_GIT_FIXTURE_SECRET"}))?["id"].clone();
    let listed = app.invoke(
        "tool_run",
        json!({"project":pid,"tool":"git.remote_list","arguments":{}}),
    )?;
    assert_eq!(
        listed,
        json!([{"id":remote,"url":"https://example.invalid/repo","branch":"refs/heads/main"}])
    );
    let db = rusqlite::Connection::open(data.join("state.sqlite3"))?;
    db.execute(
        "INSERT INTO requests(id,project,intent) VALUES('push-request',?,'Push my commit')",
        [pid.as_str().unwrap()],
    )?;
    db.execute(
        "INSERT INTO submissions VALUES('push-submission',?,'push-request','push-key','hash','{}')",
        [pid.as_str().unwrap()],
    )?;
    let target = "A".repeat(40);
    let body = json!({"operation":"tool_run","arguments":{"tool":"git.push","arguments":{"remote":remote,"expected":null,"target":target}}});
    db.execute("INSERT INTO jobs(id,project,request,revision,session,role,kind,input,state) SELECT 'push-job',id,'push-request',1,discriminator,'worker','tool',?,'executing' FROM projects WHERE id=?", rusqlite::params![body.to_string(),pid.as_str().unwrap()])?;
    let job = json!({"job":"push-job"});
    assert!(app.invoke("chat_git_push", job.clone()).is_err());
    db.execute(
        "UPDATE jobs SET role='discriminator',session='foreign' WHERE id='push-job'",
        [],
    )?;
    assert!(app.invoke("chat_git_push", job.clone()).is_err());
    db.execute("UPDATE jobs SET session=(SELECT discriminator FROM projects WHERE id=jobs.project),revision=0 WHERE id='push-job'", [])?;
    assert!(app.invoke("chat_git_push", job.clone()).is_err());
    db.execute("UPDATE jobs SET revision=1 WHERE id='push-job'", [])?;
    for arguments in [
        json!({"remote":remote,"target":target}),
        json!({"remote":remote,"expected":null,"target":target,"force":true}),
        json!({"remote":remote,"expected":null,"target":"not-a-sha"}),
        json!({"remote":"foreign","expected":null,"target":target}),
    ] {
        let mut invalid = body.clone();
        invalid["arguments"]["arguments"] = arguments;
        db.execute(
            "UPDATE jobs SET input=? WHERE id='push-job'",
            [invalid.to_string()],
        )?;
        assert!(app.invoke("chat_git_push", job.clone()).is_err());
    }
    db.execute(
        "UPDATE jobs SET input=? WHERE id='push-job'",
        [body.to_string()],
    )?;
    let plan = app.invoke("chat_git_push", job.clone())?;
    assert_eq!(plan["$smart"]["target"], target.to_ascii_lowercase());
    assert_eq!(plan["$smart"]["expected"], serde_json::Value::Null);
    assert_eq!(plan["$smart"]["branch"], "refs/heads/main");
    assert_eq!(plan["$smart"]["credential"], "RELAI_GIT_FIXTURE_SECRET");
    let operation = plan["state"]["operation"].as_str().unwrap();
    let replay = app.invoke("chat_git_push", job.clone())?;
    assert_eq!(replay["state"], "uncertain");
    assert_eq!(replay["operation"], operation);
    assert!(replay.get("$smart").is_none());
    db.execute("INSERT INTO jobs(id,project,request,revision,session,role,kind,input,state) SELECT 'push-second',project,request,revision,session,role,kind,input,state FROM jobs WHERE id='push-job'", [])?;
    assert!(
        app.invoke("chat_git_push", json!({"job":"push-second"}))
            .is_err()
    );
    db.execute(
        "UPDATE git_transfers SET state='uncertain' WHERE id=?",
        [operation],
    )?;
    assert!(
        app.invoke("chat_git_push", json!({"job":"push-second"}))
            .is_err()
    );
    let status_plan = app.invoke(
        "tool_run",
        json!({"project":pid,"tool":"git.push_status","arguments":{"operation":operation}}),
    )?;
    assert_eq!(status_plan["$smart"]["op"], "reconcile");
    assert_eq!(status_plan["state"]["reconcile"], true);
    assert_eq!(status_plan["state"]["operation"], operation);
    assert_eq!(
        db.query_row(
            "SELECT state FROM git_transfers WHERE id=?",
            [operation],
            |r| r.get::<_, String>(0)
        )?,
        "uncertain"
    );
    assert!(
        app.invoke(
            "tool_run",
            json!({"project":pid,"tool":"git.push_status","arguments":{"operation":"foreign"}})
        )
        .is_err()
    );
    let uncertain = app.invoke("chat_git_push", job.clone())?;
    assert_eq!(uncertain["state"], "uncertain");
    assert!(uncertain.get("$smart").is_none());
    db.execute(
        "UPDATE git_transfers SET state='sending' WHERE id=?",
        [operation],
    )?;
    let receipt = json!({"state":"pushed","target":target.to_ascii_lowercase()});
    app.invoke(
        "guest_transfer_done",
        json!({"state":plan["state"],"result":receipt,"error":null}),
    )?;
    app.invoke("remote", json!({"project":pid,"op":"revoke","id":remote}))?;
    assert!(
        app.invoke(
            "tool_run",
            json!({"project":pid,"tool":"git.push_status","arguments":{"operation":operation}})
        )
        .is_err()
    );
    let replay = app.invoke("chat_git_push", job.clone())?;
    assert_eq!(replay["state"], "pushed");
    assert_eq!(replay["operation"], operation);
    assert_eq!(
        db.query_row("SELECT count(*) FROM git_transfers", [], |r| r
            .get::<_, i64>(0))?,
        1
    );
    assert_eq!(
        app.invoke(
            "tool_run",
            json!({"project":pid,"tool":"git.remote_list","arguments":{}})
        )?,
        json!([])
    );
    db.execute("UPDATE requests SET revision=2 WHERE id='push-request'", [])?;
    assert!(app.invoke("chat_git_push", job).is_err());
    Ok(())
}
