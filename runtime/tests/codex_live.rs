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
    let (count,msg):(i64,String)=db.query_row("SELECT count(*),coalesce(json_extract(payload,'$.findings'),'') FROM finals JOIN actions ON actions.id=finals.action",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(count, 1);
    assert!(msg.to_lowercase().contains("marigold"));
    let foreground = start.elapsed();
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

#[tokio::test]
#[ignore = "requires ChatGPT sign-in and consumes subscription usage"]
async fn subscription_reads_open_project() {
    let temp = tempfile::tempdir().unwrap();
    let work = temp.path().join("project");
    fs::create_dir(&work).unwrap();
    let display_name = format!("Orchard-{}", uuid::Uuid::new_v4().simple());
    fs::write(
        work.join("README.md"),
        format!("Display name: {display_name}\n"),
    )
    .unwrap();
    let mut app = App::open(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../app"),
        &temp.path().join("db"),
    )
    .unwrap();
    let p = app.invoke("open_project", json!({"root":work})).unwrap()["id"].clone();
    app.invoke("save_model",json!({"project":p,"role":"discriminator","provider":"codex","model":"gpt-6.1-sol","effort":"low"})).unwrap();
    let prompt = "What exact display name does the README give this project?";
    app.invoke(
        "submit",
        json!({"project":p,"key":"open-project","text":prompt,"attachments":[]}),
    )
    .unwrap();
    // Resume the conversation from the upload question issued by earlier versions.
    let seal = app.invoke("poll", Value::Null).unwrap();
    app.invoke("seal", seal).unwrap();
    let intake = app.invoke("poll", Value::Null).unwrap();
    let ask = json!({
        "version":1, "next":"wait_for",
        "dispositions":[{"start":0,"end":prompt.len(),"attachment":null,"disposition":"Find the display name."}],
        "actions":[{"type":"ASK","summary":"Ask for the README.","work":"Identify the display name.","question":"Can you attach the README?","choices":[]}]
    });
    app.invoke("receive", json!({"job":intake["job"],"attempt":intake["attempt"],"response":{"message":{"params":{"item":{"text":ask.to_string()}}}}})).unwrap();
    let db = rusqlite::Connection::open(temp.path().join("db/state.sqlite3")).unwrap();
    let question: String = db
        .query_row("SELECT id FROM questions", [], |r| r.get(0))
        .unwrap();
    app.invoke("submit", json!({"project":p,"key":"answer","text":"I have no attachment. Please check the project I opened.","attachments":[],"reply":question})).unwrap();
    let mut turns = 0;
    let mut read_name = false;
    for _ in 0..16 {
        let job = app.invoke("poll", Value::Null).unwrap();
        match job["local"].as_str() {
            Some("seal") => {
                app.invoke("seal", job).unwrap();
            }
            Some("tool_run") => {
                let args = &job["arguments"];
                assert_eq!(args["project"], p);
                assert!(matches!(
                    args["tool"].as_str(),
                    Some("fs.list" | "fs.read" | "fs.search" | "tool.find")
                ));
                assert!(args["arguments"]["host_path"].is_null());
                let receipt = app.invoke("tool_run", args.clone()).unwrap();
                read_name |= args["tool"] == "fs.read"
                    && receipt["value"]["text"]
                        .as_str()
                        .is_some_and(|text| text.contains(&display_name));
                app.invoke("tool_done", json!({"job":job["job"],"result":receipt}))
                    .unwrap();
            }
            None if !job["transport"].is_null() => {
                turns += 1;
                assert!(
                    turns <= 5,
                    "Project lookup exceeded five subscription turns"
                );
                let response = transport::exchange(&job["transport"], app.metadata())
                    .await
                    .unwrap();
                app.invoke(
                    "receive",
                    json!({"job":job["job"],"attempt":job["attempt"],"response":response}),
                )
                .unwrap();
                let actions: String = db.query_row(
                    "SELECT coalesce(group_concat(coalesce(payload->>'$.tool',kind)||' ('||state||')'),'no actions') FROM actions WHERE job=?",
                    [job["job"].as_str().unwrap()], |r| r.get(0),
                ).unwrap();
                println!("Project lookup turn {turns}: {actions}");
            }
            _ => panic!("Project lookup stopped before producing an answer"),
        }
        let questions: i64 = db
            .query_row("SELECT count(*) FROM questions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            questions, 1,
            "The open project contained all requested information for the reply"
        );
        let answer: Option<String> = db
            .query_row(
                "SELECT max(payload->>'$.findings') FROM finals JOIN actions ON actions.id=finals.action",
                [],
                |r| r.get(0),
            )
            .unwrap();
        if let Some(answer) = answer {
            let state: String = db
                .query_row("SELECT state FROM questions WHERE id=?", [&question], |r| {
                    r.get(0)
                })
                .unwrap();
            assert_eq!(state, "answered");
            assert!(
                read_name && answer.contains(&display_name),
                "Answer must use the name observed in the project"
            );
            println!(
                "Verified that a reply to an existing upload question reached its recipient and returned the README value in {turns} subscription turns without asking again."
            );
            return;
        }
    }
    panic!("Project lookup exceeded its local step budget")
}
