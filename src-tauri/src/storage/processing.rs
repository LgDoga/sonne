use std::{
    collections::{HashMap, HashSet},
    io::Cursor,
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::State;
use uuid::Uuid;

use super::{MAX_FILE_SIZE_BYTES, Storage};

const MAX_SECTIONS: usize = 10_000;
const MAX_CHUNKS: usize = 50_000;
const MAX_DOCUMENT_TEXT_BYTES: usize = 32 * 1024 * 1024;
const MAX_CHUNK_BYTES: usize = 256 * 1024;
const MAX_METADATA_BYTES: usize = 256 * 1024;
const MAX_CONTEXT_BYTES: usize = 8 * 1024 * 1024;
const MAX_DOCUMENT_ASSETS: usize = 1_000;
const MAX_ASSET_BYTES: usize = 16 * 1024 * 1024;
const MAX_DOCUMENT_ASSET_BYTES: usize = 64 * 1024 * 1024;

fn now_timestamp() -> Result<i64, String> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("System clock error: {error}"))?;

    i64::try_from(duration.as_millis()).map_err(|error| format!("Timestamp overflow: {error}"))
}

fn validate_extractor(extractor: &str, version: &str) -> Result<(), String> {
    if extractor.is_empty()
        || extractor.len() > 64
        || !extractor
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err("Invalid document extractor identifier".into());
    }

    if version.is_empty() || version.len() > 64 {
        return Err("Invalid document extractor version".into());
    }

    Ok(())
}

fn validate_json_size(value: &Option<Value>, label: &str) -> Result<Option<String>, String> {
    let Some(value) = value else {
        return Ok(None);
    };

    let serialized = serde_json::to_string(value)
        .map_err(|error| format!("Cannot serialize {label}: {error}"))?;

    if serialized.len() > MAX_METADATA_BYTES {
        return Err(format!("{label} exceeds the metadata size limit"));
    }

    Ok(Some(serialized))
}

