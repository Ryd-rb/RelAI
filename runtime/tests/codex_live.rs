use fluxr_app_host::{App, transport};
use serde_json::{Value, json};
use std::{fs, path::Path, time::Instant};

/// Uses the configured authenticated local Codex App Server and subscription.
#[tokio::test]
#[ignore = "requires ChatGPT sign-in and consumes subscription usage"]
async fn subscription_attachment() {
    let temp = tempfile::tempdir().unwrap();
    let work = temp.path().join("notes");
    fs::create_dir(&work).unwrap();
    let mut app = App::open(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../app"),
        &temp.path().join("db"),
    )
    .unwrap();
    let p = app.invoke("open_project", json!({"root":work})).unwrap()["id"].clone();
    for role in ["discriminator", "workflow_improver"] {
        app.invoke("save_model",json!({"project":p,"role":role,"provider":"codex","model":"gpt-6.1-sol","effort":"low"})).unwrap();
    }
    let a = app
        .invoke(
            "capture",
            json!({"project":p,"name":"color.txt","mime":"text/plain","base64":"bWFyaWdvbGQ="}),
        )
        .unwrap();
    app.invoke("submit",json!({"project":p,"key":"live","text":"Return the attached word as a direct final result.","attachments":[a["id"]]})).unwrap();
    let seal = app.invoke("poll", Value::Null).unwrap();
    app.invoke("seal", seal).unwrap();
    let job = app.invoke("poll", Value::Null).unwrap();
    let start = Instant::now();
    let response = transport::exchange(&job["transport"], app.metadata())
        .await
        .unwrap();
    app.invoke(
        "receive",
        json!({"job":job["job"],"attempt":job["attempt"],"response":response}),
    )
    .unwrap();
    let db = rusqlite::Connection::open(temp.path().join("db/state.sqlite3")).unwrap();
    let (count,msg):(i64,String)=db.query_row("SELECT count(*),coalesce(json_extract(payload,'$.findings'),'') FROM actions WHERE state='awaiting_acceptance'",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(count, 1);
    assert!(msg.to_lowercase().contains("marigold"));
    let foreground = start.elapsed();
    let action: String = db
        .query_row(
            "SELECT id FROM actions WHERE state='awaiting_acceptance'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    app.invoke(
        "control",
        json!({"op":"accept","project":p,"action":action}),
    )
    .unwrap();
    let seal = app.invoke("poll", Value::Null).unwrap();
    app.invoke("seal", seal).unwrap();
    let improvement = app.invoke("poll", Value::Null).unwrap();
    let second = Instant::now();
    let response = transport::exchange(&improvement["transport"], app.metadata())
        .await
        .unwrap();
    app.invoke(
        "receive",
        json!({"job":improvement["job"],"attempt":improvement["attempt"],"response":response}),
    )
    .unwrap();
    assert!(app.invoke("poll", Value::Null).unwrap().is_null());
    let counts:(i64,i64,i64,i64)=db.query_row("SELECT (SELECT count(*) FROM attempts),(SELECT count(*) FROM improvements),(SELECT count(*) FROM events WHERE kind='Final result'),(SELECT count(*) FROM actions WHERE json_extract(payload,'$.outcome')='no_change')",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
    assert_eq!(counts, (2, 1, 1, 1));
    println!(
        "Two subscription attempts. Attachment verified and accepted in {:.1} s. One final delivery and one no-change improvement in {:.1} s.",
        foreground.as_secs_f64(),
        second.elapsed().as_secs_f64()
    );
}
