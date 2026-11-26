//! Bounded SHA-1 packs for Smart HTTP. Fetched packs are indexed by gitoxide.
use anyhow::{Context, Result, bail, ensure};
use flate2::{Compression, bufread::ZlibDecoder, write::ZlibEncoder};
use gix::{ObjectId, objs::Kind};
use sha1::{Digest, Sha1};
use std::{
    collections::BTreeSet,
    io::{Read, Write},
    path::Path,
    sync::atomic::AtomicBool,
};
const MAX_OBJECT: usize = 4 * 1024 * 1024;
const MAX_TOTAL: usize = 32 * 1024 * 1024;
const MAX_ENTRIES: usize = 4096;

pub fn reachable(
    repo: &gix::Repository,
    target: ObjectId,
    ancestor: Option<ObjectId>,
) -> Result<Vec<u8>> {
    ensure!(
        repo.find_header(target)?.kind() == Kind::Commit,
        "Push target must be a commit"
    );
    let mut fast_forward = ancestor.is_none();
    let mut seen = BTreeSet::new();
    let mut queue = vec![(target, Kind::Commit)];
    let mut data = Vec::new();
    let mut total = 0usize;
    while let Some((id, expected_kind)) = queue.pop() {
        let h = repo.find_header(id)?;
        ensure!(
            h.kind() == expected_kind,
            "Unexpected Git object kind in history"
        );
        if !seen.insert(id) {
            continue;
        }
        ensure!(seen.len() <= MAX_ENTRIES, "Pack object count exceeds bound");
        ensure!(h.size() <= MAX_OBJECT as u64, "Git object exceeds bound");
        total = total
            .checked_add(h.size() as usize)
            .context("Pack size overflow")?;
        ensure!(total <= MAX_TOTAL, "Pack expansion exceeds bound");
        let object = repo.find_object(id)?;
        ensure!(
            gix::objs::compute_hash(repo.object_hash(), object.kind, &object.data)? == id,
            "Git object integrity failure"
        );
        if object.kind == Kind::Commit {
            if Some(id) == ancestor {
                fast_forward = true
            }
            let commit = repo.find_commit(id)?;
            let parsed = commit.decode()?;
            queue.push((parsed.tree(), Kind::Tree));
            queue.extend(parsed.parents().map(|oid| (oid, Kind::Commit)));
        } else if object.kind == Kind::Tree {
            for entry in repo.find_tree(id)?.iter() {
                let entry = entry?;
                if !entry.mode().is_commit() {
                    queue.push((
                        entry.oid().to_owned(),
                        if entry.mode().is_tree() {
                            Kind::Tree
                        } else {
                            Kind::Blob
                        },
                    ))
                }
            }
        } else {
            ensure!(
                object.kind == Kind::Blob,
                "Unexpected object in branch history"
            )
        }
        let kind = match object.kind {
            Kind::Commit => 1,
            Kind::Tree => 2,
            Kind::Blob => 3,
            Kind::Tag => 4,
        };
        let mut size = object.data.len();
        let mut byte = (kind << 4) | (size as u8 & 15);
        size >>= 4;
        while size != 0 {
            data.push(byte | 128);
            byte = (size & 127) as u8;
            size >>= 7;
        }
        data.push(byte);
        let mut encoder = ZlibEncoder::new(&mut data, Compression::fast());
        encoder.write_all(&object.data)?;
        encoder.finish()?;
        ensure!(data.len() <= MAX_TOTAL, "Pack output exceeds bound");
    }
    ensure!(fast_forward, "Non-fast-forward update refused");
    let mut out = b"PACK\0\0\0\x02".to_vec();
    out.extend_from_slice(&(seen.len() as u32).to_be_bytes());
    out.extend_from_slice(&data);
    let hash = Sha1::digest(&out);
    out.extend_from_slice(&hash);
    Ok(out)
}
fn take(bytes: &mut &[u8]) -> Result<u8> {
    let (&b, rest) = bytes.split_first().context("Truncated pack")?;
    *bytes = rest;
    Ok(b)
}
fn delta_integer(data: &mut &[u8]) -> Result<usize> {
    let mut n = 0usize;
    for i in 0..5 {
        let x = take(data)?;
        n |= ((x & 127) as usize) << (i * 7);
        if x & 128 == 0 {
            return Ok(n);
        }
    }
    bail!("Invalid delta length")
}
fn preflight(data: &[u8]) -> Result<()> {
    ensure!(
        data.len() >= 32 && data.len() <= MAX_TOTAL && &data[..4] == b"PACK",
        "Invalid or excessive pack"
    );
    ensure!(
        matches!(u32::from_be_bytes(data[4..8].try_into()?), 2 | 3),
        "Unsupported pack version"
    );
    let count = u32::from_be_bytes(data[8..12].try_into()?) as usize;
    ensure!(count <= MAX_ENTRIES, "Pack entry limit");
    let stop = data.len() - 20;
    ensure!(
        Sha1::digest(&data[..stop])[..] == data[stop..],
        "Pack checksum mismatch"
    );
    let mut bytes = &data[12..stop];
    let mut total = 0usize;
    for _ in 0..count {
        let mut b = take(&mut bytes)?;
        let kind = (b >> 4) & 7;
        let mut size = (b & 15) as usize;
        let mut shift = 4;
        while b & 128 != 0 {
            ensure!(shift <= 25, "Excessive pack length");
            b = take(&mut bytes)?;
            size |= ((b & 127) as usize) << shift;
            shift += 7;
        }
        ensure!(size <= MAX_OBJECT, "Pack entry exceeds bound");
        match kind {
            1..=4 => {}
            6 => {
                for t in 0..8 {
                    if take(&mut bytes)? & 128 == 0 {
                        break;
                    }
                    ensure!(t < 7, "Invalid delta offset")
                }
            }
            7 => bytes = bytes.get(20..).context("Truncated delta base")?,
            _ => bail!("Invalid pack kind"),
        }
        let mut decoder = ZlibDecoder::new(bytes);
        let mut inflated = Vec::with_capacity(size.min(65536));
        (&mut decoder)
            .take((size + 1) as u64)
            .read_to_end(&mut inflated)?;
        ensure!(
            inflated.len() == size && decoder.total_in() > 0,
            "Invalid packed length"
        );
        bytes = bytes
            .get(decoder.total_in() as usize..)
            .context("Invalid zlib stream")?;
        let result_len = if kind >= 6 {
            let mut delta = inflated.as_slice();
            let base_len = delta_integer(&mut delta)?;
            let out_len = delta_integer(&mut delta)?;
            ensure!(
                base_len <= MAX_OBJECT && out_len <= MAX_OBJECT,
                "Excessive delta expansion"
            );
            out_len
        } else {
            size
        };
        total = total
            .checked_add(size.max(result_len))
            .context("Size overflow")?;
        ensure!(total <= MAX_TOTAL, "Pack expands beyond budget");
    }
    ensure!(bytes.is_empty(), "Unexpected trailing pack data");
    Ok(())
}
pub fn install(data: &[u8], dir: &Path) -> Result<()> {
    preflight(data)?;
    let options = gix_pack::bundle::write::Options {
        thread_limit: Some(1),
        ..Default::default()
    };
    gix_pack::Bundle::write_to_directory(
        &mut std::io::Cursor::new(data),
        Some(dir),
        &mut gix::progress::Discard,
        &AtomicBool::new(false),
        None::<&gix::odb::Handle>,
        options,
    )?;
    Ok(())
}
