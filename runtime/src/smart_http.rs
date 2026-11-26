//! Single-branch Smart HTTP v0. All process execution, URL rewriting, hooks,
//! checkout filters and credential helpers are outside this adapter.
use crate::{git_operations, git_pack, text};
use anyhow::{Context, Result, ensure};
use gix::{ObjectId, refs::transaction::PreviousValue};
use reqwest::{Client, Url};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    time::Duration,
};
const MAX_PACK: usize = 32 * 1024 * 1024;
const ZERO: &str = "0000000000000000000000000000000000000000";
struct Advertisement {
    refs: BTreeMap<String, ObjectId>,
    caps: BTreeSet<String>,
}
fn pkt(out: &mut Vec<u8>, data: &[u8]) -> Result<()> {
    ensure!(data.len() <= 65516, "Git packet too long");
    write_header(out, data.len() + 4);
    out.extend_from_slice(data);
    Ok(())
}
fn write_header(out: &mut Vec<u8>, n: usize) {
    out.extend_from_slice(format!("{n:04x}").as_bytes())
}
fn next<'a>(bytes: &mut &'a [u8]) -> Result<Option<&'a [u8]>> {
    let n = usize::from_str_radix(
        std::str::from_utf8(bytes.get(..4).context("Truncated Git packet")?)?,
        16,
    )?;
    *bytes = &bytes[4..];
    if n == 0 {
        return Ok(None);
    }
    ensure!((4..=65520).contains(&n), "Invalid Git packet size");
    let payload = bytes.get(..n - 4).context("Truncated Git payload")?;
    *bytes = &bytes[n - 4..];
    Ok(Some(payload))
}
fn advertisement(bytes: &[u8], service: &str) -> Result<Advertisement> {
    let mut data = bytes;
    let header = format!("# service={service}\n");
    ensure!(
        next(&mut data)? == Some(header.as_bytes()) && next(&mut data)?.is_none(),
        "Invalid Smart HTTP service advertisement"
    );
    let mut refs = BTreeMap::new();
    let mut caps = BTreeSet::new();
    let mut first = true;
    while let Some(row) = next(&mut data)? {
        let s = std::str::from_utf8(row)?.trim_end_matches('\n');
        let (line, cap) = s.split_once('\0').unwrap_or((s, ""));
        if first {
            caps.extend(cap.split_ascii_whitespace().map(str::to_owned));
            first = false
        } else {
            ensure!(cap.is_empty(), "Unexpected capability list")
        }
        let (hex, name) = line
            .split_once(' ')
            .context("Invalid advertised reference")?;
        if name != "capabilities^{}" {
            ensure!(refs.len() < 4096, "Remote ref limit");
            refs.insert(name.into(), hex.parse()?);
        }
    }
    ensure!(data.is_empty(), "Trailing advertised data");
    ensure!(
        !caps
            .iter()
            .any(|c| c.starts_with("object-format=") && c != "object-format=sha1"),
        "SHA-1 remotes only"
    );
    Ok(Advertisement { refs, caps })
}
struct Endpoint {
    base: Url,
    client: Client,
    token: Option<String>,
}
impl Endpoint {
    fn request(&self, service: &str, listing: bool) -> Result<reqwest::RequestBuilder> {
        let path = if listing { "info/refs" } else { service };
        let dest = self.base.join(path)?;
        let mut r = if listing {
            self.client.get(dest).query(&[("service", service)])
        } else {
            self.client
                .post(dest)
                .header("Content-Type", format!("application/x-{service}-request"))
        };
        if let Some(token) = &self.token {
            r = r.basic_auth("x-access-token", Some(token))
        }
        Ok(r.header(
            "Accept",
            format!(
                "application/x-{service}-{}",
                if listing { "advertisement" } else { "result" }
            ),
        ))
    }
}
async fn response(req: reqwest::RequestBuilder, typ: &str, limit: usize) -> Result<Vec<u8>> {
    let mut res = req.send().await?;
    ensure!(
        res.status().is_success(),
        "Git server returned {}",
        res.status()
    );
    ensure!(
        res.headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.split(';').next())
            == Some(typ),
        "Unexpected Git response type"
    );
    let mut out = Vec::new();
    while let Some(piece) = res.chunk().await? {
        ensure!(
            piece.len() <= limit - out.len(),
            "Git response exceeds limit"
        );
        out.extend_from_slice(&piece);
    }
    Ok(out)
}
pub async fn exchange(plan: &Value, private: &Path) -> Result<Value> {
    let mut base = Url::parse(text(&plan["url"])?)?;
    ensure!(
        base.scheme() == "https"
            && base.username().is_empty()
            && base.password().is_none()
            && base.fragment().is_none()
            && base.query().is_none(),
        "Use an approved HTTPS repository URL"
    );
    if !base.path().ends_with('/') {
        let suffix = format!("{}/", base.path());
        base.set_path(&suffix);
    }
    let token = if let Some(var) = plan["credential"].as_str() {
        ensure!(
            var.starts_with("RELAI_GIT_")
                && var.ends_with("_TOKEN")
                && var
                    .bytes()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == b'_'),
            "Invalid Git credential reference"
        );
        Some(std::env::var(var).context("Missing Git credential")?)
    } else {
        None
    };
    let mut client = Client::builder()
        .timeout(Duration::from_secs(45))
        .redirect(reqwest::redirect::Policy::none());
    if let Some(path) = std::env::var_os("RELAI_GIT_CA_FILE") {
        let pem = std::fs::read(path)?;
        ensure!(pem.len() <= 65536, "CA bundle exceeds bound");
        client = client.add_root_certificate(reqwest::Certificate::from_pem(&pem)?);
    }
    let endpoint = Endpoint {
        base,
        token,
        client: client.build()?,
    };
    transfer(plan, private, &endpoint).await
}
async fn transfer(plan: &Value, private: &Path, remote: &Endpoint) -> Result<Value> {
    let op = text(&plan["op"])?;
    let service = if op == "push" {
        "git-receive-pack"
    } else {
        "git-upload-pack"
    };
    let listing = response(
        remote.request(service, true)?,
        &format!("application/x-{service}-advertisement"),
        1024 * 1024,
    )
    .await?;
    let ad = advertisement(&listing, service)?;
    let branch = text(&plan["branch"])?;
    ensure!(
        branch.starts_with("refs/heads/") && branch.len() <= 256,
        "Expected full branch ref"
    );
    let _ = gix::refs::FullName::try_from(branch)?;
    let advertised = ad.refs.get(branch).copied();
    if op == "reconcile" {
        let observed = advertised.map(|id| id.to_string());
        let proven = observed.as_deref().is_some_and(|id| plan["target"] == id);
        return Ok(
            json!({"state":if proven{"completed"}else{"uncertain"},"observed":observed,"target":plan["target"]}),
        );
    }
    if op == "refs" {
        return Ok(json!({"branch":branch,"target":advertised.map(|id|id.to_string())}));
    }
    let target = text(&plan["target"])?.parse::<ObjectId>()?;
    let root = Path::new(text(&plan["root"])?);
    if op == "push" {
        let expected = plan["expected"]
            .as_str()
            .map(str::parse::<ObjectId>)
            .transpose()?;
        if expected != advertised {
            return Ok(
                json!({"state":"conflict","reason":"Remote branch changed before push","observed":advertised.map(|id|id.to_string())}),
            );
        }
        if expected == Some(target) {
            return Ok(
                json!({"branch":branch,"target":target.to_string(),"state":"already_current"}),
            );
        }
        ensure!(
            ad.caps.contains("report-status"),
            "Remote lacks push status receipts"
        );
        let pack = {
            let repo = git_operations::open(root, private)?;
            match git_pack::reachable(&repo, target, expected) {
                Ok(bytes) => bytes,
                Err(error) => return Ok(json!({"state":"rejected","reason":error.to_string()})),
            }
        };
        let mut body = Vec::new();
        pkt(
            &mut body,
            format!(
                "{} {} {}\0report-status\n",
                expected.map_or_else(|| ZERO.to_owned(), |id| id.to_string()),
                target,
                branch
            )
            .as_bytes(),
        )?;
        body.extend_from_slice(b"0000");
        body.extend_from_slice(&pack);
        let bytes = response(
            remote.request(service, false)?.body(body),
            "application/x-git-receive-pack-result",
            65536,
        )
        .await?;
        let mut data = bytes.as_slice();
        ensure!(
            next(&mut data)? == Some(b"unpack ok\n"),
            "Remote did not confirm unpack"
        );
        ensure!(
            next(&mut data)? == Some(format!("ok {branch}\n").as_bytes())
                && next(&mut data)?.is_none()
                && data.is_empty(),
            "Remote did not confirm branch update"
        );
        return Ok(
            json!({"branch":branch,"previous":plan["expected"],"target":target.to_string(),"state":"pushed"}),
        );
    }
    ensure!(op == "fetch", "Invalid remote action");
    ensure!(
        advertised == Some(target),
        "Remote branch differs from approved fetch target"
    );
    let mut body = Vec::new();
    pkt(&mut body, format!("want {target}\n").as_bytes())?;
    body.extend_from_slice(b"0000");
    pkt(&mut body, b"done\n")?;
    let bytes = response(
        remote.request(service, false)?.body(body),
        "application/x-git-upload-pack-result",
        MAX_PACK,
    )
    .await?;
    let mut pack = bytes.as_slice();
    ensure!(
        next(&mut pack)? == Some(b"NAK\n"),
        "Unexpected fetch acknowledgement"
    );
    let id = text(&plan["remote"])?;
    ensure!(
        id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit()),
        "Invalid remote registration"
    );
    let tracking = format!("refs/remotes/relai/{id}/{}", &branch["refs/heads/".len()..]);
    let repo = git_operations::open(root, private)?;
    let current = repo
        .try_find_reference(tracking.as_str())?
        .and_then(|r| r.target().try_id().map(|i| i.to_owned()));
    let expect = plan["expected"]
        .as_str()
        .map(str::parse::<ObjectId>)
        .transpose()?;
    ensure!(current == expect, "Local tracking ref changed");
    let packdir = crate::join(repo.path(), "objects/pack")?;
    std::fs::create_dir_all(&packdir)?;
    git_pack::install(pack, &packdir)?;
    let repo = git_operations::open(root, private)?;
    let _ = repo.find_commit(target)?;
    let previous = expect.map_or(PreviousValue::MustNotExist, |id| {
        PreviousValue::MustExistAndMatch(id.into())
    });
    use gix::refs::transaction::{Change, LogChange, RefEdit, RefLog};
    let change = Change::Update {
        expected: previous,
        new: target.into(),
        log: LogChange {
            mode: RefLog::AndReference,
            force_create_reflog: true,
            message: "Approved HTTPS fetch".into(),
        },
    };
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs() as i64;
    let actor = gix::actor::Signature {
        name: "RelAI".into(),
        email: "relai@localhost".into(),
        time: gix::date::Time::new(seconds, 0),
    };
    let mut buffer = gix::date::parse::TimeBuf::default();
    repo.edit_references_as(
        [RefEdit {
            name: tracking.as_str().try_into()?,
            deref: false,
            change,
        }],
        Some(actor.to_ref(&mut buffer)),
    )?;
    Ok(json!({"branch":branch,"tracking":tracking,"target":target.to_string(),"state":"fetched"}))
}