fn resolve_blob(
    storage: &Storage,
    chat_id: &str,
    reference_id: &str,
) -> Result<(String, i64), String> {
    storage
        .connection
        .query_row(
            "SELECT v.blob_sha256, b.original_size
             FROM attachment_references r
             JOIN attachment_versions v ON v.id = r.attachment_version_id
             JOIN blobs b ON b.sha256 = v.blob_sha256
             WHERE r.id = ?1 AND r.chat_id = ?2",
            params![reference_id, chat_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| format!("Cannot resolve attachment for processing: {error}"))?
        .ok_or_else(|| "Attachment does not belong to the specified chat".into())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentProcessingStatus {
    pub document_id: Option<String>,
    pub status: Option<String>,
    pub detected_format: Option<String>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub processed_at: Option<i64>,
    pub source_size: Option<i64>,
}

fn query_status(
    storage: &Storage,
    sha256: &str,
    extractor: &str,
    extractor_version: &str,
) -> Result<AttachmentProcessingStatus, String> {
    storage
        .connection
        .query_row(
            "SELECT id, status, detected_format, error_code,
                    error_message, processed_at, source_size
             FROM extracted_documents
             WHERE blob_sha256 = ?1
               AND extractor = ?2
               AND extractor_version = ?3",
            params![sha256, extractor, extractor_version],
            |row| {
                Ok(AttachmentProcessingStatus {
                    document_id: row.get(0)?,
                    status: row.get(1)?,
                    detected_format: row.get(2)?,
                    error_code: row.get(3)?,
                    error_message: row.get(4)?,
                    processed_at: row.get(5)?,
                    source_size: row.get(6)?,
                })
            },
        )
        .optional()
        .map_err(|error| format!("Cannot read processing status: {error}"))
        .map(|status| {
            status.unwrap_or(AttachmentProcessingStatus {
                document_id: None,
                status: None,
                detected_format: None,
                error_code: None,
                error_message: None,
                processed_at: None,
                source_size: None,
            })
        })
}

pub fn recover_interrupted_document_processing(
    connection: &rusqlite::Connection,
) -> Result<usize, String> {
    connection
        .execute(
            "UPDATE extracted_documents
             SET status = 'pending',
                 error_code = NULL,
                 error_message = NULL,
                 updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             WHERE status = 'processing'",
            [],
        )
        .map_err(|error| format!("Cannot recover interrupted document processing: {error}"))
}

#[tauri::command]
pub fn get_attachment_processing_status(
    state: State<'_, Mutex<Storage>>,
    chat_id: String,
    reference_id: String,
    extractor: String,
    extractor_version: String,
) -> Result<AttachmentProcessingStatus, String> {
    validate_extractor(&extractor, &extractor_version)?;

    let storage = state
        .lock()
        .map_err(|error| format!("Storage lock poisoned: {error}"))?;

    let (sha256, _) = resolve_blob(&storage, &chat_id, &reference_id)?;
    query_status(&storage, &sha256, &extractor, &extractor_version)
}

#[tauri::command]
pub fn begin_attachment_processing(
    state: State<'_, Mutex<Storage>>,
    chat_id: String,
    reference_id: String,
    extractor: String,
    extractor_version: String,
) -> Result<AttachmentProcessingStatus, String> {
    validate_extractor(&extractor, &extractor_version)?;

    let storage = state
        .lock()
        .map_err(|error| format!("Storage lock poisoned: {error}"))?;

    let (sha256, source_size) = resolve_blob(&storage, &chat_id, &reference_id)?;
    let timestamp = now_timestamp()?;
    let document_id = Uuid::new_v4().to_string();

    storage
        .connection
        .execute(
            "INSERT INTO extracted_documents (
            id, blob_sha256, extractor, extractor_version, status,
            created_at, updated_at, source_size
         ) VALUES (?1, ?2, ?3, ?4, 'processing', ?5, ?5, ?6)
         ON CONFLICT(blob_sha256, extractor, extractor_version)
         DO UPDATE SET
            status = CASE
                WHEN extracted_documents.status = 'completed'
                THEN 'completed'
                ELSE 'processing'
            END,
            error_code = CASE
                WHEN extracted_documents.status = 'completed'
                THEN extracted_documents.error_code
                ELSE NULL
            END,
            error_message = CASE
                WHEN extracted_documents.status = 'completed'
                THEN extracted_documents.error_message
                ELSE NULL
            END,
            updated_at = excluded.updated_at,
            source_size = excluded.source_size",
            params![
                document_id,
                sha256,
                extractor,
                extractor_version,
                timestamp,
                source_size
            ],
        )
        .map_err(|error| format!("Cannot start document processing: {error}"))?;

    query_status(&storage, &sha256, &extractor, &extractor_version)
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractedSectionInput {
    pub key: String,
    pub parent_key: Option<String>,
    pub section_type: String,
    pub title: Option<String>,
    pub ordinal: i64,
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractedChunkInput {
    pub section_key: Option<String>,
    pub content: String,
    pub token_count: Option<i64>,
    pub start_offset: Option<i64>,
    pub end_offset: Option<i64>,
    pub chunk_type: String,
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractedAssetInput {
    pub section_key: Option<String>,
    pub asset_type: String,
    pub mime_type: Option<String>,
    pub file_name: Option<String>,
    pub ordinal: i64,
    pub data: Vec<u8>,
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveExtractedDocumentInput {
    pub chat_id: String,
    pub reference_id: String,
    pub extractor: String,
    pub extractor_version: String,
    pub detected_format: String,
    pub language: Option<String>,
    pub metadata: Option<Value>,
    pub source_size: i64,
    pub sections: Vec<ExtractedSectionInput>,
    pub chunks: Vec<ExtractedChunkInput>,
    #[serde(default)]
    pub assets: Vec<ExtractedAssetInput>,
}

fn validate_document(input: &SaveExtractedDocumentInput) -> Result<(), String> {
    validate_extractor(&input.extractor, &input.extractor_version)?;

    if input.detected_format.is_empty() || input.detected_format.len() > 128 {
        return Err("Invalid detected document format".into());
    }

    if input.source_size < 0 || input.source_size as u64 > MAX_FILE_SIZE_BYTES {
        return Err("Document source size is outside the allowed limit".into());
    }

    if input.sections.is_empty() || input.sections.len() > MAX_SECTIONS {
        return Err(format!(
            "Document must contain between 1 and {MAX_SECTIONS} sections"
        ));
    }

    if input.chunks.is_empty() || input.chunks.len() > MAX_CHUNKS {
        return Err(format!(
            "Document must contain between 1 and {MAX_CHUNKS} chunks"
        ));
    }

    validate_json_size(&input.metadata, "Document metadata")?;

    if input.assets.len() > MAX_DOCUMENT_ASSETS {
        return Err(format!(
            "Document contains more than {MAX_DOCUMENT_ASSETS} embedded assets"
        ));
    }

    let section_keys: HashSet<&str> =
        input.sections.iter().map(|section| section.key.as_str()).collect();

    let mut total_asset_bytes = 0usize;

    for asset in &input.assets {
        if asset.asset_type != "image" && asset.asset_type != "chart" {
            return Err(format!("Unsupported document asset type: {}", asset.asset_type));
        }

        if asset.ordinal < 0 {
            return Err("Asset ordinal cannot be negative".into());
        }

        if asset.data.is_empty() || asset.data.len() > MAX_ASSET_BYTES {
            return Err("Embedded asset is empty or exceeds the 16 MiB limit".into());
        }

        total_asset_bytes = total_asset_bytes
            .checked_add(asset.data.len())
            .ok_or_else(|| "Document asset size overflow".to_string())?;

        if total_asset_bytes > MAX_DOCUMENT_ASSET_BYTES {
            return Err("Document embedded assets exceed the 64 MiB limit".into());
        }

        if asset.mime_type.as_ref().is_some_and(|value| value.is_empty() || value.len() > 128) {
            return Err("Invalid embedded asset MIME type".into());
        }

        if asset.file_name.as_ref().is_some_and(|value| value.len() > 512) {
            return Err("Embedded asset filename is too long".into());
        }

        if let Some(key) = &asset.section_key {
            if !section_keys.contains(key.as_str()) {
                return Err(format!("Unknown asset section key: {key}"));
            }
        }

        validate_json_size(&asset.metadata, "Asset metadata")?;
    }

    let allowed_types = [
        "root",
        "page",
        "sheet",
        "slide",
        "chapter",
        "table",
        "paragraph",
        "header",
        "footer",
        "notes",
        "archive_entry",
    ];

    let mut keys = HashSet::new();
    let mut total_text_bytes = 0usize;

    for section in &input.sections {
        if section.key.is_empty() || section.key.len() > 128 || !keys.insert(section.key.as_str()) {
            return Err("Document contains an empty or duplicate section key".into());
        }

        if !allowed_types.contains(&section.section_type.as_str()) {
            return Err(format!(
                "Unsupported document section type: {}",
                section.section_type
            ));
        }

        if section.ordinal < 0 {
            return Err("Section ordinal cannot be negative".into());
        }

        validate_json_size(&section.metadata, "Section metadata")?;
    }

    let mut seen_sections = HashSet::new();

    for section in &input.sections {
        if let Some(parent_key) = &section.parent_key {
            if !seen_sections.contains(parent_key.as_str()) {
                return Err("Section parent must appear before its child".into());
            }
        }
        seen_sections.insert(section.key.as_str());
    }

    for chunk in &input.chunks {
        if chunk.content.trim().is_empty() {
            return Err("Document contains an empty text chunk".into());
        }

        if chunk.content.len() > MAX_CHUNK_BYTES {
            return Err("A document text chunk exceeds 256 KiB".into());
        }

        total_text_bytes = total_text_bytes
            .checked_add(chunk.content.len())
            .ok_or_else(|| "Document text size overflow".to_string())?;

        if chunk.chunk_type.is_empty() || chunk.chunk_type.len() > 64 {
            return Err("Invalid document chunk type".into());
        }

        if chunk.token_count.is_some_and(|value| value < 0)
            || chunk.start_offset.is_some_and(|value| value < 0)
            || chunk.end_offset.is_some_and(|value| value < 0)
        {
            return Err("Chunk token count and offsets cannot be negative".into());
        }

        if let (Some(start), Some(end)) = (chunk.start_offset, chunk.end_offset) {
            if end < start {
                return Err("Chunk end offset cannot precede its start offset".into());
            }
        }

        validate_json_size(&chunk.metadata, "Chunk metadata")?;
    }

    if total_text_bytes > MAX_DOCUMENT_TEXT_BYTES {
        return Err("Extracted document text exceeds the 32 MiB limit".into());
    }

    Ok(())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedDocumentRecord {
    pub document_id: String,
    pub status: String,
    pub section_count: usize,
    pub chunk_count: usize,
    pub text_bytes: usize,
}

#[tauri::command]
pub fn save_extracted_document(
    state: State<'_, Mutex<Storage>>,
    input: SaveExtractedDocumentInput,
) -> Result<SavedDocumentRecord, String> {
    validate_document(&input)?;

    let mut storage = state
        .lock()
        .map_err(|error| format!("Storage lock poisoned: {error}"))?;

    let (sha256, actual_source_size) = resolve_blob(&storage, &input.chat_id, &input.reference_id)?;

    if actual_source_size != input.source_size {
        return Err("Document source size does not match the stored attachment".into());
    }

    let timestamp = now_timestamp()?;
    let document_metadata = validate_json_size(&input.metadata, "Document metadata")?;

    let stored_assets = input
        .assets
        .iter()
        .map(|asset| {
            let blob = storage.store_blob_with_limit(
                Cursor::new(asset.data.as_slice()),
                MAX_ASSET_BYTES as u64,
            )?;

            Ok((asset, blob.sha256))
        })
        .collect::<Result<Vec<_>, String>>()?;

    let transaction = storage
        .connection
        .transaction()
        .map_err(|error| format!("Cannot begin document transaction: {error}"))?;

    let document_id = Uuid::new_v4().to_string();

    transaction
        .execute(
            "INSERT INTO extracted_documents (
            id, blob_sha256, extractor, extractor_version, status,
            language, created_at, updated_at, detected_format,
            metadata_json, error_code, error_message, processed_at, source_size
         ) VALUES (?1, ?2, ?3, ?4, 'processing', ?5, ?6, ?6, ?7, ?8, NULL, NULL, NULL, ?9)
         ON CONFLICT(blob_sha256, extractor, extractor_version)
         DO UPDATE SET
            status = 'processing',
            language = excluded.language,
            updated_at = excluded.updated_at,
            detected_format = excluded.detected_format,
            metadata_json = excluded.metadata_json,
            error_code = NULL,
            error_message = NULL,
            processed_at = NULL,
            source_size = excluded.source_size",
            params![
                document_id,
                sha256,
                input.extractor,
                input.extractor_version,
                input.language,
                timestamp,
                input.detected_format,
                document_metadata,
                input.source_size
            ],
        )
        .map_err(|error| format!("Cannot prepare extracted document record: {error}"))?;

    let resolved_document_id: String = transaction
        .query_row(
            "SELECT id FROM extracted_documents
         WHERE blob_sha256 = ?1 AND extractor = ?2 AND extractor_version = ?3",
            params![sha256, input.extractor, input.extractor_version],
            |row| row.get(0),
        )
        .map_err(|error| format!("Cannot resolve extracted document record: {error}"))?;

    transaction
        .execute(
            "DELETE FROM document_assets WHERE document_id = ?1",
            params![resolved_document_id],
        )
        .map_err(|error| format!("Cannot replace previous document assets: {error}"))?;

    transaction
        .execute(
            "DELETE FROM document_vectors_nomic
         WHERE chunk_id IN (
             SELECT id FROM document_chunks WHERE document_id = ?1
         )",
            params![resolved_document_id],
        )
        .map_err(|error| format!("Cannot remove previous document vectors: {error}"))?;
    transaction
        .execute(
            "DELETE FROM document_chunks WHERE document_id = ?1",
            params![resolved_document_id],
        )
        .map_err(|error| format!("Cannot replace previous document chunks: {error}"))?;

    transaction
        .execute(
            "DELETE FROM document_sections WHERE document_id = ?1",
            params![resolved_document_id],
        )
        .map_err(|error| format!("Cannot replace previous document sections: {error}"))?;

    let mut section_ids: HashMap<String, String> = HashMap::new();

    for section in &input.sections {
        let id = Uuid::new_v4().to_string();
        let parent_id = match &section.parent_key {
            Some(key) => Some(
                section_ids
                    .get(key)
                    .ok_or_else(|| format!("Unknown parent section key: {key}"))?
                    .clone(),
            ),
            None => None,
        };
        let metadata = validate_json_size(&section.metadata, "Section metadata")?;

        transaction
            .execute(
                "INSERT INTO document_sections (
                id, document_id, parent_id, section_type,
                title, ordinal, metadata_json, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    id,
                    resolved_document_id,
                    parent_id,
                    section.section_type,
                    section.title,
                    section.ordinal,
                    metadata,
                    timestamp
                ],
            )
            .map_err(|error| format!("Cannot save document section: {error}"))?;

        section_ids.insert(section.key.clone(), id);
    }

    for (asset, blob_sha256) in &stored_assets {
        let section_id = match &asset.section_key {
            Some(key) => Some(
                section_ids
                    .get(key)
                    .ok_or_else(|| format!("Unknown asset section key: {key}"))?
                    .clone(),
            ),
            None => None,
        };

        let metadata = validate_json_size(&asset.metadata, "Asset metadata")?;

        transaction
            .execute(
                "INSERT INTO document_assets (
                    id, document_id, section_id, blob_sha256,
                    asset_type, mime_type, file_name, metadata_json,
                    ordinal, created_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    Uuid::new_v4().to_string(),
                    resolved_document_id,
                    section_id,
                    blob_sha256,
                    asset.asset_type,
                    asset.mime_type,
                    asset.file_name,
                    metadata,
                    asset.ordinal,
                    timestamp
                ],
            )
            .map_err(|error| format!("Cannot save document asset: {error}"))?;
    }

    let mut total_text_bytes = 0usize;

    for (ordinal, chunk) in input.chunks.iter().enumerate() {
        total_text_bytes += chunk.content.len();

        let section_id = match &chunk.section_key {
            Some(key) => Some(
                section_ids
                    .get(key)
                    .ok_or_else(|| format!("Unknown chunk section key: {key}"))?
                    .clone(),
            ),
            None => None,
        };

        let metadata = validate_json_size(&chunk.metadata, "Chunk metadata")?;

        transaction
            .execute(
                "INSERT INTO document_chunks (
                id, document_id, ordinal, content, token_count,
                start_offset, end_offset, section_id, chunk_type, metadata_json
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    Uuid::new_v4().to_string(),
                    resolved_document_id,
                    i64::try_from(ordinal)
                        .map_err(|error| format!("Chunk ordinal overflow: {error}"))?,
                    chunk.content,
                    chunk.token_count,
                    chunk.start_offset,
                    chunk.end_offset,
                    section_id,
                    chunk.chunk_type,
                    metadata
                ],
            )
            .map_err(|error| format!("Cannot save document text chunk: {error}"))?;
    }

    transaction
        .commit()
        .map_err(|error| format!("Cannot commit extracted document: {error}"))?;

    Ok(SavedDocumentRecord {
        document_id: resolved_document_id,
        status: "processing".into(),
        section_count: input.sections.len(),
        chunk_count: input.chunks.len(),
        text_bytes: total_text_bytes,
    })
}

#[tauri::command]
pub fn mark_attachment_processing_failed(
    state: State<'_, Mutex<Storage>>,
    chat_id: String,
    reference_id: String,
    extractor: String,
    extractor_version: String,
    error_code: String,
    error_message: String,
) -> Result<AttachmentProcessingStatus, String> {
    validate_extractor(&extractor, &extractor_version)?;

    if error_code.is_empty() || error_code.len() > 64 {
        return Err("Invalid processing error code".into());
    }

    let safe_message: String = error_message.chars().take(2_000).collect();

    let storage = state
        .lock()
        .map_err(|error| format!("Storage lock poisoned: {error}"))?;

    let (sha256, source_size) = resolve_blob(&storage, &chat_id, &reference_id)?;
    let timestamp = now_timestamp()?;

    storage
        .connection
        .execute(
            "INSERT INTO extracted_documents (
            id, blob_sha256, extractor, extractor_version, status,
            created_at, updated_at, error_code, error_message,
            processed_at, source_size
         ) VALUES (?1, ?2, ?3, ?4, 'failed', ?5, ?5, ?6, ?7, ?5, ?8)
         ON CONFLICT(blob_sha256, extractor, extractor_version)
         DO UPDATE SET
            status = 'failed',
            error_code = excluded.error_code,
            error_message = excluded.error_message,
            updated_at = excluded.updated_at,
            processed_at = excluded.processed_at,
            source_size = excluded.source_size",
            params![
                Uuid::new_v4().to_string(),
                sha256,
                extractor,
                extractor_version,
                timestamp,
                error_code,
                safe_message,
                source_size
            ],
        )
        .map_err(|error| format!("Cannot save document processing failure: {error}"))?;

    query_status(&storage, &sha256, &extractor, &extractor_version)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentDocumentContext {
    pub document_id: String,
    pub file_name: String,
    pub detected_format: Option<String>,
    pub language: Option<String>,
    pub metadata: Option<Value>,
    pub content: String,
    pub truncated: bool,
}

#[tauri::command]
pub fn get_attachment_document_context(
    state: State<'_, Mutex<Storage>>,
    chat_id: String,
    reference_id: String,
    extractor: String,
    extractor_version: String,
    max_bytes: Option<usize>,
) -> Result<AttachmentDocumentContext, String> {
    validate_extractor(&extractor, &extractor_version)?;

    let limit = max_bytes.unwrap_or(2 * 1024 * 1024);
    if limit == 0 || limit > MAX_CONTEXT_BYTES {
        return Err(format!(
            "Context limit must be between 1 byte and {} MiB",
            MAX_CONTEXT_BYTES / (1024 * 1024)
        ));
    }

    let storage = state
        .lock()
        .map_err(|error| format!("Storage lock poisoned: {error}"))?;

    let (sha256, _) = resolve_blob(&storage, &chat_id, &reference_id)?;

    let document: Option<(
        String,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
    )> = storage
        .connection
        .query_row(
            "SELECT d.id, v.file_name, d.detected_format, d.language, d.metadata_json
             FROM extracted_documents d
             JOIN attachment_versions v ON v.blob_sha256 = d.blob_sha256
             JOIN attachment_references r ON r.attachment_version_id = v.id
             WHERE d.blob_sha256 = ?1
               AND d.extractor = ?2
               AND d.extractor_version = ?3
               AND d.status = 'completed'
               AND r.id = ?4 AND r.chat_id = ?5
             ORDER BY r.created_at DESC
             LIMIT 1",
            params![sha256, extractor, extractor_version, reference_id, chat_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .optional()
        .map_err(|error| format!("Cannot resolve processed attachment: {error}"))?;

    let (document_id, file_name, detected_format, language, metadata_json) =
        document.ok_or_else(|| "Attachment has no completed extraction result".to_string())?;

    let metadata = metadata_json
        .as_deref()
        .map(serde_json::from_str::<Value>)
        .transpose()
        .map_err(|error| format!("Stored document metadata is invalid: {error}"))?;

    let mut statement = storage
        .connection
        .prepare(
            "SELECT content FROM document_chunks
         WHERE document_id = ?1
         ORDER BY ordinal ASC",
        )
        .map_err(|error| format!("Cannot prepare document context query: {error}"))?;

    let rows = statement
        .query_map(params![document_id], |row| row.get::<_, String>(0))
        .map_err(|error| format!("Cannot query document context: {error}"))?;

    let mut content = String::new();
    let mut truncated = false;

    for row in rows {
        let chunk = row.map_err(|error| format!("Cannot read document context chunk: {error}"))?;
        let separator = if content.is_empty() { "" } else { "\n\n" };
        let remaining = limit.saturating_sub(content.len());

        if separator.len() + chunk.len() > remaining {
            let available = remaining.saturating_sub(separator.len());
            let mut end = available.min(chunk.len());
            while end > 0 && !chunk.is_char_boundary(end) {
                end -= 1;
            }

            if end > 0 {
                content.push_str(separator);
                content.push_str(&chunk[..end]);
            }

            truncated = true;
            break;
        }

        content.push_str(separator);
        content.push_str(&chunk);
    }

    Ok(AttachmentDocumentContext {
        document_id,
        file_name,
        detected_format,
        language,
        metadata,
        content,
        truncated,
    })
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentSearchResult {
    pub chunk_id: String,
    pub file_name: String,
    pub detected_format: Option<String>,
    pub content: String,
}

#[tauri::command]
pub fn search_attachment_documents(
    state: State<'_, Mutex<Storage>>,
    chat_id: String,
    query: String,
    query_embedding: Vec<f32>,
    limit: Option<usize>,
) -> Result<Vec<DocumentSearchResult>, String> {
    let query = query.trim();

    if query.is_empty() {
        return Ok(Vec::new());
    }
    if query.len() > 512 {
        return Err("Search query exceeds 512 bytes".into());
    }
    if query_embedding.len() != NOMIC_EMBEDDING_DIMENSIONS
        || query_embedding.iter().any(|value| !value.is_finite())
    {
        return Err(format!(
            "Query embedding must contain exactly {} finite values",
            NOMIC_EMBEDDING_DIMENSIONS
        ));
    }

    let terms: Vec<String> = query
        .split_whitespace()
        .take(16)
        .map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
        .collect();

    if terms.is_empty() {
        return Ok(Vec::new());
    }

    let fts_query = terms.join(" OR ");
    let limit = limit.unwrap_or(20).clamp(1, 50);
    let candidate_limit = (limit * 4).clamp(20, 200);

    let query_vector: Vec<u8> = query_embedding
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect();

    let storage = state
        .lock()
        .map_err(|_| "Storage lock is poisoned".to_string())?;

    let mut ranked: std::collections::HashMap<
        String,
        (DocumentSearchResult, f64),
    > = std::collections::HashMap::new();

    // Lexical retrieval: FTS5 / BM25.
    let lexical_results = {
        let mut statement = storage
            .connection
            .prepare(
                "SELECT
                    c.id,
                    (
                        SELECT v.file_name
                        FROM attachment_versions v
                        JOIN attachment_references r
                            ON r.attachment_version_id = v.id
                        WHERE v.blob_sha256 = d.blob_sha256
                          AND r.chat_id = ?2
                        ORDER BY r.created_at DESC
                        LIMIT 1
                    ),
                    d.detected_format,
                    c.content
                 FROM chunk_search
                 JOIN document_chunks c
                    ON c.id = chunk_search.chunk_id
                 JOIN extracted_documents d
                    ON d.id = c.document_id
                 WHERE chunk_search MATCH ?1
                   AND d.status = 'completed'
                   AND EXISTS (
                       SELECT 1
                       FROM attachment_versions v
                       JOIN attachment_references r
                           ON r.attachment_version_id = v.id
                       WHERE v.blob_sha256 = d.blob_sha256
                         AND r.chat_id = ?2
                   )
                 ORDER BY bm25(chunk_search) ASC
                 LIMIT ?3",
            )
            .map_err(|error| format!("Cannot prepare lexical search: {error}"))?;

        let rows = statement
            .query_map(
                params![fts_query, chat_id, candidate_limit as i64],
                |row| {
                    Ok(DocumentSearchResult {
                        chunk_id: row.get(0)?,
                        file_name: row.get(1)?,
                        detected_format: row.get(2)?,
                        content: row.get(3)?,
                    })
                },
            )
            .map_err(|error| format!("Cannot execute lexical search: {error}"))?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("Cannot read lexical search results: {error}"))?
    };

    for (index, result) in lexical_results.into_iter().enumerate() {
        let key = result.chunk_id.clone();
        let entry = ranked.entry(key).or_insert((result, 0.0));
        entry.1 += 1.0 / (60.0 + (index + 1) as f64);
    }

    // Semantic retrieval: sqlite-vec cosine distance.
    let semantic_results = {
        let mut statement = storage
            .connection
            .prepare(
                "SELECT
                    c.id,
                    (
                        SELECT v.file_name
                        FROM attachment_versions v
                        JOIN attachment_references r
                            ON r.attachment_version_id = v.id
                        WHERE v.blob_sha256 = d.blob_sha256
                          AND r.chat_id = ?3
                        ORDER BY r.created_at DESC
                        LIMIT 1
                    ),
                    d.detected_format,
                    c.content
                 FROM document_vectors_nomic
                 JOIN document_chunks c
                    ON c.id = document_vectors_nomic.chunk_id
                 JOIN extracted_documents d
                    ON d.id = c.document_id
                 WHERE embedding MATCH ?1
                   AND k = ?2
                   AND d.status = 'completed'
                   AND EXISTS (
                       SELECT 1
                       FROM attachment_versions v
                       JOIN attachment_references r
                           ON r.attachment_version_id = v.id
                       WHERE v.blob_sha256 = d.blob_sha256
                         AND r.chat_id = ?3
                   )
                 ORDER BY distance ASC",
            )
            .map_err(|error| format!("Cannot prepare semantic search: {error}"))?;

        let rows = statement
            .query_map(
                params![query_vector, candidate_limit as i64, chat_id],
                |row| {
                    Ok(DocumentSearchResult {
                        chunk_id: row.get(0)?,
                        file_name: row.get(1)?,
                        detected_format: row.get(2)?,
                        content: row.get(3)?,
                    })
                },
            )
            .map_err(|error| format!("Cannot execute semantic search: {error}"))?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("Cannot read semantic search results: {error}"))?
    };

    for (index, result) in semantic_results.into_iter().enumerate() {
        let key = result.chunk_id.clone();
        let entry = ranked.entry(key).or_insert((result, 0.0));
        entry.1 += 1.0 / (60.0 + (index + 1) as f64);
    }

    let mut results: Vec<_> = ranked.into_values().collect();
    results.sort_by(|left, right| right.1.total_cmp(&left.1));

    Ok(results
        .into_iter()
        .take(limit)
        .map(|(result, _)| result)
        .collect())
}

const NOMIC_EMBEDDING_MODEL: &str = "nomic-embed-text:latest";
const NOMIC_EMBEDDING_DIMENSIONS: usize = 768;
const MAX_EMBEDDING_BATCH: usize = 64;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentEmbeddingChunk {
    pub chunk_id: String,
    pub content: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentEmbeddingInput {
    pub chunk_id: String,
    pub vector: Vec<f32>,
}

#[tauri::command]
pub fn get_document_embedding_batch(
    state: State<'_, Mutex<Storage>>,
    document_id: String,
    limit: Option<usize>,
) -> Result<Vec<DocumentEmbeddingChunk>, String> {
    if document_id.is_empty() || document_id.len() > 128 {
        return Err("Invalid document ID".into());
    }

    let limit = limit.unwrap_or(32).clamp(1, MAX_EMBEDDING_BATCH);
    let storage = state
        .lock()
        .map_err(|_| "Storage lock is poisoned".to_string())?;

    let mut statement = storage
        .connection
        .prepare(
            "SELECT c.id, c.content
             FROM document_chunks c
             WHERE c.document_id = ?1
               AND NOT EXISTS (
                   SELECT 1 FROM embeddings e
                   WHERE e.chunk_id = c.id
                     AND e.model_name = ?2
               )
             ORDER BY c.ordinal
             LIMIT ?3",
        )
        .map_err(|error| format!("Cannot prepare embedding batch: {error}"))?;

    let rows = statement
        .query_map(
            params![document_id, NOMIC_EMBEDDING_MODEL, limit as i64],
            |row| {
                Ok(DocumentEmbeddingChunk {
                    chunk_id: row.get(0)?,
                    content: row.get(1)?,
                })
            },
        )
        .map_err(|error| format!("Cannot read embedding batch: {error}"))?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("Cannot collect embedding batch: {error}"))
}

#[tauri::command]
pub fn save_document_embedding_batch(
    state: State<'_, Mutex<Storage>>,
    document_id: String,
    embeddings: Vec<DocumentEmbeddingInput>,
) -> Result<usize, String> {
    if document_id.is_empty() || document_id.len() > 128 {
        return Err("Invalid document ID".into());
    }

    if embeddings.is_empty() || embeddings.len() > MAX_EMBEDDING_BATCH {
        return Err(format!(
            "Embedding batch must contain between 1 and {MAX_EMBEDDING_BATCH} vectors"
        ));
    }

    let mut seen = HashSet::new();
    for item in &embeddings {
        if item.chunk_id.is_empty()
            || item.chunk_id.len() > 128
            || !seen.insert(item.chunk_id.as_str())
        {
            return Err("Embedding batch contains an invalid or duplicate chunk ID".into());
        }

        if item.vector.len() != NOMIC_EMBEDDING_DIMENSIONS {
            return Err(format!(
                "Embedding must contain exactly {NOMIC_EMBEDDING_DIMENSIONS} dimensions"
            ));
        }

        if item.vector.iter().any(|value| !value.is_finite()) {
            return Err("Embedding contains a non-finite value".into());
        }
    }

    let mut storage = state
        .lock()
        .map_err(|_| "Storage lock is poisoned".to_string())?;

    let timestamp = now_timestamp()?;
    let transaction = storage
        .connection
        .transaction()
        .map_err(|error| format!("Cannot begin embedding transaction: {error}"))?;

    let document_exists: bool = transaction
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM extracted_documents WHERE id = ?1)",
            [&document_id],
            |row| row.get(0),
        )
        .map_err(|error| format!("Cannot validate embedding document: {error}"))?;

    if !document_exists {
        return Err("Embedding document does not exist".into());
    }

    for item in &embeddings {
        let belongs_to_document: bool = transaction
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM document_chunks
                    WHERE id = ?1 AND document_id = ?2
                )",
                params![item.chunk_id, document_id],
                |row| row.get(0),
            )
            .map_err(|error| format!("Cannot validate embedding chunk: {error}"))?;

        if !belongs_to_document {
            return Err("Embedding chunk does not belong to the specified document".into());
        }

        let vector_bytes: Vec<u8> = item
            .vector
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect();

        transaction
            .execute(
                "INSERT INTO embeddings (
                chunk_id, model_name, dimensions, vector, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(chunk_id, model_name)
             DO UPDATE SET
                dimensions = excluded.dimensions,
                vector = excluded.vector,
                created_at = excluded.created_at",
                params![
                    item.chunk_id,
                    NOMIC_EMBEDDING_MODEL,
                    NOMIC_EMBEDDING_DIMENSIONS as i64,
                    vector_bytes,
                    timestamp
                ],
            )
            .map_err(|error| format!("Cannot save canonical embedding: {error}"))?;

        transaction
            .execute(
                "INSERT OR REPLACE INTO document_vectors_nomic (chunk_id, embedding)
             VALUES (?1, ?2)",
                params![item.chunk_id, vector_bytes],
            )
            .map_err(|error| format!("Cannot update vector search index: {error}"))?;
    }

    transaction
        .commit()
        .map_err(|error| format!("Cannot commit embedding batch: {error}"))?;

    Ok(embeddings.len())
}

