//! Only a fixed content-free audit file is writable. Audio and transcript are never passed here.
use super::AuditEvent;
use rustix::fs::{mkdirat, open, openat, statat, AtFlags, FileType, Mode, OFlags};
use std::fs::File;
use std::io::Write;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;

const AUDIT: &str = "desktop-voice-audit.jsonl";
const MAX_AUDIT_BYTES: u64 = 16 * 1024 * 1024;
static AUDIT_LOCK: Mutex<()> = Mutex::new(());
const ERROR: &str = "audit";

pub(super) fn resolve_home() -> Result<PathBuf, &'static str> {
    let value = std::env::var_os("GHOST_HOME");
    let home = match value {
        Some(value) => {
            let value = value.into_string().map_err(|_| ERROR)?;
            if value.trim().is_empty() {
                return Err(ERROR);
            }
            if value == "~" {
                std::env::home_dir().ok_or(ERROR)?
            } else if let Some(suffix) = value.strip_prefix("~/") {
                std::env::home_dir().ok_or(ERROR)?.join(suffix)
            } else if value.starts_with('~') {
                return Err(ERROR);
            } else {
                PathBuf::from(value)
            }
        }
        None => std::env::home_dir().ok_or(ERROR)?.join(".ghost"),
    };
    let home = if home.is_absolute() {
        home
    } else {
        std::env::current_dir().map_err(|_| ERROR)?.join(home)
    };
    validate_path(&home)?;
    Ok(home)
}
fn validate_path(path: &Path) -> Result<(), &'static str> {
    if !path.is_absolute() || path.to_string_lossy().starts_with("//") {
        return Err(ERROR);
    }
    for part in path.components() {
        match part {
            Component::RootDir => (),
            Component::Normal(name)
                if name
                    .to_str()
                    .is_some_and(|name| !name.to_ascii_lowercase().starts_with(".env")) =>
            {
                ()
            }
            _ => return Err(ERROR),
        }
    }
    Ok(())
}

pub(super) struct AuditStore {
    directories: Vec<File>,
    parts: Vec<std::ffi::OsString>,
}
impl AuditStore {
    pub(super) fn open(home: &Path) -> Result<Self, &'static str> {
        validate_path(home)?;
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
        let entry = statat(self.home(), AUDIT, AtFlags::SYMLINK_NOFOLLOW).map_err(|_| ERROR)?;
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
    pub(super) fn append(&self, event: AuditEvent) -> Result<(), &'static str> {
        self.append_checked(event, |_| Ok(()))
    }
    fn append_checked(
        &self,
        event: AuditEvent,
        mut checkpoint: impl FnMut(&str) -> Result<(), &'static str>,
    ) -> Result<(), &'static str> {
        let _guard = AUDIT_LOCK.lock().map_err(|_| ERROR)?;
        self.verify()?;
        let mut file = File::from(
            openat(
                self.home(),
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
        checkpoint("opened")?;
        self.verify_file(&file)?;
        let mut bytes = serde_json::to_vec(&event).map_err(|_| ERROR)?;
        bytes.push(b'\n');
        if file.metadata().map_err(|_| ERROR)?.len() + bytes.len() as u64 > MAX_AUDIT_BYTES {
            return Err(ERROR);
        }
        file.write_all(&bytes).map_err(|_| ERROR)?;
        file.sync_all().map_err(|_| ERROR)?;
        checkpoint("file-synced")?;
        self.verify_file(&file)?;
        self.home().sync_all().map_err(|_| ERROR)?;
        checkpoint("parent-synced")?;
        self.verify_file(&file)
    }
    #[cfg(test)]
    pub(super) fn test_append(
        &self,
        event: AuditEvent,
        checkpoint: impl FnMut(&str) -> Result<(), &'static str>,
    ) -> Result<(), &'static str> {
        self.append_checked(event, checkpoint)
    }
}
