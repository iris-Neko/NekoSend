use std::{
    collections::{HashSet, VecDeque},
    fs,
    path::Path,
    time::UNIX_EPOCH,
};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::domain::{EntryId, MessageKind, TransferEntryKind};

pub const MAX_FILE_ENTRIES: usize = 10_000;
pub const MAX_MANIFEST_ENTRIES: usize = 20_001;
pub const MAX_MANIFEST_JSON_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_RELATIVE_PATH_BYTES: usize = 1_024;
const MAX_PATH_DEPTH: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceItem {
    pub entry_kind: TransferEntryKind,
    pub source_ref: Option<String>,
    pub relative_path: String,
    pub size: u64,
    pub modified_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestEntry {
    pub entry_id: EntryId,
    pub entry_kind: TransferEntryKind,
    pub relative_path: String,
    pub size: u64,
    pub modified_at_ms: i64,
    #[serde(skip_serializing, default)]
    pub source_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransferManifest {
    pub message_kind: MessageKind,
    pub display_name: String,
    pub total_size: u64,
    pub entry_count: u32,
    pub entries: Vec<ManifestEntry>,
}

#[derive(Debug, Error)]
pub enum ManifestError {
    #[error("message kind must be file, image, folder, or clipboard_image")]
    InvalidMessageKind,
    #[error("source does not exist or cannot be read: {0}")]
    SourceUnavailable(String),
    #[error("symbolic links are not supported: {0}")]
    SymbolicLink(String),
    #[error("source metadata exceeds the supported 64-bit database range")]
    SizeOverflow,
    #[error(
        "manifest must contain at most {MAX_FILE_ENTRIES} files and {MAX_MANIFEST_ENTRIES} total entries"
    )]
    EntryLimit,
    #[error("manifest JSON exceeds {MAX_MANIFEST_JSON_BYTES} bytes")]
    JsonLimit,
    #[error("invalid relative path: {0}")]
    InvalidPath(String),
    #[error("manifest entry order or parent directories are invalid: {0}")]
    InvalidHierarchy(String),
    #[error("manifest totals do not match its entries")]
    InvalidTotals,
    #[error("file entry requires a source reference")]
    MissingSource,
}

pub fn enumerate_path(
    source: impl AsRef<Path>,
    message_kind: MessageKind,
) -> Result<TransferManifest, ManifestError> {
    let source = source.as_ref();
    let metadata = fs::symlink_metadata(source)
        .map_err(|error| ManifestError::SourceUnavailable(error.to_string()))?;
    if metadata.file_type().is_symlink() {
        return Err(ManifestError::SymbolicLink(source.display().to_string()));
    }
    let display_name = source
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| ManifestError::InvalidPath(source.display().to_string()))?
        .to_owned();

    match message_kind {
        MessageKind::File | MessageKind::Image | MessageKind::ClipboardImage => {
            if !metadata.is_file() {
                return Err(ManifestError::InvalidMessageKind);
            }
            build_manifest(
                message_kind,
                display_name.clone(),
                vec![SourceItem {
                    entry_kind: TransferEntryKind::File,
                    source_ref: Some(path_to_source_ref(source)?),
                    relative_path: display_name,
                    size: metadata.len(),
                    modified_at_ms: modified_at_ms(&metadata),
                }],
            )
        }
        MessageKind::Folder => {
            if !metadata.is_dir() {
                return Err(ManifestError::InvalidMessageKind);
            }
            let mut sources = vec![SourceItem {
                entry_kind: TransferEntryKind::Directory,
                source_ref: Some(path_to_source_ref(source)?),
                relative_path: display_name.clone(),
                size: 0,
                modified_at_ms: modified_at_ms(&metadata),
            }];
            let mut file_count = 0_usize;
            let mut directories = VecDeque::from([(source.to_path_buf(), display_name.clone())]);
            while let Some((directory, relative_directory)) = directories.pop_front() {
                let mut children = fs::read_dir(&directory)
                    .map_err(|error| ManifestError::SourceUnavailable(error.to_string()))?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|error| ManifestError::SourceUnavailable(error.to_string()))?;
                children.sort_by_key(|entry| entry.file_name());
                for child in children {
                    if sources.len() >= MAX_MANIFEST_ENTRIES {
                        return Err(ManifestError::EntryLimit);
                    }
                    let child_path = child.path();
                    let child_metadata = fs::symlink_metadata(&child_path)
                        .map_err(|error| ManifestError::SourceUnavailable(error.to_string()))?;
                    if child_metadata.file_type().is_symlink() {
                        continue;
                    }
                    let child_name = child
                        .file_name()
                        .to_str()
                        .ok_or_else(|| {
                            ManifestError::InvalidPath(child_path.display().to_string())
                        })?
                        .to_owned();
                    let relative_path = format!("{relative_directory}/{child_name}");
                    if child_metadata.is_dir() {
                        sources.push(SourceItem {
                            entry_kind: TransferEntryKind::Directory,
                            source_ref: None,
                            relative_path: relative_path.clone(),
                            size: 0,
                            modified_at_ms: modified_at_ms(&child_metadata),
                        });
                        directories.push_back((child_path, relative_path));
                    } else if child_metadata.is_file() {
                        if file_count >= MAX_FILE_ENTRIES {
                            return Err(ManifestError::EntryLimit);
                        }
                        file_count += 1;
                        sources.push(SourceItem {
                            entry_kind: TransferEntryKind::File,
                            source_ref: Some(path_to_source_ref(&child_path)?),
                            relative_path,
                            size: child_metadata.len(),
                            modified_at_ms: modified_at_ms(&child_metadata),
                        });
                    }
                }
            }
            build_manifest(message_kind, display_name, sources)
        }
        _ => Err(ManifestError::InvalidMessageKind),
    }
}

