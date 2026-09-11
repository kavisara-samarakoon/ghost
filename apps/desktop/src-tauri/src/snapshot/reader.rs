//! Descriptor-relative reads keep directory/file swaps from redirecting metadata reads.
use super::names;
use std::path::{Component, Path};

pub const MAX_FILE_BYTES: usize = 256 * 1024;
const MAX_SNAPSHOT_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_DIRECTORY_ENTRIES: usize = 512;
const MAX_SNAPSHOT_ENTRIES: usize = 4096;

#[derive(Clone, Copy)]
enum Scope {
    Root,
    Workspace,
    Sessions,
    Session,
    Outputs,
    OutputType,
    Drafts,
    Handoffs,
    Markdown,
    UpdatePacks,
    UpdatePack,
}

impl Scope {
    fn child(self, name: &str) -> Option<Self> {
        match (self, name) {
            (Self::Root, ".ghost") => Some(Self::Workspace),
            (Self::Workspace, "sessions") => Some(Self::Sessions),
            (Self::Workspace, "outputs") => Some(Self::Outputs),
            (Self::Workspace, "drafts") => Some(Self::Drafts),
            (Self::Sessions, id) if names::session_id(id) => Some(Self::Session),
            (Self::Outputs, "codex" | "terminal") => Some(Self::OutputType),
            (Self::Drafts, "context-packs" | "next-steps") => Some(Self::Markdown),
            (Self::Drafts, "handoffs") => Some(Self::Handoffs),
            (Self::Drafts, "update-packs") => Some(Self::UpdatePacks),
            (Self::Handoffs, "codex" | "chatgpt" | "gemini" | "antigravity") => {
                Some(Self::Markdown)
            }
            (Self::UpdatePacks, name) if names::safe_segment(name) => Some(Self::UpdatePack),
            _ => None,
        }
    }

    fn allows_file(self, name: &str) -> bool {
        match self {
            Self::Root => name == "projects.yaml",
            Self::Workspace => matches!(name, "project.yaml" | "status.md" | "active-session.yaml"),
            Self::Session => matches!(name, "session.yaml" | "notes.md"),
            Self::Outputs => name == "index.yaml",
            Self::OutputType | Self::Markdown | Self::UpdatePack => names::markdown(name),
            _ => false,
        }
    }
}

pub struct Listing {
    pub names: Vec<String>,
    pub skipped: bool,
}

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

pub struct ReadBudget {
    bytes: usize,
    entries: usize,
}

impl Default for ReadBudget {
    fn default() -> Self {
        Self {
            bytes: MAX_SNAPSHOT_BYTES,
            entries: MAX_SNAPSHOT_ENTRIES,
        }
    }
}

#[cfg(unix)]
mod platform {
    use super::*;
    use rustix::fs::{open, openat, statat, AtFlags, Dir, FileType, Mode, OFlags};
    use std::fs::File;
    use std::io::{ErrorKind, Read};
    use std::os::unix::fs::MetadataExt;

    pub struct Directory(File, Scope);

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
            let mut directory = Self(File::from(root), Scope::Root);
            for component in path.components() {
                if let Component::Normal(name) = component {
                    let file = optional_file(openat(
                        &directory.0,
                        name,
                        directory_flags(),
                        Mode::empty(),
                    ))?;
                    match file {
                        Some(file) => directory = Self(file, Scope::Root),
                        None => return Ok(None),
                    }
                }
            }
            Ok(Some(directory))
        }

        pub fn child(&self, name: &str) -> Result<Option<Self>, &'static str> {
            let scope = self
                .1
                .child(name)
                .ok_or("Directory is outside the metadata allowlist.")?;
            optional_file(openat(&self.0, name, directory_flags(), Mode::empty()))
                .map(|file| file.map(|file| Self(file, scope)))
        }

        pub fn list(&self, budget: &mut ReadBudget) -> Result<Listing, &'static str> {
            if !matches!(
                self.1,
                Scope::Markdown | Scope::UpdatePacks | Scope::UpdatePack
            ) {
                return Err("Directory scanning is outside the metadata allowlist.");
            }
            let directories = matches!(self.1, Scope::UpdatePacks);
            let mut entries =
                Dir::read_from(&self.0).map_err(|_| "Artifact directory is inaccessible.")?;
            let mut listing = Listing {
                names: Vec::new(),
                skipped: false,
            };
            let mut visited = 0;
            while let Some(entry) = entries.read() {
                let entry = entry.map_err(|_| "Artifact directory could not be listed.")?;
                let bytes = entry.file_name().to_bytes();
                if bytes == b"." || bytes == b".." {
                    continue;
                }
                if visited == MAX_DIRECTORY_ENTRIES || budget.entries == 0 {
                    return Err(
                        "Directory entry limit reached; recent artifacts and count unavailable.",
                    );
                }
                visited += 1;
                budget.entries -= 1;
                let Ok(name) = std::str::from_utf8(bytes) else {
                    listing.skipped = true;
                    continue;
                };
                let allowed = if directories {
                    names::safe_segment(name)
                } else {
                    names::markdown(name)
                };
                if !allowed {
                    listing.skipped = true;
                    continue;
                }
                let Ok(metadata) = statat(&self.0, name, AtFlags::SYMLINK_NOFOLLOW) else {
                    listing.skipped = true;
                    continue;
                };
                let kind = FileType::from_raw_mode(metadata.st_mode);
                if (directories && kind == FileType::Directory)
                    || (!directories && kind == FileType::RegularFile && metadata.st_nlink == 1)
                {
                    listing.names.push(name.to_owned());
                } else {
                    listing.skipped = true;
                }
            }
            Ok(listing)
        }

        pub fn read(
            &self,
            name: &str,
            budget: &mut ReadBudget,
        ) -> Result<Option<String>, &'static str> {
            if !self.1.allows_file(name) {
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
            if budget.bytes == 0 || metadata.len() > budget.bytes as u64 {
                return Err("Snapshot read limit reached.");
            }
            let limit = MAX_FILE_BYTES.min(budget.bytes);
            let mut bytes = Vec::new();
            let read_result = file.take((limit + 1) as u64).read_to_end(&mut bytes);
            budget.bytes = budget.bytes.saturating_sub(bytes.len());
            read_result.map_err(|_| "Metadata could not be read.")?;
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
        pub fn list(&self, _: &mut ReadBudget) -> Result<Listing, &'static str> {
            Err("Safe local snapshots are unavailable on this platform.")
        }
    }
}

pub use platform::Directory;
