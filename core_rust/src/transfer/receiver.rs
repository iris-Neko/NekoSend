use std::{
    fs::{self, OpenOptions},
    path::{Path, PathBuf},
};

use thiserror::Error;

use crate::{
    domain::{TransferEntryKind, TransferId},
    storage::TransferEntryRecord,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedReceiveEntry {
    pub entry_id: crate::domain::EntryId,
    pub destination_ref: String,
    pub partial_ref: Option<String>,
    pub persisted_offset: u64,
}

#[derive(Debug, Error)]
pub enum ReceivePathError {
    #[error("receive directory is unavailable: {0}")]
    BaseUnavailable(String),
    #[error("receive path cannot be represented as UTF-8: {0}")]
    NonUtf8(String),
    #[error("transfer manifest does not have a valid root")]
    InvalidManifest,
    #[error("failed to prepare receive path: {0}")]
    Io(#[source] std::io::Error),
}

pub fn prepare_receive_paths(
    receive_base: impl AsRef<Path>,
    transfer_id: TransferId,
    display_name: &str,
    entries: &[TransferEntryRecord],
) -> Result<Vec<PreparedReceiveEntry>, ReceivePathError> {
    if entries.is_empty() || entries[0].relative_path.split('/').next() != Some(display_name) {
        return Err(ReceivePathError::InvalidManifest);
    }
    let receive_base = receive_base.as_ref();
    fs::create_dir_all(receive_base).map_err(ReceivePathError::Io)?;
    if !receive_base.is_dir() {
        return Err(ReceivePathError::BaseUnavailable(
            receive_base.display().to_string(),
        ));
    }
    let folder = entries[0].entry_kind == TransferEntryKind::Directory;
    let root = unique_destination(receive_base, display_name);
    if folder {
        fs::create_dir(&root).map_err(ReceivePathError::Io)?;
    }

    let mut prepared = Vec::with_capacity(entries.len());
    for entry in entries {
        let destination = if folder {
            let remainder = entry
                .relative_path
                .strip_prefix(display_name)
                .and_then(|path| path.strip_prefix('/').or(Some(path)))
                .ok_or(ReceivePathError::InvalidManifest)?;
            if remainder.is_empty() {
                root.clone()
            } else {
                join_protocol_path(&root, remainder)
            }
        } else {
            root.clone()
        };
        match entry.entry_kind {
            TransferEntryKind::Directory => {
                fs::create_dir_all(&destination).map_err(ReceivePathError::Io)?;
                prepared.push(PreparedReceiveEntry {
                    entry_id: entry.entry_id,
                    destination_ref: path_string(&destination)?,
                    partial_ref: None,
                    persisted_offset: 0,
                });
            }
            TransferEntryKind::File => {
                let parent = destination
                    .parent()
                    .ok_or(ReceivePathError::InvalidManifest)?;
                fs::create_dir_all(parent).map_err(ReceivePathError::Io)?;
                let file_name = destination
                    .file_name()
                    .and_then(|name| name.to_str())
                    .ok_or_else(|| ReceivePathError::NonUtf8(destination.display().to_string()))?;
                let partial = parent.join(format!(".{file_name}.{transfer_id}.partial"));
                let file = OpenOptions::new()
                    .create(true)
                    .write(true)
                    .truncate(false)
                    .open(&partial)
                    .map_err(ReceivePathError::Io)?;
                let length = file.metadata().map_err(ReceivePathError::Io)?.len();
                let offset = length.min(entry.size);
                if length > offset {
                    file.set_len(offset).map_err(ReceivePathError::Io)?;
                }
                prepared.push(PreparedReceiveEntry {
                    entry_id: entry.entry_id,
                    destination_ref: path_string(&destination)?,
                    partial_ref: Some(path_string(&partial)?),
                    persisted_offset: offset,
                });
            }
        }
    }
    Ok(prepared)
}

fn unique_destination(base: &Path, display_name: &str) -> PathBuf {
    let initial = base.join(display_name);
    if !initial.exists() {
        return initial;
    }
    let path = Path::new(display_name);
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or(display_name);
    let extension = path.extension().and_then(|value| value.to_str());
    for sequence in 1_u64.. {
        let name = match extension {
            Some(extension) => format!("{stem} ({sequence}).{extension}"),
            None => format!("{stem} ({sequence})"),
        };
        let candidate = base.join(name);
        if !candidate.exists() {
            return candidate;
        }
    }
    unreachable!()
}

fn join_protocol_path(base: &Path, relative: &str) -> PathBuf {
    relative
        .split('/')
        .fold(base.to_path_buf(), |path, segment| path.join(segment))
}

fn path_string(path: &Path) -> Result<String, ReceivePathError> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| ReceivePathError::NonUtf8(path.display().to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{EntryId, TransferId};

    fn entry(
        transfer_id: TransferId,
        kind: TransferEntryKind,
        path: &str,
        size: u64,
    ) -> TransferEntryRecord {
        TransferEntryRecord {
            entry_id: EntryId::generate(),
            transfer_id,
            entry_kind: kind,
            relative_path: path.to_owned(),
            size,
            modified_at_ms: 0,
            source_ref: None,
            destination_ref: None,
            partial_ref: None,
            persisted_offset: 0,
            state: "queued".to_owned(),
        }
    }

    #[test]
    fn prepares_empty_directories_partials_and_non_overwriting_root() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir(temp.path().join("photos")).unwrap();
        let transfer_id = TransferId::generate();
        let entries = vec![
            entry(transfer_id, TransferEntryKind::Directory, "photos", 0),
            entry(transfer_id, TransferEntryKind::Directory, "photos/empty", 0),
            entry(transfer_id, TransferEntryKind::File, "photos/a.bin", 10),
        ];
        let prepared = prepare_receive_paths(temp.path(), transfer_id, "photos", &entries).unwrap();
        assert!(temp.path().join("photos (1)/empty").is_dir());
        assert_eq!(
            Path::new(&prepared[2].destination_ref),
            temp.path().join("photos (1)/a.bin")
        );
        assert!(Path::new(prepared[2].partial_ref.as_ref().unwrap()).is_file());
    }

    #[test]
    fn single_file_uses_incremented_name_without_overwrite() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("report.pdf"), b"old").unwrap();
        let transfer_id = TransferId::generate();
        let entries = vec![entry(transfer_id, TransferEntryKind::File, "report.pdf", 5)];
        let prepared =
            prepare_receive_paths(temp.path(), transfer_id, "report.pdf", &entries).unwrap();
        assert_eq!(
            Path::new(&prepared[0].destination_ref),
            temp.path().join("report (1).pdf")
        );
        assert_eq!(fs::read(temp.path().join("report.pdf")).unwrap(), b"old");
    }
}
