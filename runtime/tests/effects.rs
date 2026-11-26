#[path = "../src/file_effect.rs"]
mod native;

use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::fs;
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[test]
fn pure_patch_transform_checks_exact_base_and_returns_actual_derived_bytes() {
    let patch = "--- old\n+++ new\n@@ -1 +1 @@\n-before\n+after\n";
    let result =
        native::patch(&json!({"base64":STANDARD.encode(b"before\n"),"patch":patch})).unwrap();
    assert_eq!(
        STANDARD.decode(result["base64"].as_str().unwrap()).unwrap(),
        b"after\n"
    );
    assert_eq!(result["base_digest"], hash(b"before\n"));
    assert_eq!(result["digest"], hash(b"after\n"));
    assert!(
        native::patch(&json!({"base64":STANDARD.encode(b"external\n"),"patch":patch})).is_err()
    );
}

#[cfg(windows)]
#[test]
fn transacted_write_is_real_atomic_and_rejects_stale_expected_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let private = temp.path().join("private");
    fs::create_dir(&private).unwrap();
    let path = temp.path().join("file.txt");
    fs::write(&path, b"old").unwrap();
    let request = json!({"operation":"write","path":path,"expected":{"kind":"file","digest":hash(b"old")},"intended":{"kind":"file","digest":hash(b"new")},"base64":STANDARD.encode(b"new")});
    let observed = native::inspect(&request, &private).unwrap();
    assert_eq!(observed["state"]["digest"], hash(b"old"));
    let receipt = native::execute(&request, &private).unwrap();
    assert_eq!(receipt["state"], "completed", "{receipt:#}");
    assert_eq!(fs::read(&path).unwrap(), b"new");
    let stale = native::execute(&request, &private).unwrap();
    assert_eq!(stale["state"], "conflict", "{stale:#}");
    assert_eq!(fs::read(path).unwrap(), b"new");
}

#[cfg(windows)]
#[test]
fn native_recovery_distinguishes_old_intended_and_external_conflict() {
    let temp = tempfile::tempdir().unwrap();
    let private = temp.path().join("private");
    fs::create_dir(&private).unwrap();
    let path = temp.path().join("file.txt");
    let request = json!({"operation":"write","path":path,"expected":{"kind":"absent"},"intended":{"kind":"file","digest":hash(b"intended")},"base64":STANDARD.encode(b"intended")});
    assert_eq!(
        native::execute(
            &json!({"operation":"reconcile","request":request}),
            &private
        )
        .unwrap()["state"],
        "old"
    );
    assert_eq!(
        native::execute(&request, &private).unwrap()["state"],
        "completed"
    );
    assert_eq!(
        native::execute(
            &json!({"operation":"reconcile","request":request}),
            &private
        )
        .unwrap()["state"],
        "completed"
    );
    fs::write(&path, b"external").unwrap();
    assert_eq!(
        native::execute(
            &json!({"operation":"reconcile","request":request}),
            &private
        )
        .unwrap()["state"],
        "conflict"
    );
    assert_eq!(fs::read(path).unwrap(), b"external");
}

