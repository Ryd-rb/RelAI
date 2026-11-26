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
    app.invoke("save_model",json!({"project":p,"role":"discriminator","provider":"codex","model":"gpt-6.1-sol","effort":"low"})).unwrap();
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
    println!(
        "One Codex provider attempt. One direct result. Attachment verified. Elapsed {:.1} s.",
        start.elapsed().as_secs_f64()
    );
}
