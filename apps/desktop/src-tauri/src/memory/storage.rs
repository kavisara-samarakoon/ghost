use super::model::{Document, MAX_BYTES};
type Result<T, E = &'static str> = std::result::Result<T, E>;
use rustix::fs::{
    flock, mkdirat, open, openat, renameat, renameat_with, statat, AtFlags, FileType,
    FlockOperation, Mode, OFlags, RenameFlags,
};
use std::fs::File;
use std::io::{Read, Write};
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path};

const FILE: &str = "personal-memory.json";
const LOCK: &str = ".memory-lock";
static PROCESS_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
struct StoreLock {
    file: File,
    _guard: std::sync::MutexGuard<'static, ()>,
}
impl std::ops::Deref for StoreLock {
    type Target = File;
    fn deref(&self) -> &File {
        &self.file
    }
}

pub(crate) struct MemoryStore {
    directories: Vec<File>,
    parts: Vec<std::ffi::OsString>,
}
impl MemoryStore {
    pub(crate) fn open(home: &Path) -> Result<Self, &'static str> {
        if !home.is_absolute() || home.to_string_lossy().starts_with("//") {
            return Err("storage_failed");
        }
        let mut parts = Vec::new();
        for part in home.components() {
            match part {
                Component::RootDir => (),
                Component::Normal(name)
                    if name
                        .to_str()
                        .is_some_and(|name| !name.to_ascii_lowercase().starts_with(".env")) =>
                {
                    parts.push(name.to_owned())
                }
                _ => return Err("storage_failed"),
            }
        }
        if parts.is_empty() {
            return Err("storage_failed");
        }
        parts.push("memory".into());
        let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        let mut directories = vec![File::from(
            open("/", flags, Mode::empty()).map_err(|_| "storage_failed")?,
        )];
        for (index, name) in parts.iter().enumerate() {
            let parent = directories.last().ok_or("storage_failed")?;
            reject_workspace(parent)?;
            if index == parts.len() - 1 {
                let metadata = parent.metadata().map_err(|_| "storage_failed")?;
                if metadata.uid() != rustix::process::geteuid().as_raw()
                    || metadata.mode() & 0o7777 != 0o700
                {
                    return Err("storage_failed");
                }
            }
            if index >= parts.len() - 2 {
                match mkdirat(parent, name, Mode::from_raw_mode(0o700)) {
                    Ok(()) => parent.sync_all().map_err(|_| "storage_failed")?,
                    Err(rustix::io::Errno::EXIST) => (),
                    Err(_) => return Err("storage_failed"),
                }
            }
            directories.push(File::from(
                openat(parent, name, flags, Mode::empty()).map_err(|_| "storage_failed")?,
            ));
        }
        let store = Self { directories, parts };
        store.verify()?;
        let mut entries = 0;
        for entry in rustix::fs::Dir::read_from(store.directory()).map_err(|_| "storage_failed")? {
            entry.map_err(|_| "storage_failed")?;
            entries += 1;
            if entries > 64 {
                return Err("storage_failed");
            }
        }
        Ok(store)
    }
    fn directory(&self) -> &File {
        self.directories
            .last()
            .expect("validated private directory chain")
    }
    fn verify(&self) -> Result<(), &'static str> {
        for (index, name) in self.parts.iter().enumerate() {
            reject_workspace(&self.directories[index])?;
            let entry = statat(&self.directories[index], name, AtFlags::SYMLINK_NOFOLLOW)
                .map_err(|_| "storage_failed")?;
            let file = self.directories[index + 1]
                .metadata()
                .map_err(|_| "storage_failed")?;
            if !file.is_dir()
                || FileType::from_raw_mode(entry.st_mode) != FileType::Directory
                || entry.st_dev as u64 != file.dev()
                || entry.st_ino != file.ino()
                || (index >= self.parts.len() - 2
                    && (file.uid() != rustix::process::geteuid().as_raw()
                        || file.mode() & 0o7777 != 0o700))
            {
                return Err("storage_failed");
            }
        }
        Ok(())
    }
    fn verify_file(&self, name: &str, file: &File) -> Result<(), &'static str> {
        self.verify()?;
        let entry = statat(self.directory(), name, AtFlags::SYMLINK_NOFOLLOW)
            .map_err(|_| "storage_failed")?;
        let file = file.metadata().map_err(|_| "storage_failed")?;
        if !file.is_file()
            || FileType::from_raw_mode(entry.st_mode) != FileType::RegularFile
            || entry.st_dev as u64 != file.dev()
            || entry.st_ino != file.ino()
            || file.nlink() != 1
            || entry.st_nlink != 1
            || file.uid() != rustix::process::geteuid().as_raw()
            || entry.st_uid != file.uid()
            || file.mode() & 0o7777 != 0o600
            || entry.st_mode & 0o7777 != 0o600
        {
            return Err("storage_failed");
        }
        Ok(())
    }
    fn check_entry(&self, name: &str) -> Result<bool, &'static str> {
        self.verify()?;
        match statat(self.directory(), name, AtFlags::SYMLINK_NOFOLLOW) {
            Ok(entry)
                if FileType::from_raw_mode(entry.st_mode) == FileType::RegularFile
                    && entry.st_nlink == 1
                    && entry.st_uid == rustix::process::geteuid().as_raw()
                    && entry.st_mode & 0o7777 == 0o600 =>
            {
                Ok(true)
            }
            Err(rustix::io::Errno::NOENT) => Ok(false),
            _ => Err("storage_failed"),
        }
    }
    fn lock(&self) -> Result<StoreLock, &'static str> {
        // Keep trusted threads serialized too; the OS advisory lock also covers other processes.
        let guard = PROCESS_LOCK.lock().map_err(|_| "storage_failed")?;
        // Check unsafe pre-existing modes before a writable open can clear special mode bits.
        self.check_entry(LOCK)?;
        let file = File::from(
            openat(
                self.directory(),
                LOCK,
                OFlags::RDWR
                    | OFlags::CREATE
                    | OFlags::NOFOLLOW
                    | OFlags::NONBLOCK
                    | OFlags::CLOEXEC,
                Mode::from_raw_mode(0o600),
            )
            .map_err(|_| "storage_failed")?,
        );
        self.verify_file(LOCK, &file)?;
        if file.metadata().map_err(|_| "storage_failed")?.len() != 0 {
            return Err("storage_failed");
        }
        // Descriptor-held lock serializes trusted writers across processes, not only threads.
        flock(&file, FlockOperation::LockExclusive).map_err(|_| "storage_failed")?;
        self.verify_file(LOCK, &file)?;
        self.directory().sync_all().map_err(|_| "storage_failed")?;
        Ok(StoreLock {
            file,
            _guard: guard,
        })
    }
    fn read(&self) -> Result<(Document, Option<File>)> {
        if !self.check_entry(FILE)? {
            return Ok((Document::empty(), None));
        }
        let mut file = File::from(
            openat(
                self.directory(),
                FILE,
                OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|_| "storage_failed")?,
        );
        self.verify_file(FILE, &file)?;
        let length = file.metadata().map_err(|_| "storage_failed")?.len();
        if length == 0 || length > MAX_BYTES as u64 {
            return Err("corrupt_memory");
        }
        let mut bytes = Vec::new();
        (&mut file)
            .take(MAX_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "storage_failed")?;
        self.verify_file(FILE, &file)?;
        if bytes.len() != length as usize {
            return Err("corrupt_memory");
        }
        let doc: Document = serde_json::from_slice(&bytes).map_err(|_| "corrupt_memory")?;
        doc.validate()?;
        Ok((doc, Some(file)))
    }
    pub fn list(&self) -> Result<Vec<super::model::Record>> {
        let _lock = self.lock()?;
        Ok(self.read()?.0.records)
    }
    pub fn mutate(
        &self,
        event: &super::mutations::AuditEvent,
        operation: impl FnOnce(&mut Document) -> Result<()>,
        mut checkpoint: impl FnMut(&str) -> Result<()>,
    ) -> Result<bool> {
        let lock = self.lock()?;
        let (mut doc, previous) = self.read()?;
        operation(&mut doc)?;
        let bytes = doc.bytes()?;
        checkpoint("intent")?;
        self.audit_locked(event, &lock)
            .map_err(|_| "audit_failed")?;
        if let Err(error) =
            self.write_bytes_checked(FILE, &bytes, previous.as_ref(), &lock, &mut checkpoint)
        {
            let mut failed = event.clone();
            failed.result = if error == "write_outcome_uncertain" {
                "outcome_uncertain"
            } else {
                "failed_before_write"
            };
            let _ = self.audit_locked(&failed, &lock);
            return Err(error);
        }
        let mut completed = event.clone();
        completed.result = "completed";
        Ok(checkpoint("completion")
            .and_then(|_| self.audit_locked(&completed, &lock))
            .is_ok())
    }
    fn write_bytes_checked(
        &self,
        filename: &str,
        bytes: &[u8],
        previous: Option<&File>,
        lock: &File,
        mut checkpoint: impl FnMut(&str) -> Result<(), &'static str>,
    ) -> Result<(), &'static str> {
        self.verify_file(LOCK, lock)?;
        let name = format!(".memory-{}.tmp", uuid::Uuid::new_v4());
        let mut file = File::from(
            openat(
                self.directory(),
                name.as_str(),
                OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::from_raw_mode(0o600),
            )
            .map_err(|_| "storage_failed")?,
        );
        checkpoint("opened")?;
        self.verify_file(&name, &file)?;
        self.verify_file(LOCK, lock)?;
        file.write_all(bytes).map_err(|_| "storage_failed")?;
        file.sync_all().map_err(|_| "storage_failed")?;
        checkpoint("file-synced")?;
        self.verify_file(&name, &file)?;
        self.verify_file(LOCK, lock)?;
        match previous {
            Some(previous) => {
                self.verify_file(filename, previous)?;
                renameat(self.directory(), name.as_str(), self.directory(), filename)
                    .map_err(|_| "storage_failed")?;
            }
            None => renameat_with(
                self.directory(),
                name.as_str(),
                self.directory(),
                filename,
                RenameFlags::NOREPLACE,
            )
            .map_err(|_| "storage_failed")?,
        }
        // From this point metadata might be visible even if durability/identity verification fails.
        let mut finish = || -> Result<(), &'static str> {
            checkpoint("installed")?;
            self.verify_file(filename, &file)?;
            self.verify_file(LOCK, lock)?;
            self.directory().sync_all().map_err(|_| "storage_failed")?;
            checkpoint("parent-synced")?;
            self.verify_file(filename, &file)
        };
        finish().map_err(|_| "write_outcome_uncertain")
        // Failed staging files stay private; never unlink a possibly replaced name on failure.
    }
    fn audit_locked(&self, event: &super::mutations::AuditEvent, lock: &File) -> Result<()> {
        let name = "memory-audit.jsonl";
        self.check_entry(name)?;
        let mut file = File::from(
            openat(
                self.directory(),
                name,
                OFlags::WRONLY
                    | OFlags::APPEND
                    | OFlags::CREATE
                    | OFlags::NOFOLLOW
                    | OFlags::NONBLOCK
                    | OFlags::CLOEXEC,
                Mode::from_raw_mode(0o600),
            )
            .map_err(|_| "audit_failed")?,
        );
        self.verify_file(name, &file).map_err(|_| "audit_failed")?;
        let mut bytes = serde_json::to_vec(event).map_err(|_| "audit_failed")?;
        bytes.push(b'\n');
        if file.metadata().map_err(|_| "audit_failed")?.len() + bytes.len() as u64 > 4 * 1024 * 1024
        {
            return Err("audit_failed");
        }
        file.write_all(&bytes).map_err(|_| "audit_failed")?;
        file.sync_all().map_err(|_| "audit_failed")?;
        self.verify_file(name, &file).map_err(|_| "audit_failed")?;
        self.verify_file(LOCK, lock).map_err(|_| "audit_failed")?;
        self.directory().sync_all().map_err(|_| "audit_failed")?;
        Ok(())
    }
}

fn reject_workspace(directory: &File) -> Result<()> {
    for name in [".git", "project.yaml"] {
        match statat(directory, name, AtFlags::SYMLINK_NOFOLLOW) {
            Ok(_) => return Err("storage_failed"),
            Err(rustix::io::Errno::NOENT) => (),
            Err(_) => return Err("storage_failed"),
        }
    }
    match openat(
        directory,
        ".ghost",
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Ok(fd) => match statat(&fd, "project.yaml", AtFlags::SYMLINK_NOFOLLOW) {
            Ok(_) => Err("storage_failed"),
            Err(rustix::io::Errno::NOENT) => Ok(()),
            Err(_) => Err("storage_failed"),
        },
        Err(rustix::io::Errno::NOENT) => Ok(()),
        Err(_) => Err("storage_failed"),
    }
}