pub fn build_manifest(
    message_kind: MessageKind,
    display_name: String,
    sources: Vec<SourceItem>,
) -> Result<TransferManifest, ManifestError> {
    if !matches!(
        message_kind,
        MessageKind::File | MessageKind::Image | MessageKind::Folder | MessageKind::ClipboardImage
    ) {
        return Err(ManifestError::InvalidMessageKind);
    }
    validate_path(&display_name)?;
    if display_name.contains('/') {
        return Err(ManifestError::InvalidPath(display_name));
    }
    if sources.is_empty()
        || sources.len() > MAX_MANIFEST_ENTRIES
        || sources
            .iter()
            .filter(|source| source.entry_kind == TransferEntryKind::File)
            .count()
            > MAX_FILE_ENTRIES
    {
        return Err(ManifestError::EntryLimit);
    }

    let mut seen = HashSet::with_capacity(sources.len());
    let mut total_size = 0_u64;
    let mut entries = Vec::with_capacity(sources.len());
    for source in sources {
        let segments = validate_path(&source.relative_path)?;
        if !seen.insert(source.relative_path.clone()) {
            return Err(ManifestError::InvalidHierarchy(source.relative_path));
        }
        if segments[0] != display_name {
            return Err(ManifestError::InvalidHierarchy(source.relative_path));
        }
        if let Some((parent, _)) = source.relative_path.rsplit_once('/')
            && !seen.contains(parent)
        {
            return Err(ManifestError::InvalidHierarchy(source.relative_path));
        }
        match source.entry_kind {
            TransferEntryKind::Directory => {
                if source.size != 0 {
                    return Err(ManifestError::InvalidTotals);
                }
            }
            TransferEntryKind::File => {
                if source.source_ref.as_deref().is_none_or(str::is_empty) {
                    return Err(ManifestError::MissingSource);
                }
                total_size = total_size
                    .checked_add(source.size)
                    .ok_or(ManifestError::SizeOverflow)?;
                if total_size > i64::MAX as u64 || source.size > i64::MAX as u64 {
                    return Err(ManifestError::SizeOverflow);
                }
            }
        }
        entries.push(ManifestEntry {
            entry_id: EntryId::generate(),
            entry_kind: source.entry_kind,
            relative_path: source.relative_path,
            size: source.size,
            modified_at_ms: source.modified_at_ms,
            source_ref: source.source_ref,
        });
    }

    match message_kind {
        MessageKind::Folder => {
            let first = entries.first().ok_or(ManifestError::EntryLimit)?;
            if first.entry_kind != TransferEntryKind::Directory
                || first.relative_path != display_name
            {
                return Err(ManifestError::InvalidHierarchy(display_name));
            }
        }
        _ => {
            if entries.len() != 1 || entries[0].entry_kind != TransferEntryKind::File {
                return Err(ManifestError::InvalidHierarchy(display_name));
            }
        }
    }

    let manifest = TransferManifest {
        message_kind,
        display_name,
        total_size,
        entry_count: u32::try_from(entries.len()).map_err(|_| ManifestError::EntryLimit)?,
        entries,
    };
    validate_manifest(&manifest)?;
    Ok(manifest)
}