#[tauri::command]
pub fn finalize_document_embedding_index(
    state: State<'_, Mutex<Storage>>,
    document_id: String,
) -> Result<(), String> {
    if document_id.is_empty() || document_id.len() > 128 {
        return Err("Invalid document ID".into());
    }

    let mut storage = state
        .lock()
        .map_err(|_| "Storage lock is poisoned".to_string())?;

    let timestamp = now_timestamp()?;
    let transaction = storage
        .connection
        .transaction()
        .map_err(|error| format!("Cannot begin embedding finalization: {error}"))?;

    let counts: (i64, i64, i64) = transaction
        .query_row(
            "SELECT
                COUNT(c.id),
                COUNT(e.chunk_id),
                COUNT(v.chunk_id)
             FROM document_chunks c
             LEFT JOIN embeddings e
                ON e.chunk_id = c.id
               AND e.model_name = ?2
             LEFT JOIN document_vectors_nomic v
                ON v.chunk_id = c.id
             WHERE c.document_id = ?1",
            params![document_id, NOMIC_EMBEDDING_MODEL],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|error| format!("Cannot verify document embedding index: {error}"))?;

    if counts.0 == 0 || counts.0 != counts.1 || counts.0 != counts.2 {
        return Err(format!(
            "Document embedding index is incomplete: {} chunks, {} embeddings, {} vector entries",
            counts.0, counts.1, counts.2
        ));
    }

    let changed = transaction
        .execute(
            "UPDATE extracted_documents
         SET status = 'completed',
             updated_at = ?1,
             processed_at = ?1,
             error_code = NULL,
             error_message = NULL
         WHERE id = ?2",
            params![timestamp, document_id],
        )
        .map_err(|error| format!("Cannot finalize document processing: {error}"))?;

    if changed != 1 {
        return Err("Cannot finalize an unknown document".into());
    }

    transaction
        .commit()
        .map_err(|error| format!("Cannot commit embedding finalization: {error}"))
}