#[cfg(windows)]
#[test]
fn native_mkdir_copy_move_remove_and_busy_sources_are_guarded() {
    use std::os::windows::fs::OpenOptionsExt;
    let temp = tempfile::tempdir().unwrap();
    let private = temp.path().join("private");
    fs::create_dir(&private).unwrap();
    let directory = temp.path().join("new");
    let mkdir = json!({"operation":"mkdir","path":directory,"expected":{"kind":"absent"},"intended":{"kind":"directory"}});
    let result = native::execute(&mkdir, &private).unwrap();
    assert_eq!(result["state"], "completed", "{result:#}");
    assert!(directory.is_dir());
    let source = directory.join("source.txt");
    let destination = directory.join("copy.txt");
    fs::write(&source, b"source").unwrap();
    let copy = json!({"operation":"copy","source":source,"path":destination,"source_expected":{"kind":"file","digest":hash(b"source")},"expected":{"kind":"absent"},"intended":{"kind":"file","digest":hash(b"source")}});
    let mutator = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .share_mode(1)
        .open(&source)
        .unwrap();
    assert_ne!(
        native::execute(&copy, &private).unwrap()["state"],
        "completed"
    );
    assert!(!destination.exists());
    drop(mutator);
    let result = native::execute(&copy, &private).unwrap();
    assert_eq!(result["state"], "completed", "{result:#}");
    assert_eq!(fs::read(&destination).unwrap(), b"source");
    let moved = directory.join("moved.txt");
    let result=native::execute(&json!({"operation":"move","source":destination,"path":moved,"source_expected":{"kind":"file","digest":hash(b"source")},"expected":{"kind":"absent"},"intended":{"kind":"file","digest":hash(b"source")}}),&private).unwrap();
    assert_eq!(result["state"], "completed", "{result:#}");
    assert!(!destination.exists());
    assert_eq!(fs::read(&moved).unwrap(), b"source");
    let nonempty=native::execute(&json!({"operation":"remove","path":directory,"expected":{"kind":"directory"},"intended":{"kind":"absent"}}),&private).unwrap();
    assert_ne!(nonempty["state"], "completed");
    assert!(directory.exists());
    for path in [&source, &moved] {
        let result=native::execute(&json!({"operation":"remove","path":path,"expected":{"kind":"file","digest":hash(b"source")},"intended":{"kind":"absent"}}),&private).unwrap();
        assert_eq!(result["state"], "completed", "{result:#}");
        assert!(!path.exists());
    }
    let result=native::execute(&json!({"operation":"remove","path":directory,"expected":{"kind":"directory","empty":true},"intended":{"kind":"absent"}}),&private).unwrap();
    assert_eq!(result["state"], "completed", "{result:#}");
    assert!(!directory.exists());
}

