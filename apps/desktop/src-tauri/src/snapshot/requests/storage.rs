//! Fixed-name private request storage; durable drafts remain inert on partial failure.
use super::{ActionRequest, SavedRequest};
use rustix::fs::{mkdirat, open, openat, statat, AtFlags, FileType, Mode, OFlags};
use std::{
    fs::File,
    io::Write,
    os::unix::fs::MetadataExt,
    path::{Component, Path},
    sync::Mutex,
};
const AUDIT: &str = "desktop-action-audit.jsonl";
const MAX_AUDIT_BYTES: u64 = 16 * 1024 * 1024;
static SAVE_LOCK: Mutex<()> = Mutex::new(());
const ERROR: &str = "Request storage is inaccessible or unsafe. Review local drafts before retrying; no workflow action was performed.";

pub(super) struct RequestStore {
    directories: Vec<File>,
    parts: Vec<std::ffi::OsString>,
}
impl RequestStore {
    pub(super) fn open(home: &Path) -> Result<Self, &'static str> {
        super::super::reader::validate_path(home)?;
        let parts: Vec<_> = home
            .components()
            .filter_map(|part| match part {
                Component::Normal(name) => Some(name.to_owned()),
                _ => None,
            })
            .collect();
        if parts.is_empty() {
            return Err(ERROR);
        }
        let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        let mut directories = vec![File::from(
            open("/", flags, Mode::empty()).map_err(|_| ERROR)?,
        )];
        for (index, part) in parts.iter().enumerate() {
            let parent = directories.last().ok_or(ERROR)?;
            if index == parts.len() - 1 {
                match mkdirat(parent, part, Mode::from_raw_mode(0o700)) {
                    Ok(()) => parent.sync_all().map_err(|_| ERROR)?,
                    Err(rustix::io::Errno::EXIST) => (),
                    Err(_) => return Err(ERROR),
                }
            }
            directories.push(File::from(
                openat(parent, part, flags, Mode::empty()).map_err(|_| ERROR)?,
            ));
        }
        let store = Self { directories, parts };
        store.verify()?;
        Ok(store)
    }
    fn home(&self) -> &File {
        self.directories
            .last()
            .expect("validated nonempty directory chain")
    }
    fn verify(&self) -> Result<(), &'static str> {
        for (index, name) in self.parts.iter().enumerate() {
            let entry = statat(&self.directories[index], name, AtFlags::SYMLINK_NOFOLLOW)
                .map_err(|_| ERROR)?;
            let opened = self.directories[index + 1].metadata().map_err(|_| ERROR)?;
            if !opened.is_dir()
                || FileType::from_raw_mode(entry.st_mode) != FileType::Directory
                || entry.st_dev as u64 != opened.dev()
                || entry.st_ino != opened.ino()
            {
                return Err(ERROR);
            }
        }
        let home = self.home().metadata().map_err(|_| ERROR)?;
        if home.uid() != rustix::process::geteuid().as_raw() || home.mode() & 0o7777 != 0o700 {
            return Err(ERROR);
        }
        Ok(())
    }
    fn verify_file(&self, file: &File) -> Result<(), &'static str> {
        self.verify()?;
        verify_private_file(self.home(), AUDIT, file)
    }
}

fn verify_private_file(parent: &File, name: &str, file: &File) -> Result<(), &'static str> {
    let entry = statat(parent, name, AtFlags::SYMLINK_NOFOLLOW).map_err(|_| ERROR)?;
    let opened = file.metadata().map_err(|_| ERROR)?;
    if !opened.is_file()
        || FileType::from_raw_mode(entry.st_mode) != FileType::RegularFile
        || opened.nlink() != 1
        || opened.uid() != rustix::process::geteuid().as_raw()
        || opened.mode() & 0o7777 != 0o600
        || entry.st_dev as u64 != opened.dev()
        || entry.st_ino != opened.ino()
        || entry.st_nlink != 1
        || entry.st_mode & 0o7777 != 0o600
        || entry.st_uid != opened.uid()
    {
        return Err(ERROR);
    }
    Ok(())
}