// ============================================================
// DOCUMENT MAP-REDUCE SUMMARIES
// ============================================================

const MAX_SUMMARY_PARTS_BATCH: usize = 128;
const MAX_SUMMARY_PART_BYTES: usize = 64 * 1024;
const MAX_FINAL_SUMMARY_BYTES: usize = 1024 * 1024;
const MAX_SUMMARY_MODEL_NAME: usize = 128;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentSummaryChunk {
    pub chunk_id: String,
    pub ordinal: i64,
    pub content: String,
    pub chunk_type: String,
    pub chunk_metadata: Option<String>,
    pub section_type: Option<String>,
    pub section_title: Option<String>,
    pub section_metadata: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentSummaryPartInput {
    pub ordinal: i64,
    pub content: String,
    pub source_chunk_count: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentSummaryPart {
    pub ordinal: i64,
    pub content: String,
    pub source_chunk_count: i64,
    pub model_name: String,
    pub summarizer_version: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentSummaryRecord {
    pub document_id: String,
    pub summary: String,
    pub model_name: String,
    pub summarizer_version: String,
    pub status: String,
    pub source_chunk_count: i64,
    pub updated_at: i64,
}

#[tauri::command]
pub fn get_document_summary_chunk_batch(
    state: State<'_, Mutex<Storage>>,
    document_id: String,
    after_ordinal: Option<i64>,
    limit: Option<usize>,
) -> Result<Vec<DocumentSummaryChunk>, String> {
    if document_id.is_empty() || document_id.len() > 128 {
        return Err("Invalid document ID".into());
    }

    let after = after_ordinal.unwrap_or(-1);
    if after < -1 {
        return Err("Invalid summary chunk cursor".into());
    }

    let limit = limit.unwrap_or(64);
    if !(1..=128).contains(&limit) {
        return Err("Summary chunk batch limit must be between 1 and 128".into());
    }

    let storage = state
        .lock()
        .map_err(|_| "Storage lock is poisoned".to_string())?;

    let mut statement = storage.connection.prepare(
        "SELECT
            c.id,
            c.ordinal,
            c.content,
            c.chunk_type,
            c.metadata_json,
            s.section_type,
            s.title,
            s.metadata_json
         FROM document_chunks c
         LEFT JOIN document_sections s ON s.id = c.section_id
         WHERE c.document_id = ?1 AND c.ordinal > ?2
         ORDER BY c.ordinal ASC
         LIMIT ?3"
    ).map_err(|error| format!("Cannot prepare summary chunk query: {error}"))?;

    let rows = statement.query_map(
        params![document_id, after, limit as i64],
        |row| {
            Ok(DocumentSummaryChunk {
                chunk_id: row.get(0)?,
                ordinal: row.get(1)?,
                content: row.get(2)?,
                chunk_type: row.get(3)?,
                chunk_metadata: row.get(4)?,
                section_type: row.get(5)?,
                section_title: row.get(6)?,
                section_metadata: row.get(7)?,
            })
        },
    ).map_err(|error| format!("Cannot query summary chunks: {error}"))?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("Cannot collect summary chunks: {error}"))
}

#[tauri::command]
pub fn clear_document_summary_stage(
    state: State<'_, Mutex<Storage>>,
    document_id: String,
    stage: i64,
) -> Result<usize, String> {
    if document_id.is_empty() || document_id.len() > 128 || !(0..=32).contains(&stage) {
        return Err("Invalid document summary stage".into());
    }

    let storage = state
        .lock()
        .map_err(|_| "Storage lock is poisoned".to_string())?;

    let changed = storage.connection.execute(
        "DELETE FROM document_summary_parts WHERE document_id = ?1 AND stage = ?2",
        params![document_id, stage],
    ).map_err(|error| format!("Cannot clear document summary stage: {error}"))?;

    Ok(changed)
}

#[tauri::command]
pub fn save_document_summary_parts(
    state: State<'_, Mutex<Storage>>,
    document_id: String,
    stage: i64,
    model_name: String,
    summarizer_version: String,
    parts: Vec<DocumentSummaryPartInput>,
) -> Result<usize, String> {
    if document_id.is_empty()
        || document_id.len() > 128
        || !(0..=32).contains(&stage)
        || model_name.is_empty()
        || model_name.len() > MAX_SUMMARY_MODEL_NAME
        || summarizer_version.is_empty()
        || summarizer_version.len() > 128
        || parts.is_empty()
        || parts.len() > MAX_SUMMARY_PARTS_BATCH
    {
        return Err("Invalid document summary batch".into());
    }

    let mut seen = HashSet::new();
    let total_bytes: usize = parts.iter().map(|part| part.content.len()).sum();

    if total_bytes > 8 * 1024 * 1024 {
        return Err("Document summary batch exceeds 8 MiB".into());
    }

    for part in &parts {
        if part.ordinal < 0
            || !seen.insert(part.ordinal)
            || part.content.is_empty()
            || part.content.len() > MAX_SUMMARY_PART_BYTES
            || part.source_chunk_count <= 0
            || part.source_chunk_count > MAX_CHUNKS as i64
        {
            return Err("Document summary batch contains an invalid part".into());
        }
    }

    let mut storage = state
        .lock()
        .map_err(|_| "Storage lock is poisoned".to_string())?;
    let timestamp = now_timestamp()?;

    let transaction = storage.connection.transaction()
        .map_err(|error| format!("Cannot begin summary transaction: {error}"))?;

    let exists: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM extracted_documents WHERE id = ?1)",
        [&document_id],
        |row| row.get(0),
    ).map_err(|error| format!("Cannot validate summary document: {error}"))?;

    if !exists {
        return Err("Summary document does not exist".into());
    }

    for part in &parts {
        transaction.execute(
            "INSERT INTO document_summary_parts (
                document_id, stage, ordinal, content, source_chunk_count,
                model_name, summarizer_version, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(document_id, stage, ordinal)
             DO UPDATE SET
                content = excluded.content,
                source_chunk_count = excluded.source_chunk_count,
                model_name = excluded.model_name,
                summarizer_version = excluded.summarizer_version,
                updated_at = excluded.updated_at",
            params![
                document_id,
                stage,
                part.ordinal,
                part.content,
                part.source_chunk_count,
                model_name,
                summarizer_version,
                timestamp
            ],
        ).map_err(|error| format!("Cannot save summary part: {error}"))?;
    }

    transaction.commit()
        .map_err(|error| format!("Cannot commit summary parts: {error}"))?;

    Ok(parts.len())
}

