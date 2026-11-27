#[allow(dead_code)]
#[path = "../src/file_effect.rs"]
mod file_effect;
#[path = "../src/git_operations.rs"]
mod native;
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::{fs, path::PathBuf};
struct Repo {
    _temp: tempfile::TempDir,
    root: PathBuf,
    private: PathBuf,
}
impl Repo {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("repo");
        let private = temp.path().join("private");
        fs::create_dir(&private).unwrap();
        let repo = gix::init(&root).unwrap();
        fs::write(root.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        let a = repo.write_blob(b"a-old\n").unwrap().detach();
        let b = repo.write_blob(b"b-old\n").unwrap().detach();
        let tree = repo
            .write_object(gix::objs::Tree {
                entries: vec![
                    gix::objs::tree::Entry {
                        mode: gix::objs::tree::EntryKind::Blob.into(),
                        filename: "a.txt".into(),
                        oid: a,
                    },
                    gix::objs::tree::Entry {
                        mode: gix::objs::tree::EntryKind::Blob.into(),
                        filename: "b.txt".into(),
                        oid: b,
                    },
                ],
            })
            .unwrap()
            .detach();
        let actor = gix::actor::Signature {
            name: "Fixture".into(),
            email: "fixture@localhost".into(),
            time: gix::date::Time::new(0, 0),
        };
        let mut time = gix::date::parse::TimeBuf::default();
        repo.commit_as(
            actor.to_ref(&mut time.clone()),
            actor.to_ref(&mut time),
            "refs/heads/main",
            "initial",
            tree,
            Vec::<gix::ObjectId>::new(),
        )
        .unwrap();
        let mut index = repo.index_from_tree(&tree).unwrap();
        index.write(Default::default()).unwrap();
        fs::write(root.join("a.txt"), b"a-old\n").unwrap();
        fs::write(root.join("b.txt"), b"b-old\n").unwrap();
        Self {
            _temp: temp,
            root,
            private,
        }
    }
    fn status(&self) -> Value {
        native::inspect(&self.root, &self.private).unwrap()
    }
    fn stage(&self, path: &str, content: &[u8]) -> Value {
        let state = self.status();
        let plan=native::prepare(&self.root,&self.private,&json!({"operation":"stage","expected_head":state["head"],"expected_index":state["index_digest"],"files":[{"path":path,"base64":STANDARD.encode(content),"mode":"100644"}]})).unwrap();
        let result = native::execute(&plan, &self.private).unwrap();
        assert_eq!(result["state"], "completed", "{result:#}");
        plan
    }
}

