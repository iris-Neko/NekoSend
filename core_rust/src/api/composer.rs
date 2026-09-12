use super::core::{MessageDto, SendSourceDto, SourceItemDto, send_source_items, send_text_message};
use crate::{
    domain::{MessageKind, TransferEntryKind},
    transfer::enumerate_path,
};

#[derive(Debug, Clone)]
pub struct ComposerSourceDto {
    pub display_name: String,
    pub kind: String,
    pub total_size: u64,
    pub sources: Vec<SourceItemDto>,
}

// These entrypoints run through FRB's asynchronous worker rather than Dart's UI thread.
pub async fn prepare_composer_source(
    path: String,
    kind: String,
) -> Result<ComposerSourceDto, String> {
    let kind = match kind.as_str() {
        "file" => MessageKind::File,
        "image" => MessageKind::Image,
        "folder" => MessageKind::Folder,
        _ => return Err("Unsupported attachment kind".to_owned()),
    };
    let manifest = enumerate_path(path, kind).map_err(|error| error.to_string())?;
    for entry in &manifest.entries {
        if entry.entry_kind == TransferEntryKind::File {
            std::fs::File::open(entry.source_ref.as_ref().ok_or("Missing source")?)
                .map_err(|error| error.to_string())?;
        }
    }
    Ok(ComposerSourceDto {
        display_name: manifest.display_name,
        kind: match kind {
            MessageKind::Image => "image",
            MessageKind::Folder => "folder",
            _ => "file",
        }
        .to_owned(),
        total_size: manifest.total_size,
        sources: manifest
            .entries
            .into_iter()
            .map(|entry| SourceItemDto {
                entry_kind: match entry.entry_kind {
                    TransferEntryKind::File => "file",
                    TransferEntryKind::Directory => "directory",
                }
                .to_owned(),
                source_ref: entry.source_ref,
                relative_path: entry.relative_path,
                size: entry.size,
                modified_at_ms: entry.modified_at_ms,
            })
            .collect(),
    })
}

pub async fn submit_composer_attachment(
    client_operation_id: String,
    conversation_id: String,
    source: ComposerSourceDto,
) -> Result<SendSourceDto, String> {
    send_source_items(
        client_operation_id,
        conversation_id,
        source.display_name,
        source.kind,
        source.sources,
    )
}

pub async fn submit_composer_text(
    client_operation_id: String,
    conversation_id: String,
    text: String,
) -> Result<MessageDto, String> {
    send_text_message(client_operation_id, conversation_id, text)
}
