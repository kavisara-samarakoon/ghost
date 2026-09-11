//! Descriptor-relative reads keep directory/file swaps from redirecting metadata reads.
use std::path::{Component, Path};

pub const MAX_FILE_BYTES: usize = 256 * 1024;
const MAX_SNAPSHOT_BYTES: usize = 4 * 1024 * 1024;

pub fn validate_path(path: &Path) -> Result<(), &'static str> {
    if !path.is_absolute() || path.to_string_lossy().starts_with("//") {
        return Err("Expected an absolute local path.");
    }
    for component in path.components() {
        match component {
            Component::RootDir => {}
            Component::Normal(name) => {
                let name = name.to_str().ok_or("Path is not valid UTF-8.")?;
                if name.to_ascii_lowercase().starts_with(".env") {
                    return Err("Environment paths are excluded.");
                }
            }
            _ => return Err("Relative or parent path components are excluded."),
        }
    }
    Ok(())
}

pub struct ReadBudget(usize);

impl Default for ReadBudget {
    fn default() -> Self {
        Self(MAX_SNAPSHOT_BYTES)
    }
}

#[cfg(unix)]
mod platform {
    use super::*;
    use rustix::fs::{open, openat, Mode, OFlags};
    use std::fs::File;
    use std::io::{ErrorKind, Read};
    use std::os::unix::fs::MetadataExt;

    pub struct Directory(File);

    fn directory_flags() -> OFlags {
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC
    }

    fn optional_file(
        result: rustix::io::Result<rustix::fd::OwnedFd>,
    ) -> Result<Option<File>, &'static str> {
        match result {
            Ok(fd) => Ok(Some(File::from(fd))),
            Err(error) if std::io::Error::from(error).kind() == ErrorKind::NotFound => Ok(None),
            Err(_) => Err("Missing, inaccessible, or unsafe metadata path."),
        }
    }

    impl Directory {
        pub fn open(path: &Path) -> Result<Option<Self>, &'static str> {
            validate_path(path)?;
            let root = open("/", directory_flags(), Mode::empty())
                .map_err(|_| "Local filesystem is inaccessible.")?;
            let mut directory = Self(File::from(root));
            for component in path.components() {
                if let Component::Normal(name) = component {
                    let file = optional_file(openat(
                        &directory.0,
                        name,
                        directory_flags(),
                        Mode::empty(),
                    ))?;
                    match file {
                        Some(file) => directory = Self(file),
                        None => return Ok(None),
                    }
                }
            }
            Ok(Some(directory))
        }

        pub fn child(&self, name: &str) -> Result<Option<Self>, &'static str> {
            // Only GHOST-owned directories are ever opened under registered roots.
            if !matches!(name, ".ghost" | "outputs") {
                return Err("Directory is outside the metadata allowlist.");
            }
            optional_file(openat(&self.0, name, directory_flags(), Mode::empty()))
                .map(|file| file.map(Self))
        }

        pub fn read(
            &self,
            name: &str,
            budget: &mut ReadBudget,
        ) -> Result<Option<String>, &'static str> {
            if !matches!(
                name,
                "projects.yaml"
                    | "project.yaml"
                    | "status.md"
                    | "active-session.yaml"
                    | "index.yaml"
            ) {
                return Err("File is outside the metadata allowlist.");
            }
            let flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC;
            let Some(file) = optional_file(openat(&self.0, name, flags, Mode::empty()))? else {
                return Ok(None);
            };
            let metadata = file
                .metadata()
                .map_err(|_| "Metadata file is inaccessible.")?;
            if !metadata.is_file() || metadata.nlink() != 1 {
                return Err("Metadata must be a regular file with no hard links.");
            }
            if metadata.len() > MAX_FILE_BYTES as u64 {
                return Err("Metadata exceeds the 256 KiB read limit.");
            }
            if budget.0 == 0 || metadata.len() > budget.0 as u64 {
                return Err("Snapshot read limit reached.");
            }
            let limit = MAX_FILE_BYTES.min(budget.0);
            let mut bytes = Vec::new();
            file.take((limit + 1) as u64)
                .read_to_end(&mut bytes)
                .map_err(|_| "Metadata could not be read.")?;
            budget.0 = budget.0.saturating_sub(bytes.len());
            if bytes.len() > limit {
                return Err("Metadata grew beyond the read limit.");
            }
            String::from_utf8(bytes)
                .map(Some)
                .map_err(|_| "Metadata is not valid UTF-8.")
        }
    }
}

// Fail closed until an equivalent handle-based reader exists on other platforms.
#[cfg(not(unix))]
mod platform {
    use super::*;
    pub struct Directory;
    impl Directory {
        pub fn open(_: &Path) -> Result<Option<Self>, &'static str> {
            Err("Safe local snapshots are currently supported on macOS and Linux only.")
        }
        pub fn child(&self, _: &str) -> Result<Option<Self>, &'static str> {
            Self::open(Path::new(""))
        }
        pub fn read(&self, _: &str, _: &mut ReadBudget) -> Result<Option<String>, &'static str> {
            Err("Safe local snapshots are unavailable on this platform.")
        }
    }
}

pub use platform::Directory;