#[cfg(windows)]
#[test]
fn protected_patch_batch_checks_head_and_rolls_back_all_files_on_conflict() {
    let temp = tempfile::tempdir().unwrap();
    let private = temp.path().join("private");
    let root = temp.path().join("repo");
    fs::create_dir(&private).unwrap();
    fs::create_dir_all(root.join(".git/refs/heads")).unwrap();
    let revision = "1111111111111111111111111111111111111111";
    fs::write(root.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
    fs::write(root.join(".git/refs/heads/main"), format!("{revision}\n")).unwrap();
    let a = root.join("a.txt");
    let b = root.join("b.txt");
    fs::write(&a, b"old-a").unwrap();
    fs::write(&b, b"old-b").unwrap();
    let write = |path: &std::path::Path, old: &[u8], new: &[u8]| json!({"operation":"write","path":path,"expected":{"kind":"file","digest":hash(old)},"intended":{"kind":"file","digest":hash(new)},"base64":STANDARD.encode(new)});
    let standalone = native::execute(&write(&a, b"old-a", b"new-a"), &private).unwrap();
    assert_ne!(standalone["state"], "completed");
    let bad = json!({"operation":"batch","checkout":{"root":root,"revision":revision},"operations":[write(&a,b"old-a",b"new-a"),write(&b,b"stale",b"new-b")]});
    let result = native::execute(&bad, &private).unwrap();
    assert_eq!(result["state"], "conflict", "{result:#}");
    assert_eq!(fs::read(&a).unwrap(), b"old-a");
    assert_eq!(fs::read(&b).unwrap(), b"old-b");
    let good = json!({"operation":"batch","checkout":{"root":root,"revision":revision},"operations":[write(&a,b"old-a",b"new-a"),write(&b,b"old-b",b"new-b")]});
    let result = native::execute(&good, &private).unwrap();
    assert_eq!(result["state"], "completed", "{result:#}");
    assert_eq!(fs::read(&a).unwrap(), b"new-a");
    assert_eq!(fs::read(&b).unwrap(), b"new-b");
    fs::write(
        root.join(".git/refs/heads/main"),
        "2222222222222222222222222222222222222222\n",
    )
    .unwrap();
    let result = native::execute(&good, &private).unwrap();
    assert_eq!(result["state"], "conflict");
    assert_eq!(fs::read(a).unwrap(), b"new-a");
}

#[cfg(windows)]
#[test]
fn private_targets_and_hard_link_aliases_fail_closed() {
    let temp = tempfile::tempdir().unwrap();
    let private = temp.path().join("private");
    fs::create_dir(&private).unwrap();
    let secret = private.join("secret");
    fs::write(&secret, b"secret").unwrap();
    let alias = temp.path().join("alias");
    fs::hard_link(&secret, &alias).unwrap();
    for path in [&secret, &alias] {
        let request = json!({"operation":"write","path":path,"expected":{"kind":"file","digest":hash(b"secret")},"intended":{"kind":"file","digest":hash(b"changed")},"base64":STANDARD.encode(b"changed")});
        assert!(native::inspect(&request, &private).is_err());
        assert_ne!(
            native::execute(&request, &private).unwrap()["state"],
            "completed"
        );
    }
    assert_eq!(fs::read(secret).unwrap(), b"secret");
}

struct Fixture {
    _temp: tempfile::TempDir,
    app: fluxr_app_host::App,
    pid: serde_json::Value,
    root: std::path::PathBuf,
    data: std::path::PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("project");
        let data = temp.path().join("private");
        fs::create_dir(&root).unwrap();
        let bundle = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../app");
        let mut app = fluxr_app_host::App::open(&bundle, &data).unwrap();
        let pid = app.invoke("open_project", json!({"root":root})).unwrap()["id"].clone();
        Self {
            _temp: temp,
            app,
            pid,
            root,
            data,
        }
    }
    fn call(
        &mut self,
        name: &str,
        mut arg: serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        arg["project"] = self.pid.clone();
        self.app.invoke(name, arg)
    }
    fn artifact(&mut self, bytes: &[u8]) -> serde_json::Value {
        self.call(
            "capture",
            json!({"name":"output.txt","mime":"text/plain","base64":STANDARD.encode(bytes)}),
        )
        .unwrap()["id"]
            .clone()
    }
    fn authorize(&mut self, proposal: &serde_json::Value, destructive: bool) -> serde_json::Value {
        self.call("effect_authorize",json!({"effect":proposal["effect"],"digest":proposal["digest"],"destructive":destructive})).unwrap()["grant"].clone()
    }
}

#[cfg(windows)]
#[test]
fn checked_effect_requires_exact_user_grant_and_persists_idempotent_receipt() {
    let mut f = Fixture::new();
    let path = f.root.join("target.txt");
    fs::write(&path, b"old").unwrap();
    let artifact = f.artifact(b"new");
    let proposal=f.call("effect_propose",json!({"operation":"fs.write","path":path,"artifact":artifact,"expected":{"kind":"file","digest":hash(b"old")}})).unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"old");
    assert!(
        f.call(
            "effect_apply",
            json!({"effect":proposal["effect"],"grant":"invented"})
        )
        .is_err()
    );
    assert!(
        f.call(
            "effect_authorize",
            json!({"effect":proposal["effect"],"digest":"stale","destructive":true})
        )
        .is_err()
    );
    assert!(
        f.call(
            "tool_run",
            json!({"tool":"effect.authorize","arguments":{"effect":proposal["effect"]}})
        )
        .is_err()
    );
    let grant = f.authorize(&proposal, true);
    let receipt = f
        .call(
            "effect_apply",
            json!({"effect":proposal["effect"],"grant":grant}),
        )
        .unwrap();
    assert_eq!(receipt["state"], "completed", "{receipt:#}");
    assert_eq!(fs::read(&path).unwrap(), b"new");
    assert_eq!(
        f.call(
            "effect_apply",
            json!({"effect":proposal["effect"],"grant":grant})
        )
        .unwrap(),
        receipt
    );
    let saved = f
        .call("effect_get", json!({"effect":proposal["effect"]}))
        .unwrap();
    assert_eq!(saved["state"], "completed");
    assert_eq!(saved["result"], receipt);
}

