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

pub(crate) fn owner_valid(uid: u32) -> bool {
    uid == rustix::process::geteuid().as_raw()
}
const FILE: &str = "automations.json";
const LOCK: &str = ".automation-lock";
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

pub(crate) struct AutomationStore {
    directories: Vec<File>,
    parts: Vec<std::ffi::OsString>,
}
impl AutomationStore {
    pub(crate) fn open(home: &Path) -> Result<Self, &'static str> {
        if !home.is_absolute() || home.to_string_lossy().starts_with("//") {
            return Err("storage");
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
                _ => return Err("storage"),
            }
        }
        if parts.is_empty() {
            return Err("storage");
        }
        parts.push("automations".into());
        let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        let mut directories = vec![File::from(
            open("/", flags, Mode::empty()).map_err(|_| "storage")?,
        )];
        for (index, name) in parts.iter().enumerate() {
            let parent = directories.last().ok_or("storage")?;
            reject_workspace(parent)?;
            if index == parts.len() - 1 {
                let metadata = parent.metadata().map_err(|_| "storage")?;
                if !owner_valid(metadata.uid()) || metadata.mode() & 0o7777 != 0o700 {
                    return Err("storage");
                }
            }
            if index >= parts.len() - 2 {
                match mkdirat(parent, name, Mode::from_raw_mode(0o700)) {
                    Ok(()) => parent.sync_all().map_err(|_| "storage")?,
                    Err(rustix::io::Errno::EXIST) => (),
                    Err(_) => return Err("storage"),
                }
            }
            directories.push(File::from(
                openat(parent, name, flags, Mode::empty()).map_err(|_| "storage")?,
            ));
        }
        let store = Self { directories, parts };
        store.verify()?;
        let mut entries = 0;
        for entry in rustix::fs::Dir::read_from(store.directory()).map_err(|_| "storage")? {
            entry.map_err(|_| "storage")?;
            entries += 1;
            if entries > 64 {
                return Err("storage");
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
                .map_err(|_| "storage")?;
            let file = self.directories[index + 1]
                .metadata()
                .map_err(|_| "storage")?;
            if !file.is_dir()
                || FileType::from_raw_mode(entry.st_mode) != FileType::Directory
                || entry.st_dev as u64 != file.dev()
                || entry.st_ino != file.ino()
                || (index >= self.parts.len() - 2
                    && (!owner_valid(file.uid()) || file.mode() & 0o7777 != 0o700))
            {
                return Err("storage");
            }
        }
        Ok(())
    }
    fn file_directory(&self, name: &str) -> &File {
        if name == "desktop-automation-audit.jsonl" {
            &self.directories[self.directories.len() - 2]
        } else {
            self.directory()
        }
    }
    fn verify_file(&self, name: &str, file: &File) -> Result<(), &'static str> {
        self.verify()?;
        let entry = statat(self.file_directory(name), name, AtFlags::SYMLINK_NOFOLLOW)
            .map_err(|_| "storage")?;
        let file = file.metadata().map_err(|_| "storage")?;
        if !file.is_file()
            || FileType::from_raw_mode(entry.st_mode) != FileType::RegularFile
            || entry.st_dev as u64 != file.dev()
            || entry.st_ino != file.ino()
            || file.nlink() != 1
            || entry.st_nlink != 1
            || !owner_valid(file.uid())
            || entry.st_uid != file.uid()
            || file.mode() & 0o7777 != 0o600
            || entry.st_mode & 0o7777 != 0o600
        {
            return Err("storage");
        }
        Ok(())
    }
    fn check_entry(&self, name: &str) -> Result<bool, &'static str> {
        self.verify()?;
        match statat(self.file_directory(name), name, AtFlags::SYMLINK_NOFOLLOW) {
            Ok(entry)
                if FileType::from_raw_mode(entry.st_mode) == FileType::RegularFile
                    && entry.st_nlink == 1
                    && owner_valid(entry.st_uid)
                    && entry.st_mode & 0o7777 == 0o600 =>
            {
                Ok(true)
            }
            Err(rustix::io::Errno::NOENT) => Ok(false),
            _ => Err("storage"),
        }
    }
    fn lock(&self) -> Result<StoreLock, &'static str> {
        // Keep trusted threads serialized too; the OS advisory lock also covers other processes.
        let guard = PROCESS_LOCK.try_lock().map_err(|_| "busy")?;
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
            .map_err(|_| "storage")?,
        );
        self.verify_file(LOCK, &file)?;
        if file.metadata().map_err(|_| "storage")?.len() != 0 {
            return Err("storage");
        }
        // Descriptor-held lock serializes trusted writers across processes, not only threads.
        flock(&file, FlockOperation::NonBlockingLockExclusive).map_err(|_| "busy")?;
        self.verify_file(LOCK, &file)?;
        self.directory().sync_all().map_err(|_| "storage")?;
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
            .map_err(|_| "storage")?,
        );
        self.verify_file(FILE, &file)?;
        let length = file.metadata().map_err(|_| "storage")?.len();
        if length == 0 || length > MAX_BYTES as u64 {
            return Err("invalid_state");
        }
        let mut bytes = Vec::new();
        (&mut file)
            .take(MAX_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "storage")?;
        self.verify_file(FILE, &file)?;
        if bytes.len() != length as usize {
            return Err("invalid_state");
        }
        let doc: Document = serde_json::from_slice(&bytes).map_err(|_| "invalid_state")?;
        doc.validate()?;
        Ok((doc, Some(file)))
    }
    pub fn read_document(&self) -> Result<Document> {
        let _lock = self.lock()?;
        Ok(self.read()?.0)
    }
    pub fn transaction<T>(
        &self,
        operation: impl FnOnce(&mut Document) -> Result<(T, Vec<super::mutations::AuditEvent>)>,
    ) -> Result<(T, bool)> {
        let lock = self.lock()?;
        let (mut doc, previous) = self.read()?;
        let (result, events) = operation(&mut doc)?;
        if events.is_empty() {
            return Ok((result, true));
        }
        let bytes = doc.bytes()?;
        for event in &events {
            self.audit_locked(event, &lock).map_err(|_| "audit")?;
        }
        if let Err(error) =
            self.write_bytes_checked(FILE, &bytes, previous.as_ref(), &lock, |_| Ok(()))
        {
            for event in &events {
                let mut failed = event.clone();
                failed.event = "desktop.automation.failed";
                failed.result = "outcome_uncertain";
                let _ = self.audit_locked(&failed, &lock);
            }
            return Err(error);
        }
        let mut recorded = true;
        for mut event in events {
            event.result = "completed";
            recorded &= self.audit_locked(&event, &lock).is_ok();
        }
        Ok((result, recorded))
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
        let name = format!(".automation-{}.tmp", uuid::Uuid::new_v4());
        let mut file = File::from(
            openat(
                self.directory(),
                name.as_str(),
                OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::from_raw_mode(0o600),
            )
            .map_err(|_| "storage")?,
        );
        checkpoint("opened")?;
        self.verify_file(&name, &file)?;
        self.verify_file(LOCK, lock)?;
        file.write_all(bytes).map_err(|_| "storage")?;
        file.sync_all().map_err(|_| "storage")?;
        checkpoint("file-synced")?;
        self.verify_file(&name, &file)?;
        self.verify_file(LOCK, lock)?;
        match previous {
            Some(previous) => {
                self.verify_file(filename, previous)?;
                renameat(self.directory(), name.as_str(), self.directory(), filename)
                    .map_err(|_| "storage")?;
            }
            None => renameat_with(
                self.directory(),
                name.as_str(),
                self.directory(),
                filename,
                RenameFlags::NOREPLACE,
            )
            .map_err(|_| "storage")?,
        }
        // From this point metadata might be visible even if durability/identity verification fails.
        let mut finish = || -> Result<(), &'static str> {
            checkpoint("installed")?;
            self.verify_file(filename, &file)?;
            self.verify_file(LOCK, lock)?;
            self.directory().sync_all().map_err(|_| "storage")?;
            checkpoint("parent-synced")?;
            self.verify_file(filename, &file)
        };
        finish().map_err(|_| "storage")
        // Failed staging files stay private; never unlink a possibly replaced name on failure.
    }
    fn audit_locked(&self, event: &super::mutations::AuditEvent, lock: &File) -> Result<()> {
        let name = "desktop-automation-audit.jsonl";
        self.check_entry(name)?;
        let mut file = File::from(
            openat(
                self.file_directory(name),
                name,
                OFlags::WRONLY
                    | OFlags::APPEND
                    | OFlags::CREATE
                    | OFlags::NOFOLLOW
                    | OFlags::NONBLOCK
                    | OFlags::CLOEXEC,
                Mode::from_raw_mode(0o600),
            )
            .map_err(|_| "audit")?,
        );
        self.verify_file(name, &file).map_err(|_| "audit")?;
        let mut bytes = serde_json::to_vec(event).map_err(|_| "audit")?;
        bytes.push(b'\n');
        if file.metadata().map_err(|_| "audit")?.len() + bytes.len() as u64 > 4 * 1024 * 1024 {
            return Err("audit");
        }
        file.write_all(&bytes).map_err(|_| "audit")?;
        file.sync_all().map_err(|_| "audit")?;
        self.verify_file(name, &file).map_err(|_| "audit")?;
        self.verify_file(LOCK, lock).map_err(|_| "audit")?;
        self.file_directory(name).sync_all().map_err(|_| "audit")?;
        Ok(())
    }
}

fn reject_workspace(directory: &File) -> Result<()> {
    for name in [".git", "project.yaml"] {
        match statat(directory, name, AtFlags::SYMLINK_NOFOLLOW) {
            Ok(_) => return Err("storage"),
            Err(rustix::io::Errno::NOENT) => (),
            Err(_) => return Err("storage"),
        }
    }
    match openat(
        directory,
        ".ghost",
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Ok(fd) => match statat(&fd, "project.yaml", AtFlags::SYMLINK_NOFOLLOW) {
            Ok(_) => Err("storage"),
            Err(rustix::io::Errno::NOENT) => Ok(()),
            Err(_) => Err("storage"),
        },
        Err(rustix::io::Errno::NOENT) => Ok(()),
        Err(_) => Err("storage"),
    }
}