#[tauri::command]
pub fn get_document_summary_parts(
    state: State<'_, Mutex<Storage>>,
    document_id: String,
    stage: i64,
    after_ordinal: Option<i64>,
    limit: Option<usize>,
) -> Result<Vec<DocumentSummaryPart>, String> {
    if document_id.is_empty() || document_id.len() > 128 || !(0..=32).contains(&stage) {
        return Err("Invalid document summary stage".into());
    }

    let after = after_ordinal.unwrap_or(-1);
    if after < -1 {
        return Err("Invalid summary part cursor".into());
    }

    let limit = limit.unwrap_or(64);
    if !(1..=128).contains(&limit) {
        return Err("Summary part batch limit must be between 1 and 128".into());
    }

    let storage = state
        .lock()
        .map_err(|_| "Storage lock is poisoned".to_string())?;

    let mut statement = storage.connection.prepare(
        "SELECT ordinal, content, source_chunk_count,
                model_name, summarizer_version
         FROM document_summary_parts
         WHERE document_id = ?1 AND stage = ?2 AND ordinal > ?3
         ORDER BY ordinal ASC
         LIMIT ?4"
    ).map_err(|error| format!("Cannot prepare summary parts query: {error}"))?;

    let rows = statement.query_map(
        params![document_id, stage, after, limit as i64],
        |row| {
            Ok(DocumentSummaryPart {
                ordinal: row.get(0)?,
                content: row.get(1)?,
                source_chunk_count: row.get(2)?,
                model_name: row.get(3)?,
                summarizer_version: row.get(4)?,
            })
        },
    ).map_err(|error| format!("Cannot query summary parts: {error}"))?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("Cannot collect summary parts: {error}"))
}

