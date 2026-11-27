use fluxr_app_host::App;
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
};

fn bundle() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../app")
}
struct Chat {
    _temp: tempfile::TempDir,
    app: App,
    data: PathBuf,
    project: Value,
}
impl Chat {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("project");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("note.txt"), "Project evidence").unwrap();
        let data = temp.path().join("state");
        let mut app = App::open(&bundle(), &data).unwrap();
        let project = app.invoke("open_project", json!({"root":root})).unwrap()["id"].clone();
        Self {
            _temp: temp,
            app,
            data,
            project,
        }
    }
    fn sql(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(self.data.join("state.sqlite3")).unwrap()
    }
    fn submit(&mut self, key: &str, text: &str, question: Option<&str>) {
        self.app.invoke("submit",json!({"project":self.project,"key":key,"text":text,"reply":question,"attachments":[]})).unwrap();
    }
    fn next(&mut self) -> Value {
        for _ in 0..20 {
            let job = self.app.invoke("poll", Value::Null).unwrap();
            if job.is_null() {
                continue;
            }
            if job["local"] == "seal" {
                self.app.invoke("seal", job).unwrap();
                continue;
            }
            assert!(!job["attempt"].is_null(), "Expected model turn: {job}");
            return job;
        }
        panic!("Expected model turn")
    }
    fn respond(&mut self, job: &Value, actions: Value, next: &str) -> anyhow::Result<Value> {
        let (kind, input): (String, String) = self
            .sql()
            .query_row(
                "SELECT kind,input FROM jobs WHERE id=?",
                [job["job"].as_str().unwrap()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        let mut body = json!({"version":1,"actions":actions,"next":next});
        if kind == "intake" {
            let input: Value = serde_json::from_str(&input).unwrap();
            body["dispositions"] = json!([{"start":0,"end":input["text"].as_str().unwrap().len(),"attachment":null,"disposition":"Use this reply."}]);
        }
        self.app.invoke("receive",json!({"job":job["job"],"attempt":job["attempt"],"response":{"message":{"params":{"item":{"text":body.to_string()}}}}}))
    }
    fn questions(&self) -> Vec<(String, String)> {
        self.sql()
            .prepare("SELECT id,body->>'$.question' FROM questions ORDER BY rowid")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }
    fn answer(&mut self, question: &str, answer: &str, key: &str) -> Value {
        self.submit(key, answer, Some(question));
        let intake = self.next();
        assert!(self.delivery(&intake).get("question_batch").is_none());
        let recipient: String = self
            .sql()
            .query_row(
                "SELECT session FROM questions WHERE id=?",
                [question],
                |r| r.get(0),
            )
            .unwrap();
        self.respond(&intake,json!([{"type":"MESSAGE","summary":"Deliver the selected answer.","destination":recipient,"question":question,"payload":answer,"revision":1,"attachments":[]}]),"wait_for").unwrap();
        self.next()
    }
    fn delivery(&self, job: &Value) -> Value {
        let input: Value = serde_json::from_slice(
            &fs::read(
                self.data
                    .join("turns")
                    .join(job["job"].as_str().unwrap())
                    .join("input.json"),
            )
            .unwrap(),
        )
        .unwrap();
        input["delivery"].clone()
    }
    fn frame(&mut self) -> Value {
        let view = self
            .app
            .invoke("dashboard", json!({"project":self.project}))
            .unwrap();
        self.app
            .invoke("desktop_frame", json!({"view":view,"state":{}}))
            .unwrap()
    }
}
fn ask(question: &str, work: &str, choices: Value) -> Value {
    json!({"type":"ASK","summary":"Choose the presentation.","question":question,"work":work,"choices":choices})
}
fn asks() -> Value {
    json!([
        ask(
            "Which button style?",
            "Choose button presentation",
            json!(["Buttons.", "Links."])
        ),
        ask(
            "Which progress style?",
            "Choose progress presentation",
            json!(["Inline.", "Overlay."])
        )
    ])
}

#[test]
fn a_blank_excerpt_without_an_attachment_cannot_answer_an_addressed_question() {
    for blank in ["", " \t", "\u{a0}\u{3000}"] {
        let mut c = Chat::new();
        c.submit("request", "Improve the interface", None);
        let first = c.next();
        c.respond(&first, asks(), "wait_for").unwrap();
        let question = c.questions()[0].0.clone();
        c.submit(
            "answer",
            &format!("Buttons.{blank} private unrelated text"),
            Some(&question),
        );
        let intake = c.next();
        let sql = c.sql();
        let recipient: String = sql
            .query_row(
                "SELECT session FROM questions WHERE id=?",
                [&question],
                |r| r.get(0),
            )
            .unwrap();
        c.respond(
            &intake,
            json!([{"type":"MESSAGE","summary":"Route selected answer.","destination":recipient,
            "question":question,"payload":blank,"revision":1,"attachments":[]}]),
            "wait_for",
        )
        .unwrap();
        let state: String = sql
            .query_row("SELECT state FROM questions WHERE id=?", [&question], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(state, "open");
        let delivered: i64 = sql
            .query_row(
                "SELECT count(*) FROM jobs WHERE kind='message' AND input->>'$.answer_to'=?",
                [&question],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(delivered, 0);
        let held: i64 = sql
            .query_row(
                "SELECT count(*) FROM actions WHERE kind='MESSAGE' AND state='held' AND job=?",
                [intake["job"].as_str().unwrap()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(held, 1);
    }
}

#[test]
fn original_question_picks_survive_partial_wait_or_an_exact_repeated_ask() {
    for (first_index, repeat) in [(0, false), (1, true)] {
        let mut c = Chat::new();
        c.submit("request", "Improve this interface", None);
        let first = c.next();
        c.respond(&first, asks(), "wait_for").unwrap();
        let questions = c.questions();
        assert_eq!(questions.len(), 2);
        let frame = c.frame();
        for (id, text) in &questions {
            assert_eq!(
                frame["controls"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|control| control["role"] == "incoming" && control["value"] == *text)
                    .count(),
                1
            );
            assert!(
                frame["controls"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|control| control["event"]["reply"] == *id)
            );
        }
        let answers = ["Buttons.", "Inline."];
        let second_index = 1 - first_index;
        let recipient = c.answer(
            &questions[first_index].0,
            answers[first_index],
            "first-answer",
        );
        let delivery = c.delivery(&recipient);
        let batch = delivery["question_batch"].as_array().unwrap();
        assert_eq!(batch.len(), 2);
        assert_eq!(delivery["reply_context"].as_object().unwrap().len(), 8);
        for (index, question) in batch.iter().enumerate() {
            assert_eq!(question.as_object().unwrap().len(), 4);
            assert_eq!(question["question_id"], questions[index].0);
            assert_eq!(question["question"], questions[index].1);
            if index == first_index {
                assert_eq!(question["state"], "answer_queued");
                assert_eq!(question["answer"], answers[index]);
            } else {
                assert_eq!(question["state"], "open");
                assert!(question["answer"].is_null());
            }
        }
        if repeat {
            c.respond(
                &recipient,
                json!([asks()[second_index].clone()]),
                "wait_for",
            )
            .unwrap();
        } else {
            c.respond(&recipient, json!([]), "wait_for").unwrap();
        }
        assert_eq!(c.questions().len(), 2);
        let frame = c.frame();
        assert_eq!(
            frame["controls"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|control| control["role"] == "incoming"
                    && control["value"] == questions[second_index].1)
                .count(),
            1
        );
        let recipient = c.answer(
            &questions[second_index].0,
            answers[second_index],
            "second-answer",
        );
        let delivery = c.delivery(&recipient);
        for (index, entry) in delivery["question_batch"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
        {
            assert_eq!(entry["answer"], answers[index]);
            assert_eq!(
                entry["state"],
                if index == second_index {
                    "answer_queued"
                } else {
                    "answered"
                }
            );
        }
        assert!(
            c.respond(&recipient, json!([]), "wait_for").is_err(),
            "No pending sibling remains"
        );
        c.respond(&recipient,json!([{"type":"RESULT","summary":"Read the interface changes.","result_kind":"final","findings":"Use buttons and inline progress.","evidence":[],"outcome":null}]),"complete_when").unwrap();
        assert_eq!(
            c.sql()
                .query_row(
                    "SELECT count(*) FROM questions WHERE state='answered'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            2
        );
        assert_eq!(
            c.sql()
                .query_row("SELECT count(*) FROM actions WHERE state='held'", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            c.sql()
                .query_row("SELECT count(*) FROM finals", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
}

#[test]
fn singleton_reply_context_and_genuinely_new_questions_remain_unchanged() {
    let mut c = Chat::new();
    c.submit("request", "Help", None);
    let first = c.next();
    let original = ask(
        "Which style?",
        "Choose presentation",
        json!(["Buttons.", "Links."]),
    );
    c.respond(&first, json!([original.clone()]), "wait_for")
        .unwrap();
    let question = c.questions()[0].0.clone();
    let recipient = c.answer(&question, "Buttons.", "answer");
    let delivery = c.delivery(&recipient);
    assert!(delivery.get("question_batch").is_none());
    assert_eq!(delivery["reply_context"].as_object().unwrap().len(), 8);
    assert!(c.respond(&recipient, json!([]), "wait_for").is_err());
    // An answered question may be deliberately asked again.
    c.respond(&recipient, json!([original.clone()]), "wait_for")
        .unwrap();
    assert_eq!(c.questions().len(), 2);
    // The next question comes from another envelope and does not create a batch.
    let next_question = c.questions()[1].0.clone();
    let recipient = c.answer(&next_question, "Links.", "answer-again");
    assert!(c.delivery(&recipient).get("question_batch").is_none());
    c.respond(
        &recipient,
        json!([
            ask(
                "Which style?",
                "Choose presentation",
                json!(["Buttons.", "Links."])
            ),
            ask(
                "Which style?",
                "Choose a separate presentation",
                json!(["Buttons.", "Links."])
            ),
            ask(
                "Which style?",
                "Choose presentation",
                json!(["Large buttons.", "Small buttons."])
            )
        ]),
        "wait_for",
    )
    .unwrap();
    assert_eq!(
        c.questions().len(),
        5,
        "Different work or choices must remain independently answerable"
    );
}

#[test]
fn an_unaddressed_tool_continuation_cannot_use_empty_wait_for_pending_questions() {
    let mut c = Chat::new();
    c.submit("request", "Improve", None);
    let first = c.next();
    c.respond(&first, asks(), "wait_for").unwrap();
    let question = c.questions()[0].0.clone();
    let recipient = c.answer(&question, "Buttons.", "answer");
    c.respond(&recipient,json!([{"type":"RUN_TOOL","summary":"Read project context.","tool":"fs.read","arguments_json":json!({"path":"note.txt"}).to_string()}]),"wait_for").unwrap();
    let local = c.app.invoke("poll", Value::Null).unwrap();
    assert_eq!(local["local"], "tool_run");
    let receipt = c
        .app
        .invoke("tool_run", local["arguments"].clone())
        .unwrap();
    c.app
        .invoke(
            "tool_done",
            json!({"job":local["job"],"state":local["state"],"result":receipt}),
        )
        .unwrap();
    let continuation = c.next();
    assert!(c.delivery(&continuation).get("answer_to").is_none());
    assert!(c.respond(&continuation, json!([]), "wait_for").is_err());
}

#[test]
fn a_worker_receives_only_the_admitted_answer_from_a_mixed_submission() {
    let mut c = Chat::new();
    c.submit("request", "Improve the interface", None);
    let intake = c.next();
    c.respond(&intake,json!([{"type":"TASK","summary":"Choose the presentation.","role":"planner","assignment":"Plan interface changes","deliverable":"A presentation plan","criterion":"Use the requested styles","constraints":[],"skills":[],"targets":[],"attachments":[],"prerequisites":[],"sources":[]}]),"wait_for").unwrap();
    let worker = c.next();
    c.respond(&worker, asks(), "wait_for").unwrap();
    let questions = c.questions();
    let review = c.next();
    let review_id: String = c
        .sql()
        .query_row(
            "SELECT id FROM reviews WHERE job=?",
            [review["job"].as_str().unwrap()],
            |r| r.get(0),
        )
        .unwrap();
    let response = json!({"version":1,"actions":[],"next":"wait_for","review_id":review_id,"decision":"wait","achieved":[],"remaining":[questions[0].0,questions[1].0]});
    c.app.invoke("receive",json!({"job":review["job"],"attempt":review["attempt"],"response":{"message":{"params":{"item":{"text":response.to_string()}}}}})).unwrap();
    c.sql()
        .execute(
            "UPDATE questions SET body=json_set(body,'$.private_note','private-question-canary')",
            [],
        )
        .unwrap();
    c.submit(
        "mixed-answer",
        "Buttons. PRIVATE_MIXED_SUBMISSION_CANARY",
        Some(&questions[0].0),
    );
    let intake = c.next();
    let recipient: String = c
        .sql()
        .query_row(
            "SELECT session FROM questions WHERE id=?",
            [&questions[0].0],
            |r| r.get(0),
        )
        .unwrap();
    c.respond(&intake,json!([{"type":"MESSAGE","summary":"Deliver only the selected answer.","destination":recipient,"question":questions[0].0,"payload":"Buttons.","revision":1,"attachments":[]}]),"wait_for").unwrap();
    let addressed = c.next();
    let delivery = c.delivery(&addressed);
    assert_eq!(delivery["payload"], "Buttons.");
    assert_eq!(delivery["question_batch"][0]["answer"], "Buttons.");
    assert!(
        !delivery
            .to_string()
            .contains("PRIVATE_MIXED_SUBMISSION_CANARY")
    );
    assert!(!delivery.to_string().contains("private-question-canary"));
    assert_eq!(delivery["question_batch"][1]["state"], "open");
    c.respond(&addressed, json!([]), "wait_for").unwrap();
}

#[test]
fn another_envelopes_question_cannot_expand_context_or_authorize_an_empty_wait() {
    let mut c = Chat::new();
    c.submit("request", "Improve", None);
    let first = c.next();
    c.respond(&first, asks(), "wait_for").unwrap();
    let questions = c.questions();
    let first_answer = c.answer(&questions[0].0, "Buttons.", "buttons");
    c.respond(
        &first_answer,
        json!([ask(
            "A separate private decision?",
            "Choose another detail",
            json!(["Yes", "No"])
        )]),
        "wait_for",
    )
    .unwrap();
    let second_answer = c.answer(&questions[1].0, "Inline.", "progress");
    let delivery = c.delivery(&second_answer);
    assert_eq!(delivery["question_batch"].as_array().unwrap().len(), 2);
    assert!(
        !delivery
            .to_string()
            .contains("A separate private decision?")
    );
    assert!(c.respond(&second_answer, json!([]), "wait_for").is_err());
    assert_eq!(c.questions().len(), 3);
}

#[test]
fn a_legacy_freeform_question_is_not_repeated_when_choices_are_now_an_empty_array() {
    let mut c = Chat::new();
    c.submit("request", "Improve", None);
    let first = c.next();
    c.respond(
        &first,
        json!([
            ask(
                "Which button style?",
                "Choose button presentation",
                json!(["Buttons.", "Links."])
            ),
            ask("Which style?", "Choose presentation", json!([]))
        ]),
        "wait_for",
    )
    .unwrap();
    let questions = c.questions();
    c.sql()
        .execute(
            "UPDATE questions SET body=json_set(body,'$.choices',NULL) WHERE id=?",
            [&questions[1].0],
        )
        .unwrap();
    let recipient = c.answer(&questions[0].0, "Buttons.", "buttons");
    c.respond(
        &recipient,
        json!([ask("Which style?", "Choose presentation", json!([]))]),
        "wait_for",
    )
    .unwrap();
    assert_eq!(c.questions().len(), 2);
    assert_eq!(
        c.sql()
            .query_row("SELECT count(*) FROM events WHERE kind='ASK'", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        2
    );
}