#[cfg(windows)]
#[test]
fn selected_stage_and_commit_preserve_unrelated_index_and_worktree_bytes() {
    let fixture = Repo::new();
    fixture.stage("b.txt", b"b-staged\n");
    let b = fixture.status()["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["path"] == "b.txt")
        .unwrap()["object"]
        .clone();
    fixture.stage("a.txt", b"a-staged\n");
    let status = fixture.status();
    assert_eq!(
        status["entries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["path"] == "b.txt")
            .unwrap()["object"],
        b
    );
    let before = fs::read(fixture.root.join(".git/index")).unwrap();
    let plan=native::prepare(&fixture.root,&fixture.private,&json!({"operation":"commit","expected_head":status["head"],"expected_index":status["index_digest"],"paths":["a.txt"],"message":"selected a only","author":{"name":"Fixture","email":"fixture@localhost","timestamp":1}})).unwrap();
    let result = native::execute(&plan, &fixture.private).unwrap();
    assert_eq!(result["state"], "completed", "{result:#}");
    assert_eq!(fs::read(fixture.root.join(".git/index")).unwrap(), before);
    assert_eq!(fs::read(fixture.root.join("a.txt")).unwrap(), b"a-old\n");
    assert_eq!(fs::read(fixture.root.join("b.txt")).unwrap(), b"b-old\n");
    let repo = gix::open(&fixture.root).unwrap();
    let tree = repo.head_commit().unwrap().tree().unwrap();
    let a = tree.find_entry("a.txt").unwrap();
    let b = tree.find_entry("b.txt").unwrap();
    assert_eq!(
        repo.find_object(a.oid().to_owned()).unwrap().data,
        b"a-staged\n"
    );
    assert_eq!(
        repo.find_object(b.oid().to_owned()).unwrap().data,
        b"b-old\n"
    );
    assert_eq!(
        native::execute(
            &json!({"operation":"reconcile","request":plan}),
            &fixture.private
        )
        .unwrap()["state"],
        "completed"
    );
}

#[cfg(windows)]
#[test]
fn stale_index_and_ref_expectations_do_not_overwrite_external_changes() {
    let fixture = Repo::new();
    let before = fixture.status();
    let plan=native::prepare(&fixture.root,&fixture.private,&json!({"operation":"stage","expected_head":before["head"],"expected_index":before["index_digest"],"files":[{"path":"a.txt","base64":STANDARD.encode(b"a-intended"),"mode":"100644"}]})).unwrap();
    fixture.stage("b.txt", b"external staging");
    let external = fs::read(fixture.root.join(".git/index")).unwrap();
    assert_eq!(
        native::execute(&plan, &fixture.private).unwrap()["state"],
        "conflict"
    );
    assert_eq!(fs::read(fixture.root.join(".git/index")).unwrap(), external);
    assert_eq!(
        native::execute(
            &json!({"operation":"reconcile","request":plan}),
            &fixture.private
        )
        .unwrap()["state"],
        "conflict"
    );
    let create=native::prepare(&fixture.root,&fixture.private,&json!({"operation":"branch","action":"create","name":"selected","expected":null,"target":before["head"]})).unwrap();
    assert_eq!(
        native::execute(&create, &fixture.private).unwrap()["state"],
        "completed"
    );
    assert_eq!(
        native::execute(&create, &fixture.private).unwrap()["state"],
        "conflict"
    );
    let delete=native::prepare(&fixture.root,&fixture.private,&json!({"operation":"branch","action":"delete","name":"selected","expected":before["head"]})).unwrap();
    assert_eq!(
        native::execute(&delete, &fixture.private).unwrap()["state"],
        "completed"
    );
    assert!(
        native::prepare(
            &fixture.root,
            &fixture.private,
            &json!({"operation":"branch","action":"delete","name":"main","expected":before["head"]})
        )
        .is_err()
    );
}

#[test]
fn prepare_is_read_only_and_status_reports_raw_bytes_without_filters() {
    let fixture = Repo::new();
    let state = fixture.status();
    let before = fs::read(fixture.root.join(".git/index")).unwrap();
    let plan=native::prepare(&fixture.root,&fixture.private,&json!({"operation":"stage","expected_head":state["head"],"expected_index":state["index_digest"],"files":[{"path":"new.txt","base64":STANDARD.encode(b"new artifact bytes"),"mode":"100644"}]})).unwrap();
    let object = plan["objects"][0]["id"]
        .as_str()
        .unwrap()
        .parse::<gix::ObjectId>()
        .unwrap();
    assert!(
        gix::open(&fixture.root)
            .unwrap()
            .find_object(object)
            .is_err()
    );
    assert_eq!(fs::read(fixture.root.join(".git/index")).unwrap(), before);
    assert_eq!(fixture.status()["head"], state["head"]);
    fs::write(fixture.root.join("a.txt"), b"a-changed\r\n").unwrap();
    fs::write(fixture.root.join("untracked.txt"), b"new").unwrap();
    let status = fixture.status();
    assert!(
        status["status"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["path"] == "a.txt" && e["worktree"] == "modified_raw_bytes")
    );
    assert!(
        status["status"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["path"] == "untracked.txt" && e["worktree"] == "untracked")
    );
}

#[cfg(windows)]
#[test]
fn checked_consent_and_selected_commit_preserve_unrelated_staging() {
    let fixture = Repo::new();
    fixture.stage("b.txt", b"unrelated staged b\n");
    let mut app = fluxr_app_host::App::open(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../app"),
        &fixture.private,
    )
    .unwrap();
    let pid = app
        .invoke("open_project", json!({"root":fixture.root}))
        .unwrap()["id"]
        .clone();
    let artifact=app.invoke("capture",json!({"project":pid,"name":"a.txt","mime":"text/plain","base64":STANDARD.encode(b"selected staged a\n")})).unwrap()["id"].clone();
    let before = fixture.status();
    let proposal=app.invoke("git_propose",json!({"project":pid,"operation":"git.stage","expected_head":before["head"],"expected_index":before["index_digest"],"files":[{"path":"a.txt","artifact":artifact}]})).unwrap();
    assert!(
        app.invoke(
            "git_apply",
            json!({"project":pid,"mutation":proposal["mutation"],"grant":"invented"})
        )
        .is_err()
    );
    assert!(app.invoke("tool_run",json!({"project":pid,"tool":"git.authorize","arguments":{"mutation":proposal["mutation"]}})).is_err());
    let grant = app
        .invoke(
            "git_authorize",
            json!({"project":pid,"mutation":proposal["mutation"],"digest":proposal["digest"]}),
        )
        .unwrap()["grant"]
        .clone();
    let staged = app
        .invoke(
            "git_apply",
            json!({"project":pid,"mutation":proposal["mutation"],"grant":grant}),
        )
        .unwrap();
    assert_eq!(staged["state"], "completed", "{staged:#}");
    let status = fixture.status();
    let index = fs::read(fixture.root.join(".git/index")).unwrap();
    let commit=app.invoke("git_propose",json!({"project":pid,"operation":"git.commit","expected_head":status["head"],"expected_index":status["index_digest"],"paths":["a.txt"],"message":"Approved selected a","author":{"name":"Fixture","email":"fixture@localhost","timestamp":2}})).unwrap();
    let grant = app
        .invoke(
            "git_authorize",
            json!({"project":pid,"mutation":commit["mutation"],"digest":commit["digest"]}),
        )
        .unwrap()["grant"]
        .clone();
    let result = app
        .invoke(
            "git_apply",
            json!({"project":pid,"mutation":commit["mutation"],"grant":grant}),
        )
        .unwrap();
    assert_eq!(result["state"], "completed", "{result:#}");
    assert_eq!(fs::read(fixture.root.join(".git/index")).unwrap(), index);
    assert_eq!(fs::read(fixture.root.join("a.txt")).unwrap(), b"a-old\n");
    let status = fixture.status();
    assert!(
        status["status"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["path"] == "b.txt" && e["index"] == "modified")
    );
    assert_eq!(
        app.invoke(
            "git_apply",
            json!({"project":pid,"mutation":commit["mutation"],"grant":grant})
        )
        .unwrap(),
        result
    );
}

#[cfg(windows)]
#[test]
fn checked_specific_recovery_rechecks_authority_and_preserves_external_index_changes() {
    let fixture = Repo::new();
    let bundle = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../app");
    let mut app = fluxr_app_host::App::open(&bundle, &fixture.private).unwrap();
    let pid = app
        .invoke("open_project", json!({"root":fixture.root}))
        .unwrap()["id"]
        .clone();
    let artifact=app.invoke("capture",json!({"project":pid,"name":"a.txt","mime":"text/plain","base64":STANDARD.encode(b"intended a\n")})).unwrap()["id"].clone();
    let state = fixture.status();
    let proposal=app.invoke("git_propose",json!({"project":pid,"operation":"git.stage","expected_head":state["head"],"expected_index":state["index_digest"],"files":[{"path":"a.txt","artifact":artifact}]})).unwrap();
    let grant = app
        .invoke(
            "git_authorize",
            json!({"project":pid,"mutation":proposal["mutation"],"digest":proposal["digest"]}),
        )
        .unwrap()["grant"]
        .clone();
    let db = rusqlite::Connection::open(fixture.private.join("state.sqlite3")).unwrap();
    db.execute(
        "UPDATE git_mutations SET state='executing' WHERE id=?",
        [proposal["mutation"].as_str().unwrap()],
    )
    .unwrap();
    drop(app);
    let mut app = fluxr_app_host::App::open(&bundle, &fixture.private).unwrap();
    let saved = app
        .invoke(
            "git_get",
            json!({"project":pid,"mutation":proposal["mutation"]}),
        )
        .unwrap();
    assert_eq!(
        saved["state"], "completed",
        "Startup must recover the exact pending mutation"
    );
    assert_eq!(
        fixture.status()["index_digest"],
        proposal["request"]["intended_index"]
    );
    let recovered = app
        .invoke(
            "git_recover",
            json!({"project":pid,"mutation":proposal["mutation"]}),
        )
        .unwrap();
    assert_eq!(recovered["state"], "idle", "{recovered:#}");
    db.execute(
        "UPDATE git_mutations SET state='uncertain',result=NULL WHERE id=?",
        [proposal["mutation"].as_str().unwrap()],
    )
    .unwrap();
    db.execute(
        "UPDATE git_grants SET state='active' WHERE id=?",
        [grant.as_str().unwrap()],
    )
    .unwrap();
    assert_eq!(
        app.invoke(
            "git_recover",
            json!({"project":pid,"mutation":proposal["mutation"]})
        )
        .unwrap()["state"],
        "completed"
    );
    let state = fixture.status();
    let different=app.invoke("capture",json!({"project":pid,"name":"different.txt","mime":"text/plain","base64":STANDARD.encode(b"distinct intended bytes")})).unwrap()["id"].clone();
    let next=app.invoke("git_propose",json!({"project":pid,"operation":"git.stage","expected_head":state["head"],"expected_index":state["index_digest"],"files":[{"path":"a.txt","artifact":different}]})).unwrap();
    let revoked = app
        .invoke(
            "git_authorize",
            json!({"project":pid,"mutation":next["mutation"],"digest":next["digest"]}),
        )
        .unwrap()["grant"]
        .clone();
    app.invoke("git_revoke", json!({"project":pid,"grant":revoked}))
        .unwrap();
    db.execute(
        "UPDATE git_mutations SET state='executing' WHERE id=?",
        [next["mutation"].as_str().unwrap()],
    )
    .unwrap();
    let held = app
        .invoke(
            "git_recover",
            json!({"project":pid,"mutation":next["mutation"]}),
        )
        .unwrap();
    assert_eq!(held["state"], "held");
    assert_eq!(fixture.status()["index_digest"], state["index_digest"]);
    let state = fixture.status();
    let conflict=app.invoke("git_propose",json!({"project":pid,"operation":"git.stage","expected_head":state["head"],"expected_index":state["index_digest"],"files":[{"path":"a.txt","artifact":artifact}]})).unwrap();
    app.invoke(
        "git_authorize",
        json!({"project":pid,"mutation":conflict["mutation"],"digest":conflict["digest"]}),
    )
    .unwrap();
    db.execute(
        "UPDATE git_mutations SET state='executing' WHERE id=?",
        [conflict["mutation"].as_str().unwrap()],
    )
    .unwrap();
    fixture.stage("b.txt", b"external index version");
    let external = fs::read(fixture.root.join(".git/index")).unwrap();
    assert_eq!(
        app.invoke(
            "git_recover",
            json!({"project":pid,"mutation":conflict["mutation"]})
        )
        .unwrap()["state"],
        "conflict"
    );
    assert_eq!(fs::read(fixture.root.join(".git/index")).unwrap(), external);
}

#[test]
fn partial_worktree_status_keeps_index_and_does_not_infer_deletions() {
    let fixture = Repo::new();
    let baseline = fixture.status();
    fs::write(fixture.root.join(".gitignore"), "large-output\n").unwrap();
    fs::write(fixture.root.join("b.txt"), "observed edit\n").unwrap();
    let output = fs::File::create(fixture.root.join("large-output")).unwrap();
    output.set_len(8 * 1024 * 1024 + 1).unwrap();
    fs::remove_file(fixture.root.join("a.txt")).unwrap();
    let status = fixture.status();
    assert_eq!(status["worktree_coverage"], "partial");
    assert_eq!(status["index_digest"], baseline["index_digest"]);
    assert_eq!(status["branches"], baseline["branches"]);
    assert!(
        status["status"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["path"] == "b.txt" && row["worktree"] == "modified_raw_bytes")
    );
    assert!(
        status["status"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["path"] == "a.txt" && row["worktree"] == "unknown")
    );
}

#[cfg(unix)]
#[test]
fn worktree_owner_execute_bit_respects_filemode() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = Repo::new();
    let config = fixture.root.join(".git/config");
    let base = fs::read_to_string(&config).unwrap();
    fs::set_permissions(
        fixture.root.join("a.txt"),
        fs::Permissions::from_mode(0o744),
    )
    .unwrap();
    for (setting, changed) in [("true", true), ("false", false)] {
        fs::write(&config, format!("{base}\n[core]\nfileMode = {setting}\n")).unwrap();
        assert_eq!(
            fixture.status()["status"]
                .as_array()
                .unwrap()
                .iter()
                .any(|row| row["path"] == "a.txt" && row["worktree"] == "modified_mode"),
            changed
        );
    }
}

#[test]
fn unchanged_selected_tree_is_rejected() {
    let fixture = Repo::new();
    let state = fixture.status();
    let err = native::prepare(&fixture.root, &fixture.private, &json!({"operation":"commit","expected_head":state["head"],"expected_index":state["index_digest"],"paths":["a.txt"],"message":"unchanged"})).unwrap_err();
    assert!(err.to_string().starts_with("No change:"));
}

#[cfg(windows)]
#[test]
fn marker_inserted_after_prepare_blocks_commit_before_object_publication() {
    let fixture = Repo::new();
    fixture.stage("a.txt", b"selected change\n");
    let state = fixture.status();
    let plan = native::prepare(&fixture.root, &fixture.private, &json!({"operation":"commit","expected_head":state["head"],"expected_index":state["index_digest"],"paths":["a.txt"],"message":"selected change","author":{"name":"Fixture","email":"fixture@localhost","timestamp":1}})).unwrap();
    fs::write(
        fixture.root.join(".git/MERGE_HEAD"),
        state["head"].as_str().unwrap(),
    )
    .unwrap();
    assert_eq!(
        native::execute(&plan, &fixture.private).unwrap()["state"],
        "conflict"
    );
    assert_eq!(fixture.status()["head"], state["head"]);
    assert_eq!(fixture.status()["index_digest"], state["index_digest"]);
    let repo = gix::open(&fixture.root).unwrap();
    for object in plan["objects"].as_array().unwrap() {
        let id = object["id"]
            .as_str()
            .unwrap()
            .parse::<gix::ObjectId>()
            .unwrap();
        assert!(repo.find_object(id).is_err());
    }
}

#[test]
fn raw_git_paths_and_alternate_storage_are_rejected_before_writes() {
    let fixture = Repo::new();
    let state = fixture.status();
    let before = fs::read(fixture.root.join(".git/index")).unwrap();
    for path in [
        ".git/config",
        "nested/.Git/HEAD",
        "a.txt:stream",
        "../escape",
        "/absolute",
        "nested/./file",
        "nested//file",
        "CON",
    ] {
        assert!(native::prepare(&fixture.root,&fixture.private,&json!({"operation":"stage","expected_head":state["head"],"expected_index":state["index_digest"],"files":[{"path":path,"base64":STANDARD.encode(b"bytes")}]})).is_err(),"{path}");
    }
    assert_eq!(fs::read(fixture.root.join(".git/index")).unwrap(), before);
    fs::create_dir_all(fixture.root.join(".git/objects/info")).unwrap();
    fs::write(
        fixture.root.join(".git/objects/info/alternates"),
        fixture.private.to_str().unwrap(),
    )
    .unwrap();
    assert!(native::inspect(&fixture.root, &fixture.private).is_err());
    assert!(native::open(&fixture.root, &fixture.private).is_err());
}

#[cfg(windows)]
#[test]
fn hard_linked_private_metadata_and_replaced_repository_identity_are_rejected() {
    let fixture = Repo::new();
    let state = fixture.status();
    let plan=native::prepare(&fixture.root,&fixture.private,&json!({"operation":"stage","expected_head":state["head"],"expected_index":state["index_digest"],"files":[{"path":"a.txt","base64":STANDARD.encode(b"new")}]})).unwrap();
    let index = fixture.root.join(".git/index");
    let retained = fixture.private.join("index-retained");
    fs::rename(&index, &retained).unwrap();
    fs::hard_link(&retained, &index).unwrap();
    assert!(native::inspect(&fixture.root, &fixture.private).is_err());
    assert_ne!(
        native::execute(&plan, &fixture.private).unwrap()["state"],
        "completed"
    );
    fs::remove_file(&index).unwrap();
    fs::rename(&retained, &index).unwrap();
    let gitdir = fixture.root.join(".git");
    let old = fixture.root.join("saved-git");
    fs::rename(&gitdir, &old).unwrap();
    fs::create_dir(&gitdir).unwrap();
    for name in ["HEAD", "config", "description", "index"] {
        if old.join(name).is_file() {
            fs::copy(old.join(name), gitdir.join(name)).unwrap();
        }
    }
    fs::rename(old.join("objects"), gitdir.join("objects")).unwrap();
    fs::rename(old.join("refs"), gitdir.join("refs")).unwrap();
    let result = native::execute(&plan, &fixture.private).unwrap();
    assert_eq!(result["state"], "conflict", "{result:#}");
}
