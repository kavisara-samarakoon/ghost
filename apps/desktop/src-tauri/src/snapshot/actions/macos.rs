//! Native AppKit dispatch, with an identity-bound file URL instead of a process launcher.
use super::{Action, ActionState, Target};
use objc2_app_kit::NSWorkspace;
use objc2_foundation::{NSArray, NSNumber, NSString, NSURLIsAliasFileKey, NSURL};
use std::os::unix::fs::MetadataExt;

impl Target {
    pub(in crate::snapshot) fn reference_url(
        &self,
    ) -> Result<objc2::rc::Retained<NSURL>, &'static str> {
        self.revalidate()?;
        // macOS's volfs path addresses the held file by device and inode. Build the
        // URL from that identity, so a later path swap cannot select another file.
        // This kernel-generated path is never accepted from IPC or used to read data.
        let metadata = self.file.metadata().map_err(|_| "Artifact unavailable.")?;
        let path = NSString::from_str(&format!("/.vol/{}/{}", metadata.dev(), metadata.ino()));
        let reference = NSURL::fileURLWithPath(&path)
            .fileReferenceURL()
            .ok_or("File reference unavailable.")?;
        if !reference.isFileReferenceURL() {
            return Err("File reference unavailable.");
        }

        let current_path = reference
            .filePathURL()
            .and_then(|url| url.path())
            .ok_or("File reference unavailable.")?
            .to_string();
        if std::path::Path::new(&current_path) != self.path {
            return Err("Artifact changed; action rejected.");
        }
        // Finder aliases are regular files too. They must never redirect an open action.
        // SAFETY: this is an immutable Foundation resource-key constant.
        let key = unsafe { NSURLIsAliasFileKey };
        let values = reference
            .resourceValuesForKeys_error(&NSArray::from_slice(&[key]))
            .map_err(|_| "Artifact type unavailable.")?;
        let alias = values
            .objectForKey(key)
            .ok_or("Artifact type unavailable.")?;
        if alias
            .downcast_ref::<NSNumber>()
            .is_none_or(|value| value.boolValue())
        {
            return Err("File aliases are excluded.");
        }
        self.revalidate()?;
        Ok(reference)
    }
}

pub(super) fn dispatch(target: &Target, action: Action) -> ActionState {
    let Ok(url) = target.reference_url() else {
        return ActionState::Rejected;
    };
    let workspace = NSWorkspace::sharedWorkspace();
    match action {
        Action::Open => {
            if workspace.openURL(&url) {
                ActionState::Opened
            } else {
                ActionState::Unavailable
            }
        }
        Action::Reveal => {
            workspace.activateFileViewerSelectingURLs(&NSArray::from_retained_slice(&[url]));
            ActionState::Revealed
        }
    }
}
