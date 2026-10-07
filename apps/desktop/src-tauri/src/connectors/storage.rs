use super::accounts::{Account, AccountFile, AccountRepository, MAX_ACCOUNTS, MAX_ACCOUNT_BYTES};
use super::ConnectorError;
use crate::credentials::AccountId;
use rustix::fs::{
    flock, mkdirat, open, openat, renameat, renameat_with, statat, AtFlags, FileType,
    FlockOperation, Mode, OFlags, RenameFlags,
};
use std::fs::File;
use std::io::{Read, Write};
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path};

const FILE: &str = "accounts.json";
const LOCK: &str = ".accounts-lock";
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

pub(crate) struct AccountStore {
    directories: Vec<File>,
    parts: Vec<std::ffi::OsString>,
}
impl AccountStore {
    fn read_auxiliary(&self, name: &str) -> Result<Option<(Vec<u8>, File)>, ConnectorError> {
        if !self.check_entry(name)? {
            return Ok(None);
        }
        let mut file = File::from(
            openat(
                self.directory(),
                name,
                OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|_| ConnectorError::Storage)?,
        );
        self.verify_file(name, &file)?;
        let length = file.metadata().map_err(|_| ConnectorError::Storage)?.len();
        if length == 0 || length > 64 * 1024 {
            return Err(ConnectorError::Storage);
        }
        let mut bytes = Vec::new();
        (&mut file)
            .take(64 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| ConnectorError::Storage)?;
        self.verify_file(name, &file)?;
        if bytes.len() != length as usize {
            return Err(ConnectorError::Storage);
        }
        Ok(Some((bytes, file)))
    }
    pub(crate) fn open(home: &Path) -> Result<Self, ConnectorError> {
        if !home.is_absolute() || home.to_string_lossy().starts_with("//") {
            return Err(ConnectorError::Storage);
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
                _ => return Err(ConnectorError::Storage),
            }
        }
        if parts.is_empty() {
            return Err(ConnectorError::Storage);
        }
        parts.push("connectors".into());
        let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        let mut directories = vec![File::from(
            open("/", flags, Mode::empty()).map_err(|_| ConnectorError::Storage)?,
        )];
        for (index, name) in parts.iter().enumerate() {
            let parent = directories.last().ok_or(ConnectorError::Storage)?;
            if index == parts.len() - 1 {
                let metadata = parent.metadata().map_err(|_| ConnectorError::Storage)?;
                if metadata.uid() != rustix::process::geteuid().as_raw()
                    || metadata.mode() & 0o7777 != 0o700
                {
                    return Err(ConnectorError::Storage);
                }
            }
            if index >= parts.len() - 2 {
                match mkdirat(parent, name, Mode::from_raw_mode(0o700)) {
                    Ok(()) => parent.sync_all().map_err(|_| ConnectorError::Storage)?,
                    Err(rustix::io::Errno::EXIST) => (),
                    Err(_) => return Err(ConnectorError::Storage),
                }
            }
            directories.push(File::from(
                openat(parent, name, flags, Mode::empty()).map_err(|_| ConnectorError::Storage)?,
            ));
        }
        let store = Self { directories, parts };
        store.verify()?;
        Ok(store)
    }
    fn directory(&self) -> &File {
        self.directories
            .last()
            .expect("validated private directory chain")
    }
    fn verify(&self) -> Result<(), ConnectorError> {
        for (index, name) in self.parts.iter().enumerate() {
            let entry = statat(&self.directories[index], name, AtFlags::SYMLINK_NOFOLLOW)
                .map_err(|_| ConnectorError::Storage)?;
            let file = self.directories[index + 1]
                .metadata()
                .map_err(|_| ConnectorError::Storage)?;
            if !file.is_dir()
                || FileType::from_raw_mode(entry.st_mode) != FileType::Directory
                || entry.st_dev as u64 != file.dev()
                || entry.st_ino != file.ino()
                || (index >= self.parts.len() - 2
                    && (file.uid() != rustix::process::geteuid().as_raw()
                        || file.mode() & 0o7777 != 0o700))
            {
                return Err(ConnectorError::Storage);
            }
        }
        Ok(())
    }
    fn verify_file(&self, name: &str, file: &File) -> Result<(), ConnectorError> {
        self.verify()?;
        let entry = statat(self.directory(), name, AtFlags::SYMLINK_NOFOLLOW)
            .map_err(|_| ConnectorError::Storage)?;
        let file = file.metadata().map_err(|_| ConnectorError::Storage)?;
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
            return Err(ConnectorError::Storage);
        }
        Ok(())
    }
    fn check_entry(&self, name: &str) -> Result<bool, ConnectorError> {
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
            _ => Err(ConnectorError::Storage),
        }
    }
    fn lock(&self) -> Result<StoreLock, ConnectorError> {
        // Keep trusted threads serialized too; the OS advisory lock also covers other processes.
        let guard = PROCESS_LOCK.lock().map_err(|_| ConnectorError::Storage)?;
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
            .map_err(|_| ConnectorError::Storage)?,
        );
        self.verify_file(LOCK, &file)?;
        if file.metadata().map_err(|_| ConnectorError::Storage)?.len() != 0 {
            return Err(ConnectorError::Storage);
        }
        // Descriptor-held lock serializes trusted writers across processes, not only threads.
        flock(&file, FlockOperation::LockExclusive).map_err(|_| ConnectorError::Storage)?;
        self.verify_file(LOCK, &file)?;
        self.directory()
            .sync_all()
            .map_err(|_| ConnectorError::Storage)?;
        Ok(StoreLock {
            file,
            _guard: guard,
        })
    }
    fn read(&self) -> Result<(AccountFile, Option<File>), ConnectorError> {
        if !self.check_entry(FILE)? {
            return Ok((
                AccountFile {
                    version: 1,
                    accounts: Vec::new(),
                },
                None,
            ));
        }
        let mut file = File::from(
            openat(
                self.directory(),
                FILE,
                OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|_| ConnectorError::Storage)?,
        );
        self.verify_file(FILE, &file)?;
        let length = file.metadata().map_err(|_| ConnectorError::Storage)?.len();
        if length == 0 || length > MAX_ACCOUNT_BYTES as u64 {
            return Err(ConnectorError::CorruptAccounts);
        }
        let mut bytes = Vec::new();
        (&mut file)
            .take(MAX_ACCOUNT_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| ConnectorError::Storage)?;
        self.verify_file(FILE, &file)?;
        if bytes.len() != length as usize || bytes.len() > MAX_ACCOUNT_BYTES {
            return Err(ConnectorError::CorruptAccounts);
        }
        let value: AccountFile =
            serde_json::from_slice(&bytes).map_err(|_| ConnectorError::CorruptAccounts)?;
        value.validate()?;
        Ok((value, Some(file)))
    }
    fn write(
        &self,
        document: &mut AccountFile,
        previous: Option<&File>,
        lock: &File,
    ) -> Result<(), ConnectorError> {
        self.write_checked(document, previous, lock, |_| Ok(()))
    }
    fn write_checked(
        &self,
        document: &mut AccountFile,
        previous: Option<&File>,
        lock: &File,
        checkpoint: impl FnMut(&str) -> Result<(), ConnectorError>,
    ) -> Result<(), ConnectorError> {
        let bytes = document.bytes()?;
        self.write_bytes_checked(FILE, &bytes, previous, lock, checkpoint)
    }
    fn write_bytes_checked(
        &self,
        filename: &str,
        bytes: &[u8],
        previous: Option<&File>,
        lock: &File,
        mut checkpoint: impl FnMut(&str) -> Result<(), ConnectorError>,
    ) -> Result<(), ConnectorError> {
        self.verify_file(LOCK, lock)?;
        let name = format!(".accounts-{}.tmp", uuid::Uuid::new_v4());
        let mut file = File::from(
            openat(
                self.directory(),
                name.as_str(),
                OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::from_raw_mode(0o600),
            )
            .map_err(|_| ConnectorError::Storage)?,
        );
        checkpoint("opened")?;
        self.verify_file(&name, &file)?;
        self.verify_file(LOCK, lock)?;
        file.write_all(&bytes)
            .map_err(|_| ConnectorError::Storage)?;
        file.sync_all().map_err(|_| ConnectorError::Storage)?;
        checkpoint("file-synced")?;
        self.verify_file(&name, &file)?;
        self.verify_file(LOCK, lock)?;
        match previous {
            Some(previous) => {
                self.verify_file(filename, previous)?;
                renameat(self.directory(), name.as_str(), self.directory(), filename)
                    .map_err(|_| ConnectorError::Storage)?;
            }
            None => renameat_with(
                self.directory(),
                name.as_str(),
                self.directory(),
                filename,
                RenameFlags::NOREPLACE,
            )
            .map_err(|_| ConnectorError::Storage)?,
        }
        // From this point metadata might be visible even if durability/identity verification fails.
        let mut finish = || -> Result<(), ConnectorError> {
            checkpoint("installed")?;
            self.verify_file(filename, &file)?;
            self.verify_file(LOCK, lock)?;
            self.directory()
                .sync_all()
                .map_err(|_| ConnectorError::Storage)?;
            checkpoint("parent-synced")?;
            self.verify_file(filename, &file)
        };
        finish().map_err(|_| ConnectorError::ReconciliationRequired)
        // Failed staging files stay private; never unlink a possibly replaced name on failure.
    }
    #[cfg(test)]
    pub(super) fn test_write(
        &self,
        document: &mut AccountFile,
        checkpoint: impl FnMut(&str) -> Result<(), ConnectorError>,
    ) -> Result<(), ConnectorError> {
        let lock = self.lock()?;
        let (_, previous) = self.read()?;
        self.write_checked(document, previous.as_ref(), &lock, checkpoint)
    }
}
impl super::assistant::DiskStore for AccountStore {
    fn client_config(
        &self,
    ) -> super::assistant::model::Result<Option<super::assistant::GoogleClientConfig>> {
        let _lock = self.lock().map_err(super::assistant::map_error)?;
        let Some((bytes, _)) = self
            .read_auxiliary("google-client.json")
            .map_err(super::assistant::map_error)?
        else {
            return Ok(None);
        };
        let config: super::assistant::GoogleClientConfig =
            serde_json::from_slice(&bytes).map_err(|_| "storage_failed")?;
        config.validate()?;
        Ok(Some(config))
    }
    fn save_client_config(
        &mut self,
        config: &super::assistant::GoogleClientConfig,
    ) -> super::assistant::model::Result<()> {
        config.validate()?;
        let lock = self.lock().map_err(super::assistant::map_error)?;
        let (accounts, _) = self.read().map_err(super::assistant::map_error)?;
        let previous = self
            .read_auxiliary("google-client.json")
            .map_err(super::assistant::map_error)?;
        if !accounts.accounts.is_empty() {
            let old: super::assistant::GoogleClientConfig =
                serde_json::from_slice(&previous.as_ref().ok_or("accounts_exist")?.0)
                    .map_err(|_| "storage_failed")?;
            if old.client_id != config.client_id {
                return Err("accounts_exist");
            }
        }
        let bytes = serde_json::to_vec(config).map_err(|_| "storage_failed")?;
        self.write_bytes_checked(
            "google-client.json",
            &bytes,
            previous.as_ref().map(|(_, file)| file),
            &lock,
            |_| Ok(()),
        )
        .map_err(super::assistant::map_error)
    }
    fn audit(
        &self,
        event: &super::assistant::mutations::AuditEvent,
    ) -> super::assistant::model::Result<()> {
        let lock = self.lock().map_err(super::assistant::map_error)?;
        let name = "google-audit.jsonl";
        self.check_entry(name)
            .map_err(super::assistant::map_error)?;
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
        self.verify_file(LOCK, &lock).map_err(|_| "audit_failed")?;
        self.directory().sync_all().map_err(|_| "audit_failed")?;
        self.verify_file(name, &file).map_err(|_| "audit_failed")
    }
}
impl AccountRepository for AccountStore {
    fn list(&self) -> Result<Vec<Account>, ConnectorError> {
        let _lock = self.lock()?;
        let (mut document, _) = self.read()?;
        document.accounts.sort_by_key(Account::id);
        Ok(document.accounts)
    }
    fn get(&self, id: AccountId) -> Result<Option<Account>, ConnectorError> {
        Ok(self.list()?.into_iter().find(|account| account.id() == id))
    }
    fn add(&mut self, account: Account) -> Result<(), ConnectorError> {
        let lock = self.lock()?;
        let (mut document, previous) = self.read()?;
        if document
            .accounts
            .iter()
            .any(|value| value.id() == account.id())
        {
            return Err(ConnectorError::DuplicateAccount);
        }
        if document.accounts.len() == MAX_ACCOUNTS {
            return Err(ConnectorError::AccountLimit);
        }
        document.accounts.push(account);
        self.write(&mut document, previous.as_ref(), &lock)
    }
    fn update(&mut self, account: Account) -> Result<(), ConnectorError> {
        let lock = self.lock()?;
        let (mut document, previous) = self.read()?;
        let current = document
            .accounts
            .iter_mut()
            .find(|value| value.id() == account.id())
            .ok_or(ConnectorError::AccountMissing)?;
        if !current.same_identity(&account) {
            return Err(ConnectorError::InvalidAccount);
        }
        *current = account;
        self.write(&mut document, previous.as_ref(), &lock)
    }
    fn remove(&mut self, id: AccountId) -> Result<(), ConnectorError> {
        let lock = self.lock()?;
        let (mut document, previous) = self.read()?;
        let index = document
            .accounts
            .iter()
            .position(|value| value.id() == id)
            .ok_or(ConnectorError::AccountMissing)?;
        document.accounts.remove(index);
        self.write(&mut document, previous.as_ref(), &lock)
    }
}
