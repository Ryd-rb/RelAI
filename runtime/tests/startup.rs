use fluxr_app_host::App;
use serde_json::json;
#[test]
fn opens_the_existing_project_and_reopens_the_same_relay() {
    let temp = tempfile::tempdir().unwrap();
    let repo = temp.path().join("work");
    std::fs::create_dir(&repo).unwrap();
    let mut app = App::open(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../app"),
        &temp.path().join("db"),
    )
    .unwrap();
    let p = app.invoke("open_project", json!({"root":repo})).unwrap();
    assert_eq!(p, app.invoke("open_project", json!({"root":repo})).unwrap());
    assert_eq!(0, std::fs::read_dir(repo).unwrap().count());
}
