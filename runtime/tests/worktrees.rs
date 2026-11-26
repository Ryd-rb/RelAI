#[allow(dead_code)]
#[path = "../src/file_effect.rs"]
mod file_effect;
#[allow(dead_code)]
#[path = "../src/git_operations.rs"]
mod git_operations;
#[path = "../src/worktrees.rs"]
mod native;
use serde_json::{Value, json};
use std::{fs, path::PathBuf};
struct Repo {
    _temp: tempfile::TempDir,
    root: PathBuf,
    private: PathBuf,
    head: String,
}
impl Repo {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("owner-checkout");
        let private = temp.path().join("private");
        fs::create_dir(&private).unwrap();
        let repo = gix::init(&root).unwrap();
        fs::write(root.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        let blob = repo.write_blob(b"owner retained bytes\n").unwrap().detach();
        let nested = repo
            .write_object(gix::objs::Tree {
                entries: vec![gix::objs::tree::Entry {
                    mode: gix::objs::tree::EntryKind::Blob.into(),
                    filename: "nested.txt".into(),
                    oid: blob,
                }],
            })
            .unwrap()
            .detach();
        let tree = repo
            .write_object(gix::objs::Tree {
                entries: vec![
                    gix::objs::tree::Entry {
                        mode: gix::objs::tree::EntryKind::Blob.into(),
                        filename: "a.txt".into(),
                        oid: blob,
                    },
                    gix::objs::tree::Entry {
                        mode: gix::objs::tree::EntryKind::Tree.into(),
                        filename: "nested".into(),
                        oid: nested,
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
        let head = repo
            .commit_as(
                actor.to_ref(&mut time.clone()),
                actor.to_ref(&mut time),
                "refs/heads/main",
                "initial",
                tree,
                Vec::<gix::ObjectId>::new(),
            )
            .unwrap()
            .to_string();
        repo.index_from_tree(&tree)
            .unwrap()
            .write(Default::default())
            .unwrap();
        fs::write(root.join("a.txt"), b"owner retained bytes\n").unwrap();
        fs::create_dir(root.join("nested")).unwrap();
        fs::write(root.join("nested/nested.txt"), b"owner retained bytes\n").unwrap();
        Self {
            _temp: temp,
            root,
            private,
            head,
        }
    }
    fn plan(&self) -> Value {
        native::prepare(&self.root, &self.private, &json!({"operation":"create"})).unwrap()
    }
    fn execute(&self, plan: &Value) -> Value {
        native::execute(plan, &self.private).unwrap()
    }
}
#[cfg(windows)]
#[test]
fn real_linked_checkout_atomic_removal_retains_owner_anchor_and_evidence() {
    let f = Repo::new();
    let plan = f.plan();
    assert!(
        !f.private.join("managed-worktrees").exists(),
        "Preparation is read-only"
    );
    let owner_index = fs::read(f.root.join(".git/index")).unwrap();
    let owner_head = fs::read(f.root.join(".git/refs/heads/main")).unwrap();
    let result = f.execute(&plan);
    assert_eq!(result["state"], "completed", "{result:#}");
    let path = PathBuf::from(result["path"].as_str().unwrap());
    let meta = PathBuf::from(result["metadata"].as_str().unwrap());
    let anchor = PathBuf::from(result["retained_anchor"].as_str().unwrap());
    assert!(path.starts_with(f.private.canonicalize().unwrap()));
    assert_eq!(
        fs::read(path.join("a.txt")).unwrap(),
        b"owner retained bytes\n"
    );
    assert_eq!(
        fs::read(path.join("nested/nested.txt")).unwrap(),
        b"owner retained bytes\n"
    );
    let linked = gix::open(&path).unwrap();
    assert_eq!(linked.head_id().unwrap().to_string(), f.head);
    assert!(linked.head().unwrap().referent_name().is_none());
    assert_eq!(
        linked.common_dir().canonicalize().unwrap(),
        f.root.join(".git").canonicalize().unwrap()
    );
    let main = gix::open(&f.root).unwrap();
    assert_eq!(main.worktrees().unwrap().len(), 1);
    assert_eq!(
        native::inspect(&f.root, &f.private).unwrap()["worktrees"][0]["state"],
        "clean"
    );
    assert_eq!(
        f.execute(&json!({"operation":"reconcile","request":plan}))["state"],
        "completed"
    );
    drop(linked);
    drop(main);
    let remove = native::prepare(
        &f.root,
        &f.private,
        &json!({"operation":"remove","worktree":plan["id"]}),
    )
    .unwrap();
    let result = f.execute(&remove);
    assert_eq!(result["state"], "completed", "{result:#}");
    assert!(!path.exists() && !meta.exists());
    assert!(anchor.is_file());
    assert_eq!(fs::read_to_string(anchor).unwrap().trim(), f.head);
    assert!(
        f.private
            .join("worktree-evidence")
            .join(format!("{}.json", plan["id"].as_str().unwrap()))
            .is_file()
    );
    assert_eq!(
        fs::read(f.root.join("a.txt")).unwrap(),
        b"owner retained bytes\n"
    );
    assert_eq!(fs::read(f.root.join(".git/index")).unwrap(), owner_index);
    assert_eq!(
        fs::read(f.root.join(".git/refs/heads/main")).unwrap(),
        owner_head
    );
    assert_eq!(
        f.execute(&json!({"operation":"reconcile","request":remove}))["state"],
        "completed"
    );
}
#[cfg(windows)]
#[test]
fn changed_untracked_and_index_bytes_block_removal_without_deleting_anything() {
    let f = Repo::new();
    let plan = f.plan();
    let result = f.execute(&plan);
    assert_eq!(result["state"], "completed", "{result:#}");
    let path = PathBuf::from(result["path"].as_str().unwrap());
    let remove = native::prepare(
        &f.root,
        &f.private,
        &json!({"operation":"remove","worktree":plan["id"]}),
    )
    .unwrap();
    fs::create_dir(path.join("unowned-empty-directory")).unwrap();
    assert_eq!(f.execute(&remove)["state"], "conflict");
    assert!(path.join("unowned-empty-directory").is_dir());
    fs::remove_dir(path.join("unowned-empty-directory")).unwrap();
    fs::write(path.join("untracked.txt"), b"must preserve").unwrap();
    assert!(
        native::prepare(
            &f.root,
            &f.private,
            &json!({"operation":"remove","worktree":plan["id"]})
        )
        .is_err()
    );
    assert_eq!(f.execute(&remove)["state"], "conflict");
    assert_eq!(
        fs::read(path.join("untracked.txt")).unwrap(),
        b"must preserve"
    );
    fs::remove_file(path.join("untracked.txt")).unwrap();
    fs::write(path.join("a.txt"), b"edited bytes").unwrap();
    assert_eq!(f.execute(&remove)["state"], "conflict");
    assert_eq!(fs::read(path.join("a.txt")).unwrap(), b"edited bytes");
    fs::write(path.join("a.txt"), b"owner retained bytes\n").unwrap();
    let meta = PathBuf::from(result["metadata"].as_str().unwrap());
    fs::write(meta.join("index"), b"changed index").unwrap();
    assert_eq!(f.execute(&remove)["state"], "conflict");
    assert_eq!(fs::read(meta.join("index")).unwrap(), b"changed index");
    assert!(f.root.exists());
}
#[cfg(windows)]
#[test]
fn unmerged_target_and_arbitrary_removal_ids_are_refused() {
    let f = Repo::new();
    let repo = gix::open(&f.root).unwrap();
    let initial = repo.head_commit().unwrap();
    let actor = gix::actor::Signature {
        name: "Fixture".into(),
        email: "fixture@localhost".into(),
        time: gix::date::Time::new(1, 0),
    };
    let commit = gix::objs::Commit {
        tree: initial.tree_id().unwrap().detach(),
        parents: vec![initial.id().detach()].into(),
        author: actor.clone(),
        committer: actor,
        encoding: None,
        message: "unmerged managed work".into(),
        extra_headers: Default::default(),
    };
    let target = repo.write_object(commit).unwrap().detach();
    let plan = native::prepare(
        &f.root,
        &f.private,
        &json!({"operation":"create","target":target.to_string()}),
    )
    .unwrap();
    assert_eq!(f.execute(&plan)["state"], "completed");
    assert!(
        native::prepare(
            &f.root,
            &f.private,
            &json!({"operation":"remove","worktree":plan["id"]})
        )
        .is_err()
    );
    for wid in ["../owner-checkout", f.root.to_str().unwrap(), "not-an-id"] {
        assert!(
            native::prepare(
                &f.root,
                &f.private,
                &json!({"operation":"remove","worktree":wid})
            )
            .is_err()
        );
    }
    assert!(f.root.join("a.txt").is_file());
    assert_eq!(
        fs::read_to_string(f.root.join(".git/refs/heads/main"))
            .unwrap()
            .trim(),
        f.head
    );
}

#[cfg(windows)]
#[test]
fn checked_grants_specific_startup_recovery_and_destructive_removal() {
    let f = Repo::new();
    let bundle = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../app");
    let mut app = fluxr_app_host::App::open(&bundle, &f.private).unwrap();
    let pid = app.invoke("open_project", json!({"root":f.root})).unwrap()["id"].clone();
    let proposal = app
        .invoke(
            "tool_run",
            json!({"project":pid,"tool":"git.worktree_create","arguments":{}}),
        )
        .unwrap();
    assert!(
        app.invoke(
            "worktree_apply",
            json!({"project":pid,"mutation":proposal["mutation"],"grant":"model authority"})
        )
        .is_err()
    );
    assert!(
        app.invoke(
            "worktree_authorize",
            json!({"project":pid,"mutation":proposal["mutation"],"digest":"different exact bytes"})
        )
        .is_err()
    );
    assert!(
        app.invoke(
            "tool_run",
            json!({"project":pid,"tool":"worktree.authorize","arguments":{}})
        )
        .is_err()
    );
    let grant = app
        .invoke(
            "worktree_authorize",
            json!({"project":pid,"mutation":proposal["mutation"],"digest":proposal["digest"]}),
        )
        .unwrap()["grant"]
        .clone();
    let db = rusqlite::Connection::open(f.private.join("state.sqlite3")).unwrap();
    db.execute(
        "UPDATE managed_worktrees SET state='executing' WHERE id=?",
        [proposal["mutation"].as_str().unwrap()],
    )
    .unwrap();
    drop(app);
    let mut app = fluxr_app_host::App::open(&bundle, &f.private).unwrap();
    let saved = app
        .invoke(
            "worktree_get",
            json!({"project":pid,"mutation":proposal["mutation"]}),
        )
        .unwrap();
    assert_eq!(saved["state"], "completed", "{saved:#}");
    assert_eq!(
        app.invoke(
            "worktree_apply",
            json!({"project":pid,"mutation":proposal["mutation"],"grant":grant})
        )
        .unwrap(),
        saved["result"]
    );
    let list = native::inspect(&f.root, &f.private).unwrap();
    let wid = list["worktrees"][0]["id"].clone();
    let path = PathBuf::from(list["worktrees"][0]["path"].as_str().unwrap());
    let remove = app
        .invoke(
            "worktree_propose",
            json!({"project":pid,"operation":"git.worktree_remove","worktree":wid}),
        )
        .unwrap();
    assert!(
        app.invoke(
            "worktree_authorize",
            json!({"project":pid,"mutation":remove["mutation"],"digest":remove["digest"]})
        )
        .is_err()
    );
    let grant=app.invoke("worktree_authorize",json!({"project":pid,"mutation":remove["mutation"],"digest":remove["digest"],"destructive":true})).unwrap()["grant"].clone();
    app.invoke("worktree_revoke", json!({"project":pid,"grant":grant}))
        .unwrap();
    db.execute(
        "UPDATE managed_worktrees SET state='executing' WHERE id=?",
        [remove["mutation"].as_str().unwrap()],
    )
    .unwrap();
    assert_eq!(
        app.invoke(
            "worktree_recover",
            json!({"project":pid,"mutation":remove["mutation"]})
        )
        .unwrap()["state"],
        "held"
    );
    assert!(path.is_dir());
    let remove = app
        .invoke(
            "worktree_propose",
            json!({"project":pid,"operation":"git.worktree_remove","worktree":wid}),
        )
        .unwrap();
    let grant=app.invoke("worktree_authorize",json!({"project":pid,"mutation":remove["mutation"],"digest":remove["digest"],"destructive":true})).unwrap()["grant"].clone();
    let completed = app
        .invoke(
            "worktree_apply",
            json!({"project":pid,"mutation":remove["mutation"],"grant":grant}),
        )
        .unwrap();
    assert_eq!(completed["state"], "completed", "{completed:#}");
    assert!(!path.exists());
    assert!(f.root.join("a.txt").is_file());
    db.execute(
        "UPDATE managed_worktrees SET state='uncertain',result=NULL WHERE id=?",
        [remove["mutation"].as_str().unwrap()],
    )
    .unwrap();
    assert_eq!(
        app.invoke(
            "worktree_recover",
            json!({"project":pid,"mutation":remove["mutation"]})
        )
        .unwrap()["state"],
        "completed"
    );
}