#[tauri::command]
pub fn finalize_document_summary(
    state: State<'_, Mutex<Storage>>,
    document_id: String,
    summary: String,
    model_name: String,
    summarizer_version: String,
    source_chunk_count: i64,
) -> Result<(), String> {
    if document_id.is_empty()
        || document_id.len() > 128
        || summary.trim().is_empty()
        || summary.len() > MAX_FINAL_SUMMARY_BYTES
        || model_name.is_empty()
        || model_name.len() > MAX_SUMMARY_MODEL_NAME
        || summarizer_version.is_empty()
        || summarizer_version.len() > 128
        || source_chunk_count <= 0
        || source_chunk_count > MAX_CHUNKS as i64
    {
        return Err("Invalid final document summary".into());
    }

    let mut storage = state
        .lock()
        .map_err(|_| "Storage lock is poisoned".to_string())?;
    let timestamp = now_timestamp()?;

    let transaction = storage.connection.transaction()
        .map_err(|error| format!("Cannot begin final summary transaction: {error}"))?;

    let actual_count: i64 = transaction.query_row(
        "SELECT COUNT(*) FROM document_chunks WHERE document_id = ?1",
        [&document_id],
        |row| row.get(0),
    ).map_err(|error| format!("Cannot verify summarized chunk count: {error}"))?;

    if actual_count == 0 || actual_count != source_chunk_count {
        return Err(format!(
            "Summary does not cover the complete document: expected {actual_count} chunks, received {source_chunk_count}"
        ));
    }

    transaction.execute(
        "INSERT INTO document_summaries (
            document_id, summary, model_name, summarizer_version,
            status, source_chunk_count, error_message, created_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, 'completed', ?5, NULL, ?6, ?6)
         ON CONFLICT(document_id)
         DO UPDATE SET
            summary = excluded.summary,
            model_name = excluded.model_name,
            summarizer_version = excluded.summarizer_version,
            status = 'completed',
            source_chunk_count = excluded.source_chunk_count,
            error_message = NULL,
            updated_at = excluded.updated_at",
        params![
            document_id,
            summary,
            model_name,
            summarizer_version,
            source_chunk_count,
            timestamp
        ],
    ).map_err(|error| format!("Cannot save final document summary: {error}"))?;

    transaction.commit()
        .map_err(|error| format!("Cannot commit final document summary: {error}"))
}

#[tauri::command]
pub fn get_document_summary(
    state: State<'_, Mutex<Storage>>,
    document_id: String,
) -> Result<Option<DocumentSummaryRecord>, String> {
    if document_id.is_empty() || document_id.len() > 128 {
        return Err("Invalid document ID".into());
    }

    let storage = state
        .lock()
        .map_err(|_| "Storage lock is poisoned".to_string())?;

    storage.connection.query_row(
        "SELECT document_id, summary, model_name, summarizer_version,
                status, source_chunk_count, updated_at
         FROM document_summaries
         WHERE document_id = ?1",
        [&document_id],
        |row| {
            Ok(DocumentSummaryRecord {
                document_id: row.get(0)?,
                summary: row.get(1)?,
                model_name: row.get(2)?,
                summarizer_version: row.get(3)?,
                status: row.get(4)?,
                source_chunk_count: row.get(5)?,
                updated_at: row.get(6)?,
            })
        },
    ).optional()
    .map_err(|error| format!("Cannot read document summary: {error}"))
}

