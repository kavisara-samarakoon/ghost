//! Fixed-name, descriptor-relative storage. Never follow links or overwrite a request.
use super::{ActionRequest, SavedRequest};
use rustix::fs::{mkdirat, open, openat, unlinkat, AtFlags, Mode, OFlags};
use std::{fs::File, io::Write, os::unix::fs::MetadataExt, path::Path, sync::Mutex};

static SAVE_LOCK: Mutex<()> = Mutex::new(());
const ERROR: &str = "Request storage is inaccessible or unsafe. No workflow action was performed.";

fn directory(parent: &File, name: &std::ffi::OsStr, create: bool) -> Result<File, &'static str> {
    if create {
        match mkdirat(parent, name, Mode::from_raw_mode(0o700)) {
            Ok(()) | Err(rustix::io::Errno::EXIST) => {}
            Err(_) => return Err(ERROR),
        }
    }
    openat(
        parent,
        name,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map(File::from)
    .map_err(|_| ERROR)
}

pub(super) fn save(home: &Path, request: &ActionRequest) -> Result<SavedRequest, &'static str> {
    let _guard = SAVE_LOCK.lock().map_err(|_| ERROR)?;
    let filename = request.filename()?;
    super::super::reader::validate_path(home)?;
    let mut parent = File::from(
        open(
            "/",
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|_| ERROR)?,
    );
    let parts: Vec<_> = home
        .components()
        .filter_map(|part| match part {
            std::path::Component::Normal(name) => Some(name),
            _ => None,
        })
        .collect();
    if parts.is_empty() {
        return Err(ERROR);
    }
    for (index, part) in parts.iter().enumerate() {
        parent = directory(&parent, part, index == parts.len() - 1)?;
    }
    let requests = directory(&parent, "action-requests".as_ref(), true)?;
    // Validate the append target before creating a request. NONBLOCK rejects FIFOs safely.
    let mut audit = File::from(
        openat(
            &parent,
            "desktop-action-audit.jsonl",
            OFlags::WRONLY
                | OFlags::APPEND
                | OFlags::CREATE
                | OFlags::NOFOLLOW
                | OFlags::NONBLOCK
                | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o600),
        )
        .map_err(|_| ERROR)?,
    );
    let metadata = audit.metadata().map_err(|_| ERROR)?;
    if !metadata.is_file() || metadata.nlink() != 1 || metadata.mode() & 0o077 != 0 {
        return Err(ERROR);
    }
    let mut file = File::from(openat(&requests, filename.as_str(),
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::from_raw_mode(0o600)).map_err(|_| "Request already exists or storage is unavailable. Review recent requests before retrying.")?);
    let bytes = serde_json::to_vec_pretty(request).map_err(|_| ERROR)?;
    if file
        .write_all(&bytes)
        .and_then(|_| file.sync_all())
        .is_err()
    {
        let _ = unlinkat(&requests, filename.as_str(), AtFlags::empty());
        return Err(ERROR);
    }
    // A saved draft remains useful if auditing fails; report this explicitly, never invite a duplicate retry.
    let mut event = serde_json::to_vec(&request.audit()).map_err(|_| ERROR)?;
    event.push(b'\n');
    let audit_recorded = audit
        .write_all(&event)
        .and_then(|_| audit.sync_all())
        .is_ok();
    Ok(SavedRequest {
        path: home
            .join("action-requests")
            .join(filename)
            .to_string_lossy()
            .into_owned(),
        audit_recorded,
    })
}