pub fn validate_manifest(manifest: &TransferManifest) -> Result<(), ManifestError> {
    let sources = manifest
        .entries
        .iter()
        .map(|entry| SourceItem {
            entry_kind: entry.entry_kind,
            source_ref: entry
                .source_ref
                .clone()
                .or_else(|| (entry.entry_kind == TransferEntryKind::File).then(|| "remote".into())),
            relative_path: entry.relative_path.clone(),
            size: entry.size,
            modified_at_ms: entry.modified_at_ms,
        })
        .collect::<Vec<_>>();
    let rebuilt =
        build_manifest_without_ids(manifest.message_kind, &manifest.display_name, &sources)?;
    if manifest.entry_count as usize != manifest.entries.len()
        || manifest.total_size != rebuilt
        || manifest
            .entries
            .iter()
            .map(|entry| entry.entry_id)
            .collect::<HashSet<_>>()
            .len()
            != manifest.entries.len()
    {
        return Err(ManifestError::InvalidTotals);
    }
    let size = serde_json::to_vec(manifest)
        .map_err(|_| ManifestError::JsonLimit)?
        .len();
    if size > MAX_MANIFEST_JSON_BYTES {
        return Err(ManifestError::JsonLimit);
    }
    Ok(())
}

fn build_manifest_without_ids(
    message_kind: MessageKind,
    display_name: &str,
    sources: &[SourceItem],
) -> Result<u64, ManifestError> {
    if !matches!(
        message_kind,
        MessageKind::File | MessageKind::Image | MessageKind::Folder | MessageKind::ClipboardImage
    ) {
        return Err(ManifestError::InvalidMessageKind);
    }
    validate_path(display_name)?;
    if display_name.contains('/')
        || sources.is_empty()
        || sources.len() > MAX_MANIFEST_ENTRIES
        || sources
            .iter()
            .filter(|source| source.entry_kind == TransferEntryKind::File)
            .count()
            > MAX_FILE_ENTRIES
    {
        return Err(ManifestError::EntryLimit);
    }
    let mut seen = HashSet::with_capacity(sources.len());
    let mut total = 0_u64;
    for (index, source) in sources.iter().enumerate() {
        let segments = validate_path(&source.relative_path)?;
        if segments[0] != display_name || !seen.insert(source.relative_path.clone()) {
            return Err(ManifestError::InvalidHierarchy(
                source.relative_path.clone(),
            ));
        }
        if let Some((parent, _)) = source.relative_path.rsplit_once('/')
            && !seen.contains(parent)
        {
            return Err(ManifestError::InvalidHierarchy(
                source.relative_path.clone(),
            ));
        }
        match source.entry_kind {
            TransferEntryKind::Directory if source.size == 0 => {}
            TransferEntryKind::File => {
                total = total
                    .checked_add(source.size)
                    .ok_or(ManifestError::SizeOverflow)?;
                if total > i64::MAX as u64 || source.size > i64::MAX as u64 {
                    return Err(ManifestError::SizeOverflow);
                }
            }
            _ => return Err(ManifestError::InvalidTotals),
        }
        if message_kind == MessageKind::Folder {
            if index == 0
                && (source.entry_kind != TransferEntryKind::Directory
                    || source.relative_path != display_name)
            {
                return Err(ManifestError::InvalidHierarchy(
                    source.relative_path.clone(),
                ));
            }
        } else if sources.len() != 1 || source.entry_kind != TransferEntryKind::File {
            return Err(ManifestError::InvalidHierarchy(
                source.relative_path.clone(),
            ));
        }
    }
    Ok(total)
}

fn validate_path(path: &str) -> Result<Vec<&str>, ManifestError> {
    if path.is_empty()
        || path.len() > MAX_RELATIVE_PATH_BYTES
        || path.starts_with('/')
        || path.starts_with('\\')
        || path.contains('\0')
        || path.contains('\\')
    {
        return Err(ManifestError::InvalidPath(path.to_owned()));
    }
    let segments = path.split('/').collect::<Vec<_>>();
    if segments.len() > MAX_PATH_DEPTH
        || segments.iter().any(|segment| !valid_path_segment(segment))
    {
        return Err(ManifestError::InvalidPath(path.to_owned()));
    }
    Ok(segments)
}

fn valid_path_segment(segment: &str) -> bool {
    if segment.is_empty()
        || segment == "."
        || segment == ".."
        || segment.ends_with(['.', ' '])
        || segment.bytes().any(|byte| byte < 0x20)
        || segment
            .chars()
            .any(|character| "<>:\"|?*".contains(character))
    {
        return false;
    }
    let stem = segment
        .split('.')
        .next()
        .unwrap_or(segment)
        .to_ascii_uppercase();
    !matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        && !(stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && matches!(stem.as_bytes()[3], b'1'..=b'9'))
}