#[cfg(windows)]
#[test]
fn checked_recovery_completes_known_intended_state_and_preserves_external_conflicts() {
    let mut f = Fixture::new();
    let path = f.root.join("target.txt");
    let artifact = f.artifact(b"intended");
    let proposal=f.call("effect_propose",json!({"operation":"fs.write","path":path,"artifact":artifact,"expected":{"kind":"absent"}})).unwrap();
    let grant = f.authorize(&proposal, false);
    let db = rusqlite::Connection::open(f.data.join("state.sqlite3")).unwrap();
    db.execute(
        "UPDATE file_effects SET state='executing' WHERE id=?",
        [proposal["effect"].as_str().unwrap()],
    )
    .unwrap();
    let recovered = f.call("effects_recover", json!({})).unwrap();
    assert_eq!(recovered["state"], "completed", "{recovered:#}");
    assert_eq!(fs::read(&path).unwrap(), b"intended");
    db.execute(
        "UPDATE file_effects SET state='executing',result=NULL WHERE id=?",
        [proposal["effect"].as_str().unwrap()],
    )
    .unwrap();
    db.execute(
        "UPDATE file_grants SET state='active' WHERE id=?",
        [grant.as_str().unwrap()],
    )
    .unwrap();
    let recovered = f.call("effects_recover", json!({})).unwrap();
    assert_eq!(recovered["state"], "completed");
    db.execute(
        "UPDATE file_effects SET state='uncertain',result=NULL WHERE id=?",
        [proposal["effect"].as_str().unwrap()],
    )
    .unwrap();
    fs::write(&path, b"external").unwrap();
    let preserved = f.call("effects_recover", json!({})).unwrap();
    assert_eq!(preserved["state"], "conflict");
    assert_eq!(fs::read(path).unwrap(), b"external");
}

#[cfg(windows)]
#[test]
fn checked_staged_patch_protects_checkout_revision_and_exact_file_versions() {
    let mut f = Fixture::new();
    fs::create_dir_all(f.root.join(".git/refs/heads")).unwrap();
    let revision = "1111111111111111111111111111111111111111";
    fs::write(f.root.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
    fs::write(f.root.join(".git/refs/heads/main"), format!("{revision}\n")).unwrap();
    let path = f.root.join("readme.txt");
    fs::write(&path, b"before\n").unwrap();
    let artifact = f.artifact(b"before\n");
    let patch = "--- old\n+++ new\n@@ -1 +1 @@\n-before\n+after\n";
    let preview = f
        .call(
            "patch_preview",
            json!({"base_artifact":artifact,"patch":patch}),
        )
        .unwrap();
    assert_eq!(preview["intended_digest"], hash(b"after\n"));
    assert_eq!(fs::read(&path).unwrap(), b"before\n");
    let staged=f.call("patch_stage",json!({"checkout":f.pid,"revision":revision,"files":[{"path":"readme.txt","base_artifact":artifact,"patch":patch}]})).unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"before\n");
    let proposal=f.call("patch_apply",json!({"checkout":f.pid,"revision":revision,"stage":staged["stage"],"digest":staged["digest"]})).unwrap();
    let grant = f.authorize(&proposal, true);
    let result = f
        .call(
            "effect_apply",
            json!({"effect":proposal["effect"],"grant":grant}),
        )
        .unwrap();
    assert_eq!(result["state"], "completed", "{result:#}");
    assert_eq!(fs::read(path).unwrap(), b"after\n");
}
