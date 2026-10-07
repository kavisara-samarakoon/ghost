//! Only content-free intent audits and explicitly saved inert plans are writable.
use super::AuditEvent;
use rustix::fs::{mkdirat, open, openat, statat, AtFlags, FileType, Mode, OFlags};
use std::fs::File;
use std::io::Write;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;

const AUDIT: &str = "desktop-intent-audit.jsonl";
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

pub(crate) struct IntentStore {
    directories: Vec<File>,
    parts: Vec<std::ffi::OsString>,
    path: PathBuf,
}
impl IntentStore {
    pub(crate) fn open(home: &Path) -> Result<Self, &'static str> {
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
        let store = Self {
            directories,
            parts,
            path: home.to_owned(),
        };
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
    pub(super) fn save_plan(&self, bytes: &[u8]) -> Result<(String, String), &'static str> {
        let timestamp = chrono::DateTime::<chrono::Utc>::from(std::time::SystemTime::now())
            .format("%Y%m%dT%H%M%S%fZ");
        let id = format!("{timestamp}-{}", uuid::Uuid::new_v4().simple());
        self.save_plan_checked(bytes, &id, |_| Ok(()))
    }
    fn save_plan_checked(
        &self,
        bytes: &[u8],
        id: &str,
        mut checkpoint: impl FnMut(&str) -> Result<(), &'static str>,
    ) -> Result<(String, String), &'static str> {
        self.verify()?;
        if bytes.is_empty()
            || bytes.len() > 64 * 1024
            || id.len() > 100
            || id.is_empty()
            || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            return Err(ERROR);
        }
        match mkdirat(self.home(), "intent-plans", Mode::from_raw_mode(0o700)) {
            Ok(()) => self.home().sync_all().map_err(|_| ERROR)?,
            Err(rustix::io::Errno::EXIST) => (),
            Err(_) => return Err(ERROR),
        }
        let directory = File::from(
            openat(
                self.home(),
                "intent-plans",
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|_| ERROR)?,
        );
        let verify_directory = || {
            self.verify()?;
            let entry = statat(self.home(), "intent-plans", AtFlags::SYMLINK_NOFOLLOW)
                .map_err(|_| ERROR)?;
            let opened = directory.metadata().map_err(|_| ERROR)?;
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
        let name = format!("{id}.json");
        let mut file = File::from(
            openat(
                &directory,
                &name,
                OFlags::WRONLY
                    | OFlags::CREATE
                    | OFlags::EXCL
                    | OFlags::NOFOLLOW
                    | OFlags::NONBLOCK
                    | OFlags::CLOEXEC,
                Mode::from_raw_mode(0o600),
            )
            .map_err(|_| ERROR)?,
        );
        checkpoint("opened")?;
        verify_directory()?;
        verify_private_file(&directory, &name, &file)?;
        file.write_all(bytes).map_err(|_| ERROR)?;
        file.sync_all().map_err(|_| ERROR)?;
        checkpoint("file-synced")?;
        verify_directory()?;
        verify_private_file(&directory, &name, &file)?;
        directory.sync_all().map_err(|_| ERROR)?;
        checkpoint("parent-synced")?;
        verify_directory()?;
        verify_private_file(&directory, &name, &file)?;
        Ok((
            self.path
                .join("intent-plans")
                .join(name)
                .to_str()
                .ok_or(ERROR)?
                .into(),
            id.into(),
        ))
    }
    #[cfg(test)]
    pub(super) fn test_save(
        &self,
        bytes: &[u8],
        id: &str,
        checkpoint: impl FnMut(&str) -> Result<(), &'static str>,
    ) -> Result<(String, String), &'static str> {
        self.save_plan_checked(bytes, id, checkpoint)
    }
    pub(super) fn append(&self, event: AuditEvent) -> Result<(), &'static str> {
        self.append_checked(event, |_| Ok(()))
    }
    fn append_checked(
        &self,
        event: AuditEvent,
        checkpoint: impl FnMut(&str) -> Result<(), &'static str>,
    ) -> Result<(), &'static str> {
        self.append_named(AUDIT, &event, checkpoint)
    }
    pub(crate) fn append_jarvis(
        &self,
        event: &crate::jarvis::interpret::AuditEvent,
    ) -> Result<(), &'static str> {
        self.append_named("desktop-jarvis-audit.jsonl", event, |_| Ok(()))
    }
    fn append_named(
        &self,
        name: &str,
        event: &impl serde::Serialize,
        mut checkpoint: impl FnMut(&str) -> Result<(), &'static str>,
    ) -> Result<(), &'static str> {
        let _guard = AUDIT_LOCK.lock().map_err(|_| ERROR)?;
        self.verify()?;
        // Check existing mode before a writable open: some systems clear special mode bits
        // on open, which must not disguise an unsafe pre-existing entry as a private file.
        match statat(self.home(), name, AtFlags::SYMLINK_NOFOLLOW) {
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
        let mut file = File::from(
            openat(
                self.home(),
                name,
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
        self.verify()?;
        verify_private_file(self.home(), name, &file)?;
        let mut bytes = serde_json::to_vec(&event).map_err(|_| ERROR)?;
        bytes.push(b'\n');
        if file.metadata().map_err(|_| ERROR)?.len() + bytes.len() as u64 > MAX_AUDIT_BYTES {
            return Err(ERROR);
        }
        file.write_all(&bytes).map_err(|_| ERROR)?;
        file.sync_all().map_err(|_| ERROR)?;
        checkpoint("file-synced")?;
        self.verify()?;
        verify_private_file(self.home(), name, &file)?;
        self.home().sync_all().map_err(|_| ERROR)?;
        checkpoint("parent-synced")?;
        self.verify()?;
        verify_private_file(self.home(), name, &file)
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