fn path_to_source_ref(path: &Path) -> Result<String, ManifestError> {
    path.canonicalize()
        .map_err(|error| ManifestError::SourceUnavailable(error.to_string()))?
        .to_str()
        .map(str::to_owned)
        .ok_or_else(|| ManifestError::InvalidPath(path.display().to_string()))
}

fn modified_at_ms(metadata: &fs::Metadata) -> i64 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, size: u64) -> SourceItem {
        SourceItem {
            entry_kind: TransferEntryKind::File,
            source_ref: Some(format!("source:{path}")),
            relative_path: path.to_owned(),
            size,
            modified_at_ms: 123,
        }
    }

    fn directory(path: &str) -> SourceItem {
        SourceItem {
            entry_kind: TransferEntryKind::Directory,
            source_ref: None,
            relative_path: path.to_owned(),
            size: 0,
            modified_at_ms: 123,
        }
    }

    #[test]
    fn folder_manifest_preserves_empty_directories_and_checked_total() {
        let manifest = build_manifest(
            MessageKind::Folder,
            "相册".to_owned(),
            vec![
                directory("相册"),
                directory("相册/空目录"),
                directory("相册/2026"),
                file("相册/2026/a.jpg", 7),
            ],
        )
        .unwrap();
        assert_eq!(manifest.entry_count, 4);
        assert_eq!(manifest.total_size, 7);
        assert!(validate_manifest(&manifest).is_ok());
    }

    #[test]
    fn rejects_traversal_absolute_drives_reserved_names_and_missing_parents() {
        for path in [
            "../a.txt",
            "/a.txt",
            "C:/a.txt",
            "root\\a.txt",
            "CON.txt",
            "root/LPT1.log",
            "root/a. ",
        ] {
            assert!(
                build_manifest(MessageKind::File, path.to_owned(), vec![file(path, 1)]).is_err()
            );
        }
        assert!(
            build_manifest(
                MessageKind::Folder,
                "root".to_owned(),
                vec![directory("root"), file("root/missing/a.txt", 1)],
            )
            .is_err()
        );
    }

    #[test]
    fn rejects_single_file_with_multiple_entries_and_size_overflow() {
        assert!(
            build_manifest(
                MessageKind::File,
                "a.txt".to_owned(),
                vec![file("a.txt", 1), file("b.txt", 1)],
            )
            .is_err()
        );
        assert!(
            build_manifest(
                MessageKind::File,
                "a.txt".to_owned(),
                vec![file("a.txt", i64::MAX as u64 + 1)],
            )
            .is_err()
        );
    }

    #[test]
    fn accepts_ten_thousand_files_plus_root_and_rejects_one_more_file() {
        let mut sources = Vec::with_capacity(MAX_FILE_ENTRIES + 2);
        sources.push(directory("root"));
        sources.extend((0..MAX_FILE_ENTRIES).map(|index| file(&format!("root/{index:05}.bin"), 1)));
        let manifest =
            build_manifest(MessageKind::Folder, "root".to_owned(), sources.clone()).unwrap();
        assert_eq!(manifest.entries.len(), MAX_FILE_ENTRIES + 1);
        assert_eq!(manifest.total_size, MAX_FILE_ENTRIES as u64);

        sources.push(file("root/overflow.bin", 1));
        assert!(matches!(
            build_manifest(MessageKind::Folder, "root".to_owned(), sources),
            Err(ManifestError::EntryLimit)
        ));
    }

    #[test]
    fn enumerates_real_tree_in_parent_before_child_order_and_skips_symlinks() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("root");
        fs::create_dir_all(root.join("empty")).unwrap();
        fs::create_dir_all(root.join("nested")).unwrap();
        fs::write(root.join("nested").join("a.bin"), b"abc").unwrap();
        let manifest = enumerate_path(&root, MessageKind::Folder).unwrap();
        assert_eq!(manifest.total_size, 3);
        assert!(
            manifest
                .entries
                .iter()
                .any(|entry| entry.relative_path == "root/empty")
        );
        let parent = manifest
            .entries
            .iter()
            .position(|entry| entry.relative_path == "root/nested")
            .unwrap();
        let child = manifest
            .entries
            .iter()
            .position(|entry| entry.relative_path == "root/nested/a.bin")
            .unwrap();
        assert!(parent < child);
    }
}