pub(super) fn save(home: &Path, request: &ActionRequest) -> Result<SavedRequest, &'static str> {
    save_checked(home, request, |_| Ok(()))
}
fn save_checked(
    home: &Path,
    request: &ActionRequest,
    mut checkpoint: impl FnMut(&str) -> Result<(), &'static str>,
) -> Result<SavedRequest, &'static str> {
    let _guard = SAVE_LOCK.lock().map_err(|_| ERROR)?;
    let filename = request.filename()?;
    let bytes = serde_json::to_vec_pretty(request).map_err(|_| ERROR)?;
    if bytes.len() > 256 * 1024 {
        return Err(ERROR);
    }
    let store = RequestStore::open(home)?;
    match mkdirat(store.home(), "action-requests", Mode::from_raw_mode(0o700)) {
        Ok(()) => store.home().sync_all().map_err(|_| ERROR)?,
        Err(rustix::io::Errno::EXIST) => (),
        Err(_) => return Err(ERROR),
    }
    let requests = File::from(
        openat(
            store.home(),
            "action-requests",
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|_| ERROR)?,
    );
    let verify_directory = || {
        store.verify()?;
        let entry = statat(store.home(), "action-requests", AtFlags::SYMLINK_NOFOLLOW)
            .map_err(|_| ERROR)?;
        let opened = requests.metadata().map_err(|_| ERROR)?;
        if FileType::from_raw_mode(entry.st_mode) != FileType::Directory
            || entry.st_dev as u64 != opened.dev()
            || entry.st_ino != opened.ino()
            || opened.uid() != rustix::process::geteuid().as_raw()
            || opened.mode() & 0o7777 != 0o700
        {
            return Err(ERROR);
        }
        Ok(())
    };
    verify_directory()?;
    // Reject existing unsafe modes before a writable open can clear special mode bits.
    match statat(store.home(), AUDIT, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(entry) => {
            if FileType::from_raw_mode(entry.st_mode) != FileType::RegularFile
                || entry.st_nlink != 1
                || entry.st_uid != rustix::process::geteuid().as_raw()
                || entry.st_mode & 0o7777 != 0o600
            {
                return Err(ERROR);
            }
        }
        Err(rustix::io::Errno::NOENT) => (),
        Err(_) => return Err(ERROR),
    }
    let mut audit = File::from(
        openat(
            store.home(),
            AUDIT,
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
    checkpoint("audit-opened")?;
    store.verify_file(&audit)?;
    let mut event = serde_json::to_vec(&request.audit()).map_err(|_| ERROR)?;
    event.push(b'\n');
    if audit.metadata().map_err(|_| ERROR)?.len() + event.len() as u64 > MAX_AUDIT_BYTES {
        return Err(ERROR);
    }
    verify_directory()?;
    let mut file = File::from(openat(&requests, filename.as_str(), OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC, Mode::from_raw_mode(0o600)).map_err(|_| "Request already exists or storage is unavailable. Review recent requests before retrying.")?);
    // Never unlink by name on a failure: an attacker may have replaced that entry.
    checkpoint("request-opened")?;
    verify_directory()?;
    verify_private_file(&requests, &filename, &file)?;
    file.write_all(&bytes).map_err(|_| ERROR)?;
    file.sync_all().map_err(|_| ERROR)?;
    checkpoint("request-synced")?;
    verify_directory()?;
    verify_private_file(&requests, &filename, &file)?;
    requests.sync_all().map_err(|_| ERROR)?;
    checkpoint("request-parent-synced")?;
    verify_directory()?;
    verify_private_file(&requests, &filename, &file)?;
    let audit_recorded = (|| {
        store.verify_file(&audit)?;
        audit.write_all(&event).map_err(|_| ERROR)?;
        audit.sync_all().map_err(|_| ERROR)?;
        checkpoint("audit-synced")?;
        store.verify_file(&audit)?;
        store.home().sync_all().map_err(|_| ERROR)?;
        checkpoint("audit-parent-synced")?;
        store.verify_file(&audit)
    })()
    .is_ok();
    verify_directory()?;
    verify_private_file(&requests, &filename, &file)?;
    Ok(SavedRequest {
        path: home
            .join("action-requests")
            .join(filename)
            .to_string_lossy()
            .into_owned(),
        audit_recorded,
    })
}
#[cfg(test)]
pub(super) fn test_save(
    home: &Path,
    request: &ActionRequest,
    checkpoint: impl FnMut(&str) -> Result<(), &'static str>,
) -> Result<SavedRequest, &'static str> {
    save_checked(home, request, checkpoint)
}
