use std::{
    collections::{HashMap, HashSet},
    io::Cursor,
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
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
const MAX_ASSET_WARNINGS: usize = 100;
const MAX_ASSET_WARNING_LENGTH: usize = 512;
const PROCESSING_LEASE_MS: i64 = 120_000;

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
    attachment_version_id: &str,
    version_number: i64,
) -> Result<(String, i64), String> {
    if reference_id.is_empty()
        || reference_id.len() > 128
        || attachment_version_id.is_empty()
        || attachment_version_id.len() > 128
        || version_number < 1
    {
        return Err("Invalid attachment reference/version identity".into());
    }

    storage
        .connection
        .query_row(
            "SELECT v.blob_sha256, b.original_size
             FROM attachment_references r
             JOIN attachment_versions v ON v.id = r.attachment_version_id
             JOIN blobs b ON b.sha256 = v.blob_sha256
             WHERE r.id = ?1
               AND r.chat_id = ?2
               AND v.id = ?3
               AND v.version_number = ?4",
            params![reference_id, chat_id, attachment_version_id, version_number],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| format!("Cannot resolve exact attachment version: {error}"))?
        .ok_or_else(|| "Attachment reference does not match the specified chat and version".into())
}

fn resolve_message_blob(
    storage: &Storage,
    chat_id: &str,
    message_id: &str,
    reference_id: &str,
    attachment_version_id: &str,
    version_number: i64,
) -> Result<(String, i64), String> {
    if message_id.is_empty() || message_id.len() > 128 {
        return Err("Invalid message identity for attachment access".into());
    }

    let resolved = resolve_blob(
        storage,
        chat_id,
        reference_id,
        attachment_version_id,
        version_number,
    )?;

    let belongs_to_message: bool = storage
        .connection
        .query_row(
            "SELECT EXISTS (
                SELECT 1
                FROM attachment_references r
                JOIN messages m
                  ON m.id = r.message_id
                 AND m.chat_id = r.chat_id
                WHERE r.id = ?1
                  AND r.chat_id = ?2
                  AND r.attachment_version_id = ?3
                  AND r.message_id = ?4
                  AND r.purpose = 'message'
                  AND m.role = 'user'
            )",
            params![reference_id, chat_id, attachment_version_id, message_id],
            |row| row.get(0),
        )
        .map_err(|error| format!("Cannot validate message attachment ownership: {error}"))?;

    if !belongs_to_message {
        return Err("Attachment version is not attached to the specified user message".into());
    }

    Ok(resolved)
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

fn claim_document_processing(
    connection: &Connection,
    document_id: &str,
    sha256: &str,
    extractor: &str,
    extractor_version: &str,
    source_size: i64,
    processing_token: &str,
    timestamp: i64,
) -> Result<bool, String> {
    let lease_cutoff = timestamp.saturating_sub(PROCESSING_LEASE_MS);

    connection
        .execute(
            "INSERT INTO extracted_documents (
                id, blob_sha256, extractor, extractor_version, status,
                created_at, updated_at, source_size, processing_token
             ) VALUES (?1, ?2, ?3, ?4, 'processing', ?5, ?5, ?6, ?7)
             ON CONFLICT(blob_sha256, extractor, extractor_version)
             DO UPDATE SET
                status = 'processing',
                error_code = NULL,
                error_message = NULL,
                updated_at = excluded.updated_at,
                source_size = excluded.source_size,
                processing_token = excluded.processing_token
             WHERE extracted_documents.status <> 'completed'
               AND (
                    extracted_documents.status <> 'processing'
                    OR extracted_documents.processing_token IS NULL
                    OR extracted_documents.updated_at <= ?8
               )",
            params![
                document_id,
                sha256,
                extractor,
                extractor_version,
                timestamp,
                source_size,
                processing_token,
                lease_cutoff
            ],
        )
        .map(|changed| changed == 1)
        .map_err(|error| format!("Cannot claim document processing: {error}"))
}

fn validate_document_access(
    connection: &Connection,
    document_id: &str,
    processing_token: Option<&str>,
) -> Result<String, String> {
    let state: Option<(String, Option<String>)> = connection
        .query_row(
            "SELECT status, processing_token
             FROM extracted_documents
             WHERE id = ?1",
            [document_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| format!("Cannot validate document processing owner: {error}"))?;

    match state {
        Some((status, None)) if status == "completed" && processing_token.is_none() => Ok(status),
        Some((status, Some(owner)))
            if status == "processing" && processing_token == Some(owner.as_str()) =>
        {
            Ok(status)
        }
        Some(_) => Err("Document processing claim is no longer active".into()),
        None => Err("Document does not exist".into()),
    }
}

fn find_processing_document(
    connection: &Connection,
    sha256: &str,
    extractor: &str,
    extractor_version: &str,
    processing_token: &str,
) -> Result<String, String> {
    connection
        .query_row(
            "SELECT id FROM extracted_documents
             WHERE blob_sha256 = ?1
               AND extractor = ?2
               AND extractor_version = ?3
               AND status = 'processing'
               AND processing_token = ?4",
            params![sha256, extractor, extractor_version, processing_token],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("Cannot resolve owned processing document: {error}"))?
        .ok_or_else(|| "Document processing claim is no longer active".into())
}

fn renew_document_claim(
    connection: &Connection,
    document_id: &str,
    processing_token: &str,
    timestamp: i64,
) -> Result<bool, String> {
    connection
        .execute(
            "UPDATE extracted_documents
             SET updated_at = ?1
             WHERE id = ?2
               AND status = 'processing'
               AND processing_token = ?3",
            params![timestamp, document_id, processing_token],
        )
        .map(|changed| changed == 1)
        .map_err(|error| format!("Cannot renew document processing claim: {error}"))
}

pub fn recover_interrupted_document_processing(
    connection: &rusqlite::Connection,
) -> Result<usize, String> {
    let timestamp = now_timestamp()?;
    let lease_cutoff = timestamp.saturating_sub(PROCESSING_LEASE_MS);

    connection
        .execute(
            "UPDATE extracted_documents
             SET status = 'pending',
                 processing_token = NULL,
                 error_code = NULL,
                 error_message = NULL,
                 updated_at = ?1
             WHERE status = 'processing'
               AND updated_at <= ?2",
            params![timestamp, lease_cutoff],
        )
        .map_err(|error| format!("Cannot recover interrupted document processing: {error}"))
}

#[tauri::command]
pub fn get_attachment_processing_status(
    state: State<'_, Mutex<Storage>>,
    chat_id: String,
    reference_id: String,
    attachment_version_id: String,
    version_number: i64,
    extractor: String,
    extractor_version: String,
) -> Result<AttachmentProcessingStatus, String> {
    validate_extractor(&extractor, &extractor_version)?;

    let storage = state
        .lock()
        .map_err(|error| format!("Storage lock poisoned: {error}"))?;

    let (sha256, _) = resolve_blob(
        &storage,
        &chat_id,
        &reference_id,
        &attachment_version_id,
        version_number,
    )?;
    query_status(&storage, &sha256, &extractor, &extractor_version)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BeginAttachmentProcessingResult {
    pub status: AttachmentProcessingStatus,
    pub claim_token: Option<String>,
}

#[tauri::command]
pub fn begin_attachment_processing(
    state: State<'_, Mutex<Storage>>,
    chat_id: String,
    reference_id: String,
    attachment_version_id: String,
    version_number: i64,
    extractor: String,
    extractor_version: String,
) -> Result<BeginAttachmentProcessingResult, String> {
    validate_extractor(&extractor, &extractor_version)?;

    let storage = state
        .lock()
        .map_err(|error| format!("Storage lock poisoned: {error}"))?;

    let (sha256, source_size) = resolve_blob(
        &storage,
        &chat_id,
        &reference_id,
        &attachment_version_id,
        version_number,
    )?;
    let timestamp = now_timestamp()?;
    let document_id = Uuid::new_v4().to_string();
    let claim_token = Uuid::new_v4().to_string();

    let claimed = claim_document_processing(
        &storage.connection,
        &document_id,
        &sha256,
        &extractor,
        &extractor_version,
        source_size,
        &claim_token,
        timestamp,
    )?;

    let status = query_status(&storage, &sha256, &extractor, &extractor_version)?;

    Ok(BeginAttachmentProcessingResult {
        status,
        claim_token: claimed.then_some(claim_token),
    })
}

#[tauri::command]
pub fn renew_attachment_processing_claim(
    state: State<'_, Mutex<Storage>>,
    document_id: String,
    processing_token: String,
) -> Result<bool, String> {
    if document_id.is_empty()
        || document_id.len() > 128
        || processing_token.is_empty()
        || processing_token.len() > 128
    {
        return Err("Invalid document processing claim".into());
    }

    let storage = state
        .lock()
        .map_err(|error| format!("Storage lock poisoned: {error}"))?;

    renew_document_claim(
        &storage.connection,
        &document_id,
        &processing_token,
        now_timestamp()?,
    )
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
    pub attachment_version_id: String,
    pub version_number: i64,
    pub extractor: String,
    pub extractor_version: String,
    pub processing_token: String,
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

    if input.processing_token.is_empty() || input.processing_token.len() > 128 {
        return Err("Invalid document processing claim".into());
    }

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

    validate_source_document_metadata(&input.metadata)?;

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

fn bounded_asset_inputs(assets: &[ExtractedAssetInput]) -> (&[ExtractedAssetInput], usize) {
    let accepted_count = assets.len().min(MAX_DOCUMENT_ASSETS);
    (&assets[..accepted_count], assets.len() - accepted_count)
}

fn validate_asset(asset: &ExtractedAssetInput, section_keys: &HashSet<&str>) -> Result<(), String> {
    if asset.asset_type != "image" && asset.asset_type != "chart" {
        return Err(format!(
            "Unsupported document asset type: {}",
            asset.asset_type
        ));
    }
    if asset.ordinal < 0 {
        return Err("Asset ordinal cannot be negative".into());
    }
    if asset.data.is_empty() {
        return Err("Embedded asset is empty".into());
    }
    if asset.data.len() > MAX_ASSET_BYTES {
        return Err("Embedded asset exceeds the 16 MiB limit".into());
    }
    if asset
        .mime_type
        .as_ref()
        .is_some_and(|value| value.is_empty() || value.len() > 128)
    {
        return Err("Invalid embedded asset MIME type".into());
    }
    if asset
        .file_name
        .as_ref()
        .is_some_and(|value| value.len() > 512)
    {
        return Err("Embedded asset filename is too long".into());
    }
    if let Some(key) = &asset.section_key {
        if !section_keys.contains(key.as_str()) {
            return Err(format!("Unknown asset section key: {key}"));
        }
    }
    validate_json_size(&asset.metadata, "Asset metadata")?;
    Ok(())
}

fn push_asset_warning(warnings: &mut Vec<String>, omitted: &mut usize, message: impl AsRef<str>) {
    if warnings.len() < MAX_ASSET_WARNINGS {
        warnings.push(
            message
                .as_ref()
                .chars()
                .take(MAX_ASSET_WARNING_LENGTH)
                .collect(),
        );
    } else {
        *omitted = omitted.saturating_add(1);
    }
}

fn collect_input_asset_warnings(
    metadata: &Option<Value>,
    warnings: &mut Vec<String>,
    omitted: &mut usize,
) {
    let Some(Value::Object(object)) = metadata else {
        return;
    };

    if let Some(Value::Array(values)) = object.get("assetExtractionWarnings") {
        for value in values {
            if let Some(message) = value.as_str() {
                push_asset_warning(warnings, omitted, message);
            }
        }
    }

    if let Some(count) = object
        .get("assetExtractionWarningsOmitted")
        .and_then(Value::as_u64)
    {
        let count = usize::try_from(count).unwrap_or(usize::MAX);
        *omitted = omitted.saturating_add(count);
    }
}

fn metadata_without_asset_diagnostics(metadata: &Option<Value>) -> Option<Value> {
    let Some(Value::Object(object)) = metadata else {
        return metadata.clone();
    };

    let mut source = object.clone();
    source.remove("assetExtractionWarnings");
    source.remove("assetExtractionWarningsOmitted");
    Some(Value::Object(source))
}

fn validate_source_document_metadata(metadata: &Option<Value>) -> Result<(), String> {
    let source_metadata = metadata_without_asset_diagnostics(metadata);
    validate_json_size(&source_metadata, "Document metadata").map(|_| ())
}

fn build_document_metadata(
    metadata: Option<Value>,
    warnings: &[String],
    omitted: usize,
) -> Result<Option<String>, String> {
    // Extraction diagnostics are bounded separately and must never make valid
    // source metadata fail its own 256 KiB document metadata limit.
    let original_metadata = metadata_without_asset_diagnostics(&metadata);

    if warnings.is_empty() && omitted == 0 {
        return validate_json_size(&original_metadata, "Document metadata");
    }

    let base = match &original_metadata {
        Some(Value::Object(object)) => Value::Object(object.clone()),
        Some(value) => json!({ "sourceMetadata": value }),
        None => Value::Object(serde_json::Map::new()),
    };

    let mut retained = warnings.len();
    loop {
        let mut candidate = base.clone();
        if let Some(object) = candidate.as_object_mut() {
            if retained > 0 {
                object.insert(
                    "assetExtractionWarnings".into(),
                    json!(&warnings[..retained]),
                );
            }

            let omitted_count = omitted.saturating_add(warnings.len() - retained);
            if omitted_count > 0 {
                object.insert(
                    "assetExtractionWarningsOmitted".into(),
                    json!(omitted_count),
                );
            }
        }

        let serialized = serde_json::to_string(&candidate)
            .map_err(|error| format!("Cannot serialize document metadata: {error}"))?;
        if serialized.len() <= MAX_METADATA_BYTES {
            return Ok(Some(serialized));
        }

        if retained == 0 {
            break;
        }
        retained -= 1;
    }

    // If diagnostics cannot fit, preserve the original source metadata. The
    // complete bounded diagnostics remain in the batch journal and response.
    validate_json_size(&original_metadata, "Document metadata")
}

fn rollback_document_asset_batch(
    storage: &mut Storage,
    batch_operation_id: &str,
    newly_stored_hashes: &[String],
    replaced_asset_hashes: &[String],
    cause: &str,
    timestamp: i64,
) -> Vec<String> {
    let safe_error: String = cause.chars().take(2_000).collect();
    let _ = storage.connection.execute(
        "UPDATE storage_operations
         SET state = 'FAILED', error_message = ?1, updated_at = ?2
         WHERE id = ?3 AND operation_type = 'document_assets_batch'
           AND state <> 'COMPLETED'",
        params![
            safe_error,
            now_timestamp().unwrap_or(timestamp),
            batch_operation_id
        ],
    );

    let mut hashes: HashSet<String> = newly_stored_hashes.iter().cloned().collect();
    hashes.extend(replaced_asset_hashes.iter().cloned());
    let mut cleanup_errors = Vec::new();
    match super::blobs::collect_child_blob_hashes(&storage.connection, batch_operation_id) {
        Ok(child_hashes) => hashes.extend(child_hashes),
        Err(error) => cleanup_errors.push(format!(
            "Cannot inspect document asset child journals: {error}"
        )),
    }

    for hash in hashes {
        if let Err(cleanup_error) = storage.delete_unreferenced_blob(&hash) {
            cleanup_errors.push(format!("{hash}: {cleanup_error}"));
        }
    }

    if cleanup_errors.is_empty() {
        let _ = storage.connection.execute(
            "UPDATE storage_operations
             SET state = 'COMPLETED',
                 error_message = 'Document transaction rolled back; unreferenced asset cleanup completed',
                 updated_at = ?1
             WHERE id = ?2 AND operation_type = 'document_assets_batch'
               AND state <> 'COMPLETED'",
            params![now_timestamp().unwrap_or(timestamp), batch_operation_id],
        );
    } else {
        let diagnostic = format!(
            "{safe_error}; asset cleanup deferred for startup recovery: {}",
            cleanup_errors.join("; ")
        );
        let _ = storage.connection.execute(
            "UPDATE storage_operations
             SET state = 'FAILED', error_message = ?1, updated_at = ?2
             WHERE id = ?3 AND operation_type = 'document_assets_batch'
               AND state <> 'COMPLETED'",
            params![
                diagnostic.chars().take(2_000).collect::<String>(),
                now_timestamp().unwrap_or(timestamp),
                batch_operation_id
            ],
        );
    }

    cleanup_errors
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedDocumentRecord {
    pub document_id: String,
    pub status: String,
    pub section_count: usize,
    pub chunk_count: usize,
    pub text_bytes: usize,
    pub asset_count: usize,
    pub asset_warning_count: usize,
    pub asset_warnings: Vec<String>,
    pub asset_warnings_omitted: usize,
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

    let (sha256, actual_source_size) = resolve_blob(
        &storage,
        &input.chat_id,
        &input.reference_id,
        &input.attachment_version_id,
        input.version_number,
    )?;
    if actual_source_size != input.source_size {
        return Err("Document source size does not match the stored attachment".into());
    }

    let document_id = find_processing_document(
        &storage.connection,
        &sha256,
        &input.extractor,
        &input.extractor_version,
        &input.processing_token,
    )?;

    let mut old_asset_statement = storage
        .connection
        .prepare(
            "SELECT DISTINCT blob_sha256 FROM document_assets
             WHERE document_id = ?1 AND blob_sha256 IS NOT NULL",
        )
        .map_err(|error| format!("Cannot inspect previous document assets: {error}"))?;
    let old_asset_rows = old_asset_statement
        .query_map([&document_id], |row| row.get::<_, String>(0))
        .map_err(|error| format!("Cannot query previous document assets: {error}"))?;
    let replaced_asset_hashes = old_asset_rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("Cannot read previous document asset hashes: {error}"))?;
    drop(old_asset_statement);

    let timestamp = now_timestamp()?;
    let batch_operation_id = Uuid::new_v4().to_string();
    let batch_metadata = json!({
        "documentId": &document_id,
        "extractor": &input.extractor,
        "extractorVersion": &input.extractor_version,
        "expectedAssetCount": input.assets.len(),
        "replacedAssetHashes": &replaced_asset_hashes
    })
    .to_string();

    storage
        .connection
        .execute(
            "INSERT INTO storage_operations (
                id, operation_type, state, metadata_json, created_at, updated_at
             ) VALUES (?1, 'document_assets_batch', 'PROCESSING', ?2, ?3, ?3)",
            params![batch_operation_id, batch_metadata, timestamp],
        )
        .map_err(|error| format!("Cannot create document asset batch journal: {error}"))?;

    let mut asset_warnings = Vec::new();
    let mut omitted_asset_warnings = 0usize;
    collect_input_asset_warnings(
        &input.metadata,
        &mut asset_warnings,
        &mut omitted_asset_warnings,
    );

    let section_keys: HashSet<&str> = input
        .sections
        .iter()
        .map(|section| section.key.as_str())
        .collect();

    let mut stored_assets: Vec<(&ExtractedAssetInput, String)> = Vec::new();
    let mut total_asset_bytes = 0usize;

    let (assets_to_process, skipped_asset_count) = bounded_asset_inputs(&input.assets);
    if skipped_asset_count > 0 {
        push_asset_warning(
            &mut asset_warnings,
            &mut omitted_asset_warnings,
            format!(
                "Skipped {skipped_asset_count} assets because the document limit is {MAX_DOCUMENT_ASSETS}"
            ),
        );
    }

    for (index, asset) in assets_to_process.iter().enumerate() {
        if let Err(error) = validate_asset(asset, &section_keys) {
            let name = asset
                .file_name
                .as_deref()
                .filter(|name| !name.is_empty())
                .unwrap_or("unnamed asset");
            push_asset_warning(
                &mut asset_warnings,
                &mut omitted_asset_warnings,
                format!("Asset #{} ({name}) was skipped: {error}", index + 1),
            );
            continue;
        }

        let Some(next_total) = total_asset_bytes.checked_add(asset.data.len()) else {
            push_asset_warning(
                &mut asset_warnings,
                &mut omitted_asset_warnings,
                format!(
                    "Asset #{} was skipped: aggregate asset size overflow",
                    index + 1
                ),
            );
            continue;
        };

        if next_total > MAX_DOCUMENT_ASSET_BYTES {
            push_asset_warning(
                &mut asset_warnings,
                &mut omitted_asset_warnings,
                format!(
                    "Asset #{} was skipped: the 64 MiB aggregate asset limit would be exceeded",
                    index + 1
                ),
            );
            continue;
        }

        match storage.store_blob_with_parent(
            Cursor::new(asset.data.as_slice()),
            MAX_ASSET_BYTES as u64,
            Some(&batch_operation_id),
        ) {
            Ok(blob) => {
                total_asset_bytes = next_total;
                stored_assets.push((asset, blob.sha256));
            }
            Err(error) => {
                let name = asset
                    .file_name
                    .as_deref()
                    .filter(|name| !name.is_empty())
                    .unwrap_or("unnamed asset");
                push_asset_warning(
                    &mut asset_warnings,
                    &mut omitted_asset_warnings,
                    format!(
                        "Asset #{} ({name}) could not be stored and was skipped: {error}",
                        index + 1
                    ),
                );
            }
        }
    }

    let transaction_result = (|| -> Result<SavedDocumentRecord, String> {
        let document_metadata = build_document_metadata(
            input.metadata.clone(),
            &asset_warnings,
            omitted_asset_warnings,
        )?;

        // Keep the complete bounded diagnostics in the durable batch journal
        // even when the document's source metadata has no remaining capacity.
        let mut journal_metadata = serde_json::from_str::<Value>(&batch_metadata)
            .map_err(|error| format!("Cannot prepare document asset journal metadata: {error}"))?;
        if let Some(object) = journal_metadata.as_object_mut() {
            object.insert("assetWarnings".into(), json!(&asset_warnings));
            object.insert("assetWarningsOmitted".into(), json!(omitted_asset_warnings));
        }
        let journal_metadata = serde_json::to_string(&journal_metadata).map_err(|error| {
            format!("Cannot serialize document asset journal metadata: {error}")
        })?;

        let transaction = storage
            .connection
            .transaction()
            .map_err(|error| format!("Cannot begin document transaction: {error}"))?;

        let resolved_document_id = find_processing_document(
            &transaction,
            &sha256,
            &input.extractor,
            &input.extractor_version,
            &input.processing_token,
        )?;

        if resolved_document_id != document_id {
            return Err("Document processing identity changed during asset persistence".into());
        }

        let metadata_changed = transaction
            .execute(
                "UPDATE extracted_documents
                 SET language = ?1,
                     updated_at = ?2,
                     detected_format = ?3,
                     metadata_json = ?4,
                     error_code = NULL,
                     error_message = NULL,
                     processed_at = NULL,
                     source_size = ?5
                 WHERE id = ?6
                   AND status = 'processing'
                   AND processing_token = ?7",
                params![
                    input.language,
                    timestamp,
                    input.detected_format,
                    document_metadata,
                    input.source_size,
                    resolved_document_id,
                    input.processing_token
                ],
            )
            .map_err(|error| format!("Cannot update owned extracted document: {error}"))?;

        if metadata_changed != 1 {
            return Err("Document processing claim is no longer active".into());
        }

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

        let completed = transaction
            .execute(
                "UPDATE storage_operations
                 SET state = 'COMMITTING', metadata_json = ?1,
                     error_message = NULL, updated_at = ?2
                 WHERE id = ?3
                   AND operation_type = 'document_assets_batch'
                   AND state = 'PROCESSING'",
                params![journal_metadata, timestamp, batch_operation_id],
            )
            .map_err(|error| format!("Cannot complete document asset batch journal: {error}"))?;

        if completed != 1 {
            return Err("Document asset batch journal is no longer active".into());
        }

        let record = SavedDocumentRecord {
            document_id: resolved_document_id,
            status: "processing".into(),
            section_count: input.sections.len(),
            chunk_count: input.chunks.len(),
            text_bytes: total_text_bytes,
            asset_count: stored_assets.len(),
            asset_warning_count: asset_warnings.len().saturating_add(omitted_asset_warnings),
            asset_warnings: asset_warnings.clone(),
            asset_warnings_omitted: omitted_asset_warnings,
        };

        transaction
            .commit()
            .map_err(|error| format!("Cannot commit extracted document and assets: {error}"))?;

        Ok(record)
    })();

    match transaction_result {
        Ok(mut record) => {
            // The new document references and COMMITTING journal state are now
            // durable together. Old asset blobs are retired only after this
            // commit; recovery can finish the cleanup if the process exits here.
            let mut cleanup_hashes: HashSet<String> =
                replaced_asset_hashes.iter().cloned().collect();
            let mut cleanup_errors = Vec::new();
            match super::blobs::collect_child_blob_hashes(&storage.connection, &batch_operation_id)
            {
                Ok(child_hashes) => cleanup_hashes.extend(child_hashes),
                Err(error) => cleanup_errors.push(format!(
                    "Cannot inspect document asset child journals: {error}"
                )),
            }

            for hash in cleanup_hashes {
                if let Err(cleanup_error) = storage.delete_unreferenced_blob(&hash) {
                    cleanup_errors.push(format!("{hash}: {cleanup_error}"));
                }
            }

            let journal_result = if cleanup_errors.is_empty() {
                storage.connection.execute(
                    "UPDATE storage_operations
                     SET state = 'COMPLETED', error_message = NULL, updated_at = ?1
                     WHERE id = ?2 AND operation_type = 'document_assets_batch'
                       AND state = 'COMMITTING'",
                    params![now_timestamp()?, batch_operation_id],
                )
            } else {
                let message = format!(
                    "Replaced asset cleanup deferred for startup recovery: {}",
                    cleanup_errors.join("; ")
                );
                push_asset_warning(
                    &mut record.asset_warnings,
                    &mut record.asset_warnings_omitted,
                    &message,
                );
                storage.connection.execute(
                    "UPDATE storage_operations
                     SET state = 'FAILED', error_message = ?1, updated_at = ?2
                     WHERE id = ?3 AND operation_type = 'document_assets_batch'
                       AND state = 'COMMITTING'",
                    params![
                        message.chars().take(2_000).collect::<String>(),
                        now_timestamp()?,
                        batch_operation_id
                    ],
                )
            };

            match journal_result {
                Ok(1) => {}
                Ok(_) => push_asset_warning(
                    &mut record.asset_warnings,
                    &mut record.asset_warnings_omitted,
                    "Asset cleanup journal changed unexpectedly; startup recovery will verify it.",
                ),
                Err(error) => push_asset_warning(
                    &mut record.asset_warnings,
                    &mut record.asset_warnings_omitted,
                    format!("Asset cleanup journal update failed: {error}"),
                ),
            }

            record.asset_warning_count = record
                .asset_warnings
                .len()
                .saturating_add(record.asset_warnings_omitted);
            Ok(record)
        }
        Err(error) => {
            let newly_stored_hashes: Vec<String> =
                stored_assets.iter().map(|(_, hash)| hash.clone()).collect();
            let _cleanup_errors = rollback_document_asset_batch(
                &mut storage,
                &batch_operation_id,
                &newly_stored_hashes,
                &replaced_asset_hashes,
                &error,
                timestamp,
            );
            Err(error)
        }
    }
}

#[tauri::command]
pub fn mark_attachment_processing_failed(
    state: State<'_, Mutex<Storage>>,
    chat_id: String,
    reference_id: String,
    attachment_version_id: String,
    version_number: i64,
    extractor: String,
    extractor_version: String,
    processing_token: String,
    error_code: String,
    error_message: String,
) -> Result<AttachmentProcessingStatus, String> {
    validate_extractor(&extractor, &extractor_version)?;

    if processing_token.is_empty() || processing_token.len() > 128 {
        return Err("Invalid document processing claim".into());
    }
    if error_code.is_empty() || error_code.len() > 64 {
        return Err("Invalid processing error code".into());
    }

    let safe_message: String = error_message.chars().take(2_000).collect();
    let storage = state
        .lock()
        .map_err(|error| format!("Storage lock poisoned: {error}"))?;

    let (sha256, source_size) = resolve_blob(
        &storage,
        &chat_id,
        &reference_id,
        &attachment_version_id,
        version_number,
    )?;
    let timestamp = now_timestamp()?;

    let changed = storage
        .connection
        .execute(
            "UPDATE extracted_documents
             SET status = 'failed',
                 processing_token = NULL,
                 error_code = ?1,
                 error_message = ?2,
                 updated_at = ?3,
                 processed_at = ?3,
                 source_size = ?4
             WHERE blob_sha256 = ?5
               AND extractor = ?6
               AND extractor_version = ?7
               AND status = 'processing'
               AND processing_token = ?8",
            params![
                error_code,
                safe_message,
                timestamp,
                source_size,
                sha256,
                extractor,
                extractor_version,
                processing_token
            ],
        )
        .map_err(|error| format!("Cannot save document processing failure: {error}"))?;

    if changed != 1 {
        return Err("Document processing claim is no longer active".into());
    }

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
    message_id: String,
    reference_id: String,
    attachment_version_id: String,
    version_number: i64,
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

    let (sha256, _) = resolve_message_blob(
        &storage,
        &chat_id,
        &message_id,
        &reference_id,
        &attachment_version_id,
        version_number,
    )?;

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
             JOIN messages m ON m.id = r.message_id AND m.chat_id = r.chat_id
             WHERE d.blob_sha256 = ?1
               AND d.extractor = ?2
               AND d.extractor_version = ?3
               AND d.status = 'completed'
               AND r.id = ?4
               AND r.chat_id = ?5
               AND v.id = ?6
               AND v.version_number = ?7
               AND r.message_id = ?8
               AND r.purpose = 'message'
               AND m.role = 'user'
             LIMIT 1",
            params![
                sha256,
                extractor,
                extractor_version,
                reference_id,
                chat_id,
                attachment_version_id,
                version_number,
                message_id
            ],
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
        .map_err(|error| format!("Cannot resolve exact processed attachment: {error}"))?;

    let (document_id, file_name, detected_format, language, metadata_json) =
        document.ok_or_else(|| {
            "Attachment has no completed extraction for this message/version".to_string()
        })?;

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
pub struct AttachmentDocumentAsset {
    pub asset_id: String,
    pub asset_type: String,
    pub mime_type: Option<String>,
    pub file_name: Option<String>,
    pub ordinal: i64,
    pub metadata_json: Option<String>,
    pub document_id: String,
    pub section_title: Option<String>,
}

#[tauri::command]
pub fn list_attachment_document_assets(
    state: State<'_, Mutex<Storage>>,
    chat_id: String,
    message_id: String,
    reference_id: String,
    attachment_version_id: String,
    version_number: i64,
    extractor: String,
    extractor_version: String,
) -> Result<Vec<AttachmentDocumentAsset>, String> {
    validate_extractor(&extractor, &extractor_version)?;

    let storage = state
        .lock()
        .map_err(|error| format!("Storage lock poisoned: {error}"))?;
    let (sha256, _) = resolve_message_blob(
        &storage,
        &chat_id,
        &message_id,
        &reference_id,
        &attachment_version_id,
        version_number,
    )?;
    let mut statement = storage
        .connection
        .prepare(
            "SELECT a.id, a.asset_type, a.mime_type, a.file_name, a.ordinal,
                a.metadata_json, d.id, s.title
             FROM document_assets a
             JOIN extracted_documents d ON d.id = a.document_id
             LEFT JOIN document_sections s ON s.id = a.section_id
             WHERE d.blob_sha256 = ?1
               AND d.extractor = ?2
               AND d.extractor_version = ?3
               AND d.status = 'completed'
             ORDER BY a.ordinal ASC",
        )
        .map_err(|error| format!("Cannot prepare document asset query: {error}"))?;
    let rows = statement
        .query_map(params![sha256, extractor, extractor_version], |row| {
            Ok(AttachmentDocumentAsset {
                asset_id: row.get(0)?,
                asset_type: row.get(1)?,
                mime_type: row.get(2)?,
                file_name: row.get(3)?,
                ordinal: row.get(4)?,
                metadata_json: row.get(5)?,
                document_id: row.get(6)?,
                section_title: row.get(7)?,
            })
        })
        .map_err(|error| format!("Cannot query document assets: {error}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("Cannot read document asset metadata: {error}"))
}

#[tauri::command]
pub fn read_attachment_document_asset(
    state: State<'_, Mutex<Storage>>,
    chat_id: String,
    message_id: String,
    reference_id: String,
    attachment_version_id: String,
    version_number: i64,
    extractor: String,
    extractor_version: String,
    asset_id: String,
) -> Result<Vec<u8>, String> {
    validate_extractor(&extractor, &extractor_version)?;
    let storage = state
        .lock()
        .map_err(|error| format!("Storage lock poisoned: {error}"))?;
    let (sha256, _) = resolve_message_blob(
        &storage,
        &chat_id,
        &message_id,
        &reference_id,
        &attachment_version_id,
        version_number,
    )?;
    let relative_path: String = storage
        .connection
        .query_row(
            "SELECT b.relative_path
             FROM document_assets a
             JOIN extracted_documents d ON d.id = a.document_id
             JOIN blobs b ON b.sha256 = a.blob_sha256
             WHERE a.id = ?1
               AND d.blob_sha256 = ?2
               AND d.extractor = ?3
               AND d.extractor_version = ?4
               AND d.status = 'completed'",
            params![asset_id, sha256, extractor, extractor_version],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("Cannot resolve document asset: {error}"))?
        .ok_or_else(|| {
            "Document asset does not belong to the exact attachment version".to_string()
        })?;
    let path = storage.resolve_blob_path(&relative_path)?;
    let metadata = std::fs::metadata(&path)
        .map_err(|error| format!("Cannot inspect document asset: {error}"))?;
    if metadata.len() == 0 || metadata.len() > MAX_ASSET_BYTES as u64 {
        return Err("Document asset is empty or exceeds the 16 MiB limit".into());
    }
    std::fs::read(path).map_err(|error| format!("Cannot read document asset: {error}"))
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedVisionDescription {
    pub document_id: String,
    pub chunk_id: String,
}

#[tauri::command]
pub fn save_attachment_document_asset_vision_description(
    state: State<'_, Mutex<Storage>>,
    chat_id: String,
    message_id: String,
    reference_id: String,
    attachment_version_id: String,
    version_number: i64,
    extractor: String,
    extractor_version: String,
    asset_id: String,
    description: String,
    model_name: Option<String>,
    prompt_version: Option<i64>,
) -> Result<SavedVisionDescription, String> {
    validate_extractor(&extractor, &extractor_version)?;
    let description = description.trim();
    if description.is_empty() || description.len() > 32 * 1024 {
        return Err("Vision description is empty or exceeds the 32 KiB limit".into());
    }
    if asset_id.is_empty() || asset_id.len() > 128 {
        return Err("Invalid document asset ID".into());
    }

    let model_name = model_name.unwrap_or_else(|| "sonne-gemma:latest".into());
    if model_name.is_empty() || model_name.len() > 128 {
        return Err("Invalid Vision model name".into());
    }
    let prompt_version = prompt_version.unwrap_or(2);
    if prompt_version < 1 || prompt_version > 1000 {
        return Err("Invalid Vision prompt version".into());
    }

    let mut storage = state
        .lock()
        .map_err(|error| format!("Storage lock poisoned: {error}"))?;
    let (sha256, _) = resolve_message_blob(
        &storage,
        &chat_id,
        &message_id,
        &reference_id,
        &attachment_version_id,
        version_number,
    )?;
    let timestamp = now_timestamp()?;
    let transaction = storage
        .connection
        .transaction()
        .map_err(|error| format!("Cannot begin Vision metadata transaction: {error}"))?;

    let (document_id, section_id, file_name, ordinal, metadata_json): (
        String,
        Option<String>,
        Option<String>,
        i64,
        Option<String>,
    ) = transaction
        .query_row(
            "SELECT d.id, a.section_id, a.file_name, a.ordinal, a.metadata_json
             FROM document_assets a
             JOIN extracted_documents d ON d.id = a.document_id
             WHERE a.id = ?1 AND d.blob_sha256 = ?2
               AND d.extractor = ?3 AND d.extractor_version = ?4
               AND d.status = 'completed' AND a.asset_type = 'image'",
            params![asset_id, sha256, extractor, extractor_version],
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
        .map_err(|error| format!("Cannot resolve Vision asset: {error}"))?
        .ok_or_else(|| "Image asset does not belong to the specified attachment".to_string())?;

    let mut metadata: Value = metadata_json
        .as_deref()
        .and_then(|raw| serde_json::from_str(raw).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| Value::Object(serde_json::Map::new()));
    let metadata_object = metadata
        .as_object_mut()
        .ok_or_else(|| "Image asset metadata is not a JSON object".to_string())?;

    // Find the original image-index chunks by their stable ordinal/name metadata.
    // Replace them instead of leaving stale Vision descriptions searchable.
    let mut statement = transaction
        .prepare(
            "SELECT id, metadata_json FROM document_chunks
             WHERE document_id = ?1 AND chunk_type IN ('image_ocr', 'asset_vision')",
        )
        .map_err(|error| format!("Cannot prepare existing image chunk query: {error}"))?;
    let rows = statement
        .query_map([&document_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })
        .map_err(|error| format!("Cannot query existing image chunks: {error}"))?;
    let candidate_chunks = rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("Cannot collect existing image chunks: {error}"))?;
    drop(statement);

    let mut matching_chunk_ids = Vec::new();
    for (candidate_id, candidate_metadata) in candidate_chunks {
        let Some(candidate_metadata) = candidate_metadata
            .as_deref()
            .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
        else {
            continue;
        };
        let Some(candidate_object) = candidate_metadata.as_object() else {
            continue;
        };
        let same_ordinal =
            candidate_object.get("assetOrdinal").and_then(Value::as_i64) == Some(ordinal);
        let same_name =
            candidate_object.get("fileName").and_then(Value::as_str) == file_name.as_deref();
        let source_matches = matches!(
            candidate_object.get("source").and_then(Value::as_str),
            Some("embedded_asset") | Some("embedded_asset_vision")
        );
        if same_ordinal && same_name && source_matches {
            matching_chunk_ids.push(candidate_id);
        }
    }

    let stored_chunk_id = metadata_object
        .get("visionChunkId")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty() && value.len() <= 128)
        .map(str::to_owned);
    let chunk_id = stored_chunk_id
        .filter(|stored| {
            matching_chunk_ids
                .iter()
                .any(|candidate| candidate == stored)
        })
        .or_else(|| matching_chunk_ids.first().cloned())
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    // Remove stale FTS/vector entries before replacing the source chunk.
    for old_chunk_id in &matching_chunk_ids {
        transaction
            .execute(
                "DELETE FROM document_vectors_nomic WHERE chunk_id = ?1",
                [old_chunk_id],
            )
            .map_err(|error| format!("Cannot remove stale image vector: {error}"))?;
        transaction
            .execute(
                "DELETE FROM document_chunks WHERE id = ?1 AND document_id = ?2",
                params![old_chunk_id, document_id],
            )
            .map_err(|error| format!("Cannot remove stale image search chunk: {error}"))?;
    }

    let alt_text = metadata_object
        .get("altText")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned);
    let ocr_text = metadata_object
        .get("ocrText")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned);

    metadata_object.insert(
        "visionDescription".into(),
        Value::String(description.to_string()),
    );
    metadata_object.insert("visionModel".into(), Value::String(model_name.clone()));
    metadata_object.insert("visionPromptVersion".into(), Value::from(prompt_version));
    metadata_object.insert("visionAnalyzedAt".into(), Value::from(timestamp));
    metadata_object.insert("visionChunkId".into(), Value::String(chunk_id.clone()));

    let serialized_metadata = serde_json::to_string(&metadata)
        .map_err(|error| format!("Cannot serialize Vision metadata: {error}"))?;
    if serialized_metadata.len() > MAX_METADATA_BYTES {
        return Err("Updated image metadata exceeds the metadata size limit".into());
    }

    let display_name = file_name
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| format!("зображення {}", ordinal + 1));
    let mut chunk_lines = vec![
        "Embedded image".to_string(),
        format!("File name: {display_name}"),
        format!("Vision description: {description}"),
    ];
    if let Some(alt_text) = alt_text {
        chunk_lines.push(format!("Description: {alt_text}"));
    }
    if let Some(ocr_text) = ocr_text {
        chunk_lines.push(format!("OCR text:\n{ocr_text}"));
    }
    let mut chunk_content = chunk_lines.join("\n");
    if chunk_content.len() > MAX_CHUNK_BYTES {
        let mut boundary = MAX_CHUNK_BYTES;
        while !chunk_content.is_char_boundary(boundary) {
            boundary -= 1;
        }
        chunk_content.truncate(boundary);
    }
    let chunk_metadata = serde_json::json!({
        "source": "embedded_asset_vision",
        "assetId": asset_id.clone(),
        "assetOrdinal": ordinal,
        "fileName": file_name,
        "visionModel": model_name,
        "visionPromptVersion": prompt_version
    })
    .to_string();

    let next_ordinal: i64 = transaction
        .query_row(
            "SELECT COALESCE(MAX(ordinal), -1) + 1 FROM document_chunks WHERE document_id = ?1",
            [&document_id],
            |row| row.get(0),
        )
        .map_err(|error| format!("Cannot allocate Vision search chunk order: {error}"))?;
    transaction
        .execute(
            "INSERT INTO document_chunks (
            id, document_id, ordinal, content, token_count,
            start_offset, end_offset, section_id, chunk_type, metadata_json
         ) VALUES (?1, ?2, ?3, ?4, ?5, NULL, NULL, ?6, 'asset_vision', ?7)",
            params![
                chunk_id,
                document_id,
                next_ordinal,
                chunk_content,
                (chunk_content.len() / 4) as i64,
                section_id,
                chunk_metadata
            ],
        )
        .map_err(|error| format!("Cannot insert Vision search chunk: {error}"))?;

    transaction
        .execute(
            "UPDATE document_assets SET metadata_json = ?1 WHERE id = ?2",
            params![serialized_metadata, asset_id],
        )
        .map_err(|error| format!("Cannot persist Vision asset metadata: {error}"))?;

    // Document summaries are derived from document_chunks. Invalidate every
    // cached Map-Reduce stage so later requests cannot reuse stale image facts.
    transaction
        .execute(
            "DELETE FROM document_summary_parts WHERE document_id = ?1",
            [&document_id],
        )
        .map_err(|error| format!("Cannot invalidate Vision summary parts: {error}"))?;
    transaction
        .execute(
            "DELETE FROM document_summaries WHERE document_id = ?1",
            [&document_id],
        )
        .map_err(|error| format!("Cannot invalidate Vision document summary: {error}"))?;

    transaction
        .commit()
        .map_err(|error| format!("Cannot commit Vision metadata transaction: {error}"))?;

    Ok(SavedVisionDescription {
        document_id,
        chunk_id,
    })
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentSearchResult {
    pub chunk_id: String,
    pub reference_id: String,
    pub attachment_version_id: String,
    pub version_number: i64,
    pub file_name: String,
    pub document_id: String,
    pub section_id: Option<String>,
    pub section_title: Option<String>,
    pub ordinal: i64,
    pub chunk_type: String,
    pub detected_format: Option<String>,
    pub content: String,
}

fn map_document_search_result(row: &rusqlite::Row<'_>) -> rusqlite::Result<DocumentSearchResult> {
    Ok(DocumentSearchResult {
        chunk_id: row.get(0)?,
        reference_id: row.get(1)?,
        attachment_version_id: row.get(2)?,
        version_number: row.get(3)?,
        file_name: row.get(4)?,
        document_id: row.get(5)?,
        section_id: row.get(6)?,
        section_title: row.get(7)?,
        ordinal: row.get(8)?,
        chunk_type: row.get(9)?,
        detected_format: row.get(10)?,
        content: row.get(11)?,
    })
}

fn search_attachment_documents_with_connection(
    connection: &Connection,
    chat_id: &str,
    message_id: Option<&str>,
    reference_id: Option<&str>,
    attachment_version_id: Option<&str>,
    version_number: Option<i64>,
    extractor: &str,
    extractor_version: &str,
    query: &str,
    query_embedding: &[f32],
    limit: usize,
) -> Result<Vec<DocumentSearchResult>, String> {
    validate_extractor(extractor, extractor_version)?;
    if query_embedding.len() != NOMIC_EMBEDDING_DIMENSIONS
        || query_embedding.iter().any(|value| !value.is_finite())
    {
        return Err(format!(
            "Query embedding must contain exactly {} finite values",
            NOMIC_EMBEDDING_DIMENSIONS
        ));
    }

    let (scope_cte, scope_values): (&str, Vec<rusqlite::types::Value>) = if let Some(reference_id) =
        reference_id
    {
        let attachment_version_id = attachment_version_id
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "A version ID is required for attachment-scoped search".to_string())?;
        let version_number = version_number.filter(|value| *value > 0).ok_or_else(|| {
            "A positive version number is required for attachment-scoped search".to_string()
        })?;
        if reference_id.is_empty() || reference_id.len() > 128 || attachment_version_id.len() > 128
        {
            return Err("Invalid attachment scope identity".into());
        }

        let exact_scope_exists: bool = connection
            .query_row(
                "SELECT EXISTS (
                        SELECT 1
                        FROM attachment_references r
                        JOIN attachment_versions v ON v.id = r.attachment_version_id
                        WHERE r.id = ?1
                          AND r.chat_id = ?2
                          AND v.id = ?3
                          AND v.version_number = ?4
                          AND (
                            (?5 IS NULL AND r.purpose = 'composer' AND r.message_id IS NULL)
                            OR
                            (?5 IS NOT NULL AND r.purpose = 'message' AND r.message_id = ?5
                             AND EXISTS (
                                SELECT 1 FROM messages m
                                WHERE m.id = r.message_id
                                  AND m.chat_id = r.chat_id
                                  AND m.role = 'user'
                             ))
                          )
                    )",
                rusqlite::params![
                    reference_id,
                    chat_id,
                    attachment_version_id,
                    version_number,
                    message_id
                ],
                |row| row.get(0),
            )
            .map_err(|error| format!("Cannot validate attachment search scope: {error}"))?;
        if !exact_scope_exists {
            return Err(
                "Attachment search scope does not match the exact reference/version/message".into(),
            );
        }

        (
            r#"WITH authorized AS (
                    SELECT DISTINCT
                        d.id AS document_id,
                        r.id AS reference_id,
                        v.id AS attachment_version_id,
                        v.version_number,
                        v.file_name
                    FROM extracted_documents d
                    JOIN attachment_versions v ON v.blob_sha256 = d.blob_sha256
                    JOIN attachment_references r ON r.attachment_version_id = v.id
                    WHERE d.status = 'completed'
                      AND d.extractor = ?8
                      AND d.extractor_version = ?9
                      AND r.chat_id = ?3
                      AND r.id = ?4
                      AND v.id = ?5
                      AND v.version_number = ?6
                      AND (
                        (?7 IS NULL AND r.purpose = 'composer' AND r.message_id IS NULL)
                        OR
                        (?7 IS NOT NULL AND r.purpose = 'message' AND r.message_id = ?7
                         AND EXISTS (
                            SELECT 1 FROM messages m
                            WHERE m.id = r.message_id
                              AND m.chat_id = r.chat_id
                              AND m.role = 'user'
                         ))
                      )
                )"#,
            vec![
                rusqlite::types::Value::Text(chat_id.to_owned()),
                rusqlite::types::Value::Text(reference_id.to_owned()),
                rusqlite::types::Value::Text(attachment_version_id.to_owned()),
                rusqlite::types::Value::Integer(version_number),
                message_id.map_or(rusqlite::types::Value::Null, |value| {
                    rusqlite::types::Value::Text(value.to_owned())
                }),
                rusqlite::types::Value::Text(extractor.to_owned()),
                rusqlite::types::Value::Text(extractor_version.to_owned()),
            ],
        )
    } else if let Some(message_id) = message_id {
        if message_id.is_empty()
            || message_id.len() > 128
            || attachment_version_id.is_some()
            || version_number.is_some()
        {
            return Err("Invalid message-scoped search identity".into());
        }
        let valid_message: bool = connection
            .query_row(
                "SELECT EXISTS (
                        SELECT 1 FROM messages
                        WHERE id = ?1 AND chat_id = ?2 AND role = 'user'
                    )",
                rusqlite::params![message_id, chat_id],
                |row| row.get(0),
            )
            .map_err(|error| format!("Cannot validate message search scope: {error}"))?;
        if !valid_message {
            return Err("Search message does not belong to the specified chat".into());
        }

        (
            r#"WITH authorized AS (
                    SELECT DISTINCT
                        d.id AS document_id,
                        r.id AS reference_id,
                        v.id AS attachment_version_id,
                        v.version_number,
                        v.file_name
                    FROM extracted_documents d
                    JOIN attachment_versions v ON v.blob_sha256 = d.blob_sha256
                    JOIN attachment_references r ON r.attachment_version_id = v.id
                    JOIN messages m
                      ON m.id = r.message_id
                     AND m.chat_id = r.chat_id
                     AND m.role = 'user'
                    WHERE d.status = 'completed'
                      AND d.extractor = ?5
                      AND d.extractor_version = ?6
                      AND r.chat_id = ?3
                      AND r.message_id = ?4
                      AND r.purpose = 'message'
                )"#,
            vec![
                rusqlite::types::Value::Text(chat_id.to_owned()),
                rusqlite::types::Value::Text(message_id.to_owned()),
                rusqlite::types::Value::Text(extractor.to_owned()),
                rusqlite::types::Value::Text(extractor_version.to_owned()),
            ],
        )
    } else {
        return Err(
            "Attachment search requires a user message ID or an exact attachment reference/version"
                .into(),
        );
    };

    let terms: Vec<String> = query
        .split_whitespace()
        .take(16)
        .map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
        .collect();
    if terms.is_empty() {
        return Ok(Vec::new());
    }

    let fts_query = terms.join(" OR ");
    let limit = limit.clamp(1, 50);
    let candidate_limit = (limit * 4).clamp(20, 200);
    let query_vector: Vec<u8> = query_embedding
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect();

    let lexical_results = {
        let sql = format!(
            "{scope_cte}
             SELECT c.id, a.reference_id, a.attachment_version_id, a.version_number,
                    a.file_name, d.id, c.section_id, s.title, c.ordinal, c.chunk_type,
                    d.detected_format, c.content
             FROM chunk_search
             JOIN document_chunks c ON c.id = chunk_search.chunk_id
             JOIN extracted_documents d ON d.id = c.document_id
             JOIN authorized a ON a.document_id = d.id
             LEFT JOIN document_sections s ON s.id = c.section_id
             WHERE chunk_search MATCH ?1
             ORDER BY bm25(chunk_search) ASC
             LIMIT ?2"
        );
        let mut statement = connection
            .prepare(&sql)
            .map_err(|error| format!("Cannot prepare scoped lexical search: {error}"))?;
        let mut values = vec![
            rusqlite::types::Value::Text(fts_query),
            rusqlite::types::Value::Integer(candidate_limit as i64),
        ];
        values.extend(scope_values.iter().cloned());
        let rows = statement
            .query_map(
                rusqlite::params_from_iter(values.iter()),
                map_document_search_result,
            )
            .map_err(|error| format!("Cannot execute scoped lexical search: {error}"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("Cannot read scoped lexical search results: {error}"))?
    };

    let mut ranked: HashMap<String, (DocumentSearchResult, f64)> = HashMap::new();
    for (index, result) in lexical_results.into_iter().enumerate() {
        let key = format!(
            "{}:{}:{}",
            result.chunk_id, result.reference_id, result.attachment_version_id
        );
        let entry = ranked.entry(key).or_insert((result, 0.0));
        entry.1 += 1.0 / (60.0 + (index + 1) as f64);
    }

    // sqlite-vec v0.1.9 supports an ID-IN prefilter as part of the vec0 KNN
    // virtual-table query plan. Keep this predicate inside vec0: an outer WHERE
    // after KNN would allow unrelated vectors to consume the top-K candidate set.
    let semantic_results = {
        let sql = format!(
            "{scope_cte}
             SELECT c.id, a.reference_id, a.attachment_version_id, a.version_number,
                    a.file_name, d.id, c.section_id, s.title, c.ordinal, c.chunk_type,
                    d.detected_format, c.content
             FROM document_vectors_nomic
             JOIN document_chunks c ON c.id = document_vectors_nomic.chunk_id
             JOIN extracted_documents d ON d.id = c.document_id
             JOIN authorized a ON a.document_id = d.id
             LEFT JOIN document_sections s ON s.id = c.section_id
             WHERE embedding MATCH ?1
               AND k = ?2
               AND document_vectors_nomic.chunk_id IN (
                    SELECT allowed_chunks.id
                    FROM document_chunks allowed_chunks
                    JOIN authorized allowed_docs
                      ON allowed_docs.document_id = allowed_chunks.document_id
               )
             ORDER BY distance ASC"
        );
        let mut statement = connection
            .prepare(&sql)
            .map_err(|error| format!("Cannot prepare scoped semantic search: {error}"))?;
        let mut values = vec![
            rusqlite::types::Value::Blob(query_vector),
            rusqlite::types::Value::Integer(candidate_limit as i64),
        ];
        values.extend(scope_values.iter().cloned());
        let rows = statement
            .query_map(
                rusqlite::params_from_iter(values.iter()),
                map_document_search_result,
            )
            .map_err(|error| format!("Cannot execute scoped semantic search: {error}"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("Cannot read scoped semantic search results: {error}"))?
    };

    for (index, result) in semantic_results.into_iter().enumerate() {
        let key = format!(
            "{}:{}:{}",
            result.chunk_id, result.reference_id, result.attachment_version_id
        );
        let entry = ranked.entry(key).or_insert((result, 0.0));
        entry.1 += 1.0 / (60.0 + (index + 1) as f64);
    }

    let mut results: Vec<_> = ranked.into_values().collect();
    results.sort_by(|left, right| {
        right
            .1
            .total_cmp(&left.1)
            .then_with(|| left.0.file_name.cmp(&right.0.file_name))
            .then_with(|| left.0.ordinal.cmp(&right.0.ordinal))
            .then_with(|| left.0.reference_id.cmp(&right.0.reference_id))
    });

    Ok(results
        .into_iter()
        .take(limit)
        .map(|(result, _)| result)
        .collect())
}

#[tauri::command]
pub fn search_attachment_documents(
    state: State<'_, Mutex<Storage>>,
    chat_id: String,
    message_id: Option<String>,
    reference_id: Option<String>,
    attachment_version_id: Option<String>,
    version_number: Option<i64>,
    extractor: String,
    extractor_version: String,
    query: String,
    query_embedding: Vec<f32>,
    limit: Option<usize>,
) -> Result<Vec<DocumentSearchResult>, String> {
    validate_extractor(&extractor, &extractor_version)?;
    let query = query.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }
    if query.len() > 512 {
        return Err("Search query exceeds 512 bytes".into());
    }

    let storage = state
        .lock()
        .map_err(|_| "Storage lock is poisoned".to_string())?;

    search_attachment_documents_with_connection(
        &storage.connection,
        &chat_id,
        message_id.as_deref(),
        reference_id.as_deref(),
        attachment_version_id.as_deref(),
        version_number,
        &extractor,
        &extractor_version,
        query,
        &query_embedding,
        limit.unwrap_or(20),
    )
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
    processing_token: Option<String>,
    limit: Option<usize>,
) -> Result<Vec<DocumentEmbeddingChunk>, String> {
    if document_id.is_empty() || document_id.len() > 128 {
        return Err("Invalid document ID".into());
    }

    let limit = limit.unwrap_or(32).clamp(1, MAX_EMBEDDING_BATCH);
    let storage = state
        .lock()
        .map_err(|_| "Storage lock is poisoned".to_string())?;

    validate_document_access(
        &storage.connection,
        &document_id,
        processing_token.as_deref(),
    )?;

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
    processing_token: Option<String>,
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

    validate_document_access(&transaction, &document_id, processing_token.as_deref())?;

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
    processing_token: Option<String>,
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

    let document_status =
        validate_document_access(&transaction, &document_id, processing_token.as_deref())?;
    if document_status == "completed" {
        transaction
            .commit()
            .map_err(|error| format!("Cannot finish idempotent document finalization: {error}"))?;
        return Ok(());
    }

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
             processing_token = NULL,
             updated_at = ?1,
             processed_at = ?1,
             error_code = NULL,
             error_message = NULL
         WHERE id = ?2
           AND status = 'processing'
           AND processing_token = ?3",
            params![timestamp, document_id, processing_token],
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

const DOCUMENT_SUMMARY_PART_UPSERT_SQL: &str = r#"
INSERT INTO document_summary_parts (
    document_id, stage, ordinal, content, source_chunk_count,
    model_name, summarizer_version, updated_at
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
ON CONFLICT(document_id, stage, ordinal)
DO UPDATE SET
    content = excluded.content,
    source_chunk_count = excluded.source_chunk_count,
    model_name = excluded.model_name,
    summarizer_version = excluded.summarizer_version,
    updated_at = excluded.updated_at
"#;

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

    let mut statement = storage
        .connection
        .prepare(
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
         LIMIT ?3",
        )
        .map_err(|error| format!("Cannot prepare summary chunk query: {error}"))?;

    let rows = statement
        .query_map(params![document_id, after, limit as i64], |row| {
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
        })
        .map_err(|error| format!("Cannot query summary chunks: {error}"))?;

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

    let changed = storage
        .connection
        .execute(
            "DELETE FROM document_summary_parts WHERE document_id = ?1 AND stage = ?2",
            params![document_id, stage],
        )
        .map_err(|error| format!("Cannot clear document summary stage: {error}"))?;

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

    let transaction = storage
        .connection
        .transaction()
        .map_err(|error| format!("Cannot begin summary transaction: {error}"))?;

    let exists: bool = transaction
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM extracted_documents WHERE id = ?1)",
            [&document_id],
            |row| row.get(0),
        )
        .map_err(|error| format!("Cannot validate summary document: {error}"))?;

    if !exists {
        return Err("Summary document does not exist".into());
    }

    for part in &parts {
        transaction
            .execute(
                DOCUMENT_SUMMARY_PART_UPSERT_SQL,
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
            )
            .map_err(|error| format!("Cannot save summary part: {error}"))?;
    }

    transaction
        .commit()
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

    let mut statement = storage
        .connection
        .prepare(
            "SELECT ordinal, content, source_chunk_count,
                model_name, summarizer_version
         FROM document_summary_parts
         WHERE document_id = ?1 AND stage = ?2 AND ordinal > ?3
         ORDER BY ordinal ASC
         LIMIT ?4",
        )
        .map_err(|error| format!("Cannot prepare summary parts query: {error}"))?;

    let rows = statement
        .query_map(params![document_id, stage, after, limit as i64], |row| {
            Ok(DocumentSummaryPart {
                ordinal: row.get(0)?,
                content: row.get(1)?,
                source_chunk_count: row.get(2)?,
                model_name: row.get(3)?,
                summarizer_version: row.get(4)?,
            })
        })
        .map_err(|error| format!("Cannot query summary parts: {error}"))?;

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

    let transaction = storage
        .connection
        .transaction()
        .map_err(|error| format!("Cannot begin final summary transaction: {error}"))?;

    let actual_count: i64 = transaction
        .query_row(
            "SELECT COUNT(*) FROM document_chunks WHERE document_id = ?1",
            [&document_id],
            |row| row.get(0),
        )
        .map_err(|error| format!("Cannot verify summarized chunk count: {error}"))?;

    if actual_count == 0 || actual_count != source_chunk_count {
        return Err(format!(
            "Summary does not cover the complete document: expected {actual_count} chunks, received {source_chunk_count}"
        ));
    }

    transaction
        .execute(
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
        )
        .map_err(|error| format!("Cannot save final document summary: {error}"))?;

    transaction
        .commit()
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

    storage
        .connection
        .query_row(
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
        )
        .optional()
        .map_err(|error| format!("Cannot read document summary: {error}"))
}

#[cfg(test)]
mod processing_claim_tests {
    use super::{
        PROCESSING_LEASE_MS, claim_document_processing, now_timestamp,
        recover_interrupted_document_processing, renew_document_claim, validate_document_access,
    };
    use rusqlite::Connection;
    use std::{
        sync::{Arc, Barrier},
        thread,
        time::Duration,
    };
    use uuid::Uuid;

    fn connection() -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE extracted_documents (
                    id TEXT PRIMARY KEY,
                    blob_sha256 TEXT NOT NULL,
                    extractor TEXT NOT NULL,
                    extractor_version TEXT NOT NULL,
                    status TEXT NOT NULL,
                    created_at INTEGER NOT NULL,
                    updated_at INTEGER NOT NULL,
                    source_size INTEGER NOT NULL,
                    processing_token TEXT,
                    error_code TEXT,
                    error_message TEXT,
                    UNIQUE(blob_sha256, extractor, extractor_version)
                );",
            )
            .unwrap();
        connection
    }

    fn claim(connection: &Connection, id: &str, token: &str, timestamp: i64) -> bool {
        claim_document_processing(
            connection,
            id,
            "aabbcc",
            "sonne-unified",
            "v1",
            42,
            token,
            timestamp,
        )
        .unwrap()
    }

    #[test]
    fn simultaneous_claims_across_independent_connections_have_one_winner() {
        let directory =
            std::env::temp_dir().join(format!("sonne-processing-claim-{}", Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let database_path = directory.join("claims.sqlite");

        let setup = Connection::open(&database_path).unwrap();
        setup
            .execute_batch(
                "CREATE TABLE extracted_documents (
                    id TEXT PRIMARY KEY,
                    blob_sha256 TEXT NOT NULL,
                    extractor TEXT NOT NULL,
                    extractor_version TEXT NOT NULL,
                    status TEXT NOT NULL,
                    created_at INTEGER NOT NULL,
                    updated_at INTEGER NOT NULL,
                    source_size INTEGER NOT NULL,
                    processing_token TEXT,
                    error_code TEXT,
                    error_message TEXT,
                    UNIQUE(blob_sha256, extractor, extractor_version)
                );",
            )
            .unwrap();
        drop(setup);

        let first_connection = Connection::open(&database_path).unwrap();
        first_connection
            .busy_timeout(Duration::from_secs(5))
            .unwrap();

        let second_connection = Connection::open(&database_path).unwrap();
        second_connection
            .busy_timeout(Duration::from_secs(5))
            .unwrap();

        let barrier = Arc::new(Barrier::new(3));

        let first_barrier = Arc::clone(&barrier);
        let first_thread = thread::spawn(move || {
            first_barrier.wait();
            claim(&first_connection, "doc-1", "owner-1", 1_000)
        });

        let second_barrier = Arc::clone(&barrier);
        let second_thread = thread::spawn(move || {
            second_barrier.wait();
            claim(&second_connection, "doc-2", "owner-2", 1_000)
        });

        // Release both independent SQLite connections at the same time.
        barrier.wait();

        let first_won = first_thread.join().unwrap();
        let second_won = second_thread.join().unwrap();

        assert_ne!(
            first_won, second_won,
            "exactly one simultaneous processing claim must succeed"
        );

        let verification = Connection::open(&database_path).unwrap();
        let (stored_id, stored_token): (String, String) = verification
            .query_row(
                "SELECT id, processing_token FROM extracted_documents",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();

        if first_won {
            assert_eq!(stored_id, "doc-1");
            assert_eq!(stored_token, "owner-1");
        } else {
            assert_eq!(stored_id, "doc-2");
            assert_eq!(stored_token, "owner-2");
        }

        drop(verification);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn only_one_active_owner_can_claim_a_document() {
        let connection = connection();

        assert!(claim(&connection, "doc-1", "owner-1", 1_000));
        assert!(!claim(&connection, "doc-2", "owner-2", 1_001));

        assert_eq!(
            validate_document_access(&connection, "doc-1", Some("owner-1")).unwrap(),
            "processing"
        );
        assert!(validate_document_access(&connection, "doc-1", Some("owner-2")).is_err());
    }

    #[test]
    fn expired_claim_can_be_reclaimed_and_old_owner_is_rejected() {
        let connection = connection();

        assert!(claim(&connection, "doc-1", "owner-1", 1_000));
        assert!(claim(
            &connection,
            "doc-2",
            "owner-2",
            1_000 + PROCESSING_LEASE_MS + 1
        ));

        assert!(validate_document_access(&connection, "doc-1", Some("owner-1")).is_err());
        assert_eq!(
            validate_document_access(&connection, "doc-1", Some("owner-2")).unwrap(),
            "processing"
        );
    }

    #[test]
    fn heartbeat_renews_only_the_current_owner_lease() {
        let connection = connection();
        assert!(claim(&connection, "doc-1", "owner-1", 1_000));

        assert!(!renew_document_claim(&connection, "doc-1", "wrong-owner", 2_000).unwrap());
        assert!(renew_document_claim(&connection, "doc-1", "owner-1", 2_000).unwrap());

        assert!(!claim(
            &connection,
            "doc-2",
            "owner-2",
            1_000 + PROCESSING_LEASE_MS + 1
        ));
    }

    #[test]
    fn startup_recovery_preserves_active_claim_and_resets_expired_claim() {
        let connection = connection();
        let now = now_timestamp().unwrap();

        assert!(claim(&connection, "doc-active", "active-owner", now));

        let expired_timestamp = now.saturating_sub(PROCESSING_LEASE_MS).saturating_sub(1);

        connection
            .execute(
                "INSERT INTO extracted_documents (
                    id, blob_sha256, extractor, extractor_version, status,
                    created_at, updated_at, source_size, processing_token
                ) VALUES (
                    ?1, ?2, ?3, ?4, 'processing', ?5, ?5, ?6, ?7
                )",
                rusqlite::params![
                    "doc-stale",
                    "ddeeff",
                    "sonne-unified",
                    "v1",
                    expired_timestamp,
                    42,
                    "stale-owner"
                ],
            )
            .unwrap();

        let recovered = recover_interrupted_document_processing(&connection).unwrap();
        assert_eq!(recovered, 1);

        let (active_status, active_token): (String, Option<String>) = connection
            .query_row(
                "SELECT status, processing_token
                 FROM extracted_documents WHERE id = 'doc-active'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();

        assert_eq!(active_status, "processing");
        assert_eq!(active_token.as_deref(), Some("active-owner"));
        assert_eq!(
            validate_document_access(&connection, "doc-active", Some("active-owner")).unwrap(),
            "processing"
        );

        let (stale_status, stale_token): (String, Option<String>) = connection
            .query_row(
                "SELECT status, processing_token
                 FROM extracted_documents WHERE id = 'doc-stale'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();

        assert_eq!(stale_status, "pending");
        assert!(stale_token.is_none());
        assert!(validate_document_access(&connection, "doc-stale", Some("stale-owner")).is_err());
    }

    #[test]
    fn completed_document_cannot_be_claimed_again() {
        let connection = connection();
        assert!(claim(&connection, "doc-1", "owner-1", 1_000));

        connection
            .execute(
                "UPDATE extracted_documents
                 SET status = 'completed', processing_token = NULL
                 WHERE id = 'doc-1'",
                [],
            )
            .unwrap();

        assert!(!claim(
            &connection,
            "doc-2",
            "owner-2",
            1_000 + PROCESSING_LEASE_MS + 10_000
        ));

        assert_eq!(
            validate_document_access(&connection, "doc-1", None).unwrap(),
            "completed"
        );
    }
}
#[cfg(test)]
mod extracted_asset_validation_tests {
    use super::*;

    fn asset() -> ExtractedAssetInput {
        ExtractedAssetInput {
            section_key: Some("root".into()),
            asset_type: "image".into(),
            mime_type: Some("image/png".into()),
            file_name: Some("image.png".into()),
            ordinal: 0,
            data: vec![1, 2, 3],
            metadata: None,
        }
    }

    #[test]
    fn one_invalid_asset_can_be_rejected_without_invalidating_valid_peers() {
        let section_keys = HashSet::from(["root"]);
        let valid_before = asset();

        let mut invalid = asset();
        invalid.mime_type = Some(String::new());

        let valid_after = asset();

        assert!(validate_asset(&valid_before, &section_keys).is_ok());
        assert!(validate_asset(&invalid, &section_keys).is_err());
        assert!(validate_asset(&valid_after, &section_keys).is_ok());
    }

    #[test]
    fn document_validation_allows_asset_overflow_for_bounded_processing() {
        let assets = (0..(MAX_DOCUMENT_ASSETS + 3))
            .map(|ordinal| ExtractedAssetInput {
                section_key: Some("root".into()),
                asset_type: "image".into(),
                mime_type: Some("image/png".into()),
                file_name: Some(format!("asset-{ordinal}.png")),
                ordinal: ordinal as i64,
                data: vec![1, 2, 3],
                metadata: None,
            })
            .collect();
        let input = SaveExtractedDocumentInput {
            chat_id: "chat".into(),
            reference_id: "attachment".into(),
            attachment_version_id: "version-test".into(),
            version_number: 1,
            extractor: "test".into(),
            extractor_version: "v1".into(),
            processing_token: "test-token".into(),
            detected_format: "docx".into(),
            language: None,
            metadata: None,
            source_size: 1,
            sections: vec![ExtractedSectionInput {
                key: "root".into(),
                parent_key: None,
                section_type: "root".into(),
                title: Some("Document".into()),
                ordinal: 0,
                metadata: None,
            }],
            chunks: vec![ExtractedChunkInput {
                section_key: Some("root".into()),
                content: "Document text".into(),
                token_count: Some(2),
                start_offset: Some(0),
                end_offset: Some(13),
                chunk_type: "text".into(),
                metadata: None,
            }],
            assets,
        };

        assert!(validate_document(&input).is_ok());
        let (accepted, skipped) = bounded_asset_inputs(&input.assets);
        assert_eq!(accepted.len(), MAX_DOCUMENT_ASSETS);
        assert_eq!(skipped, 3);
    }

    #[test]
    fn backend_caps_asset_inputs_at_the_document_limit() {
        let assets = vec![asset(); MAX_DOCUMENT_ASSETS + 3];
        let (accepted, skipped) = bounded_asset_inputs(&assets);

        assert_eq!(accepted.len(), MAX_DOCUMENT_ASSETS);
        assert_eq!(skipped, 3);
    }

    #[test]
    fn asset_warnings_are_bounded_and_count_overflow() {
        let mut warnings = Vec::new();
        let mut omitted = 0usize;

        for index in 0..(MAX_ASSET_WARNINGS + 4) {
            push_asset_warning(&mut warnings, &mut omitted, format!("warning {index}"));
        }

        assert_eq!(warnings.len(), MAX_ASSET_WARNINGS);
        assert_eq!(omitted, 4);
    }

    #[test]
    fn oversized_asset_diagnostics_do_not_reject_source_metadata() {
        let payload = "x".repeat(MAX_METADATA_BYTES - 32);
        let mut metadata = serde_json::json!({ "payload": payload });
        let object = metadata.as_object_mut().unwrap();
        object.insert(
            "assetExtractionWarnings".into(),
            serde_json::json!(vec![
                "w".repeat(MAX_ASSET_WARNING_LENGTH);
                MAX_ASSET_WARNINGS
            ]),
        );
        object.insert(
            "assetExtractionWarningsOmitted".into(),
            serde_json::json!(7),
        );
        let metadata = Some(metadata);

        assert!(validate_json_size(&metadata, "Document metadata").is_err());
        assert!(validate_source_document_metadata(&metadata).is_ok());

        let saved = build_document_metadata(metadata, &["An asset was skipped".to_string()], 0)
            .unwrap()
            .unwrap();
        let saved: Value = serde_json::from_str(&saved).unwrap();
        assert_eq!(
            saved["payload"].as_str().unwrap().len(),
            MAX_METADATA_BYTES - 32
        );
        assert!(saved.get("assetExtractionWarnings").is_none());
        assert!(saved.get("assetExtractionWarningsOmitted").is_none());
    }

    #[test]
    fn asset_warnings_are_persisted_in_document_metadata() {
        let warnings = vec!["Asset #2 was skipped: invalid MIME type".to_string()];
        let serialized = build_document_metadata(
            Some(serde_json::json!({ "fileName": "report.pdf" })),
            &warnings,
            2,
        )
        .unwrap()
        .unwrap();

        let value: serde_json::Value = serde_json::from_str(&serialized).unwrap();
        assert_eq!(
            value["assetExtractionWarnings"][0],
            "Asset #2 was skipped: invalid MIME type"
        );
        assert_eq!(value["assetExtractionWarningsOmitted"], 2);
        assert_eq!(value["fileName"], "report.pdf");
    }
}

#[cfg(test)]
mod document_asset_batch_rollback_tests {
    use super::*;
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    struct TestDirectory(PathBuf);

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn test_storage() -> (TestDirectory, Storage) {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = TestDirectory(std::env::temp_dir().join(format!(
            "sonne-document-asset-rollback-{}-{nonce}",
            std::process::id()
        )));
        fs::create_dir_all(&directory.0).unwrap();
        let storage = Storage::initialize(&directory.0).expect("storage should initialize");
        (directory, storage)
    }

    fn insert_document(storage: &Storage, id: &str, source_hash: &str) {
        storage
            .connection
            .execute(
                "INSERT INTO extracted_documents (
                id, blob_sha256, extractor, extractor_version, status,
                created_at, updated_at, detected_format, source_size
             ) VALUES (?1, ?2, 'test', 'v1', 'completed', 1, 1, 'docx', 1)",
                params![id, source_hash],
            )
            .unwrap();
    }

    fn insert_asset(storage: &Storage, id: &str, document_id: &str, blob_hash: &str) {
        storage
            .connection
            .execute(
                "INSERT INTO document_assets (
                id, document_id, section_id, blob_sha256,
                asset_type, ordinal, created_at
             ) VALUES (?1, ?2, NULL, ?3, 'image', 0, 1)",
                params![id, document_id, blob_hash],
            )
            .unwrap();
    }

    #[test]
    fn rolled_back_document_replacement_preserves_old_and_shared_assets_only() {
        let (_directory, mut storage) = test_storage();
        let timestamp = now_timestamp().unwrap();
        let source_main = storage
            .store_blob(Cursor::new(b"source-main".to_vec()))
            .unwrap();
        let source_other = storage
            .store_blob(Cursor::new(b"source-other".to_vec()))
            .unwrap();
        let old_asset = storage
            .store_blob(Cursor::new(b"old-document-asset".to_vec()))
            .unwrap();
        let shared_asset = storage
            .store_blob(Cursor::new(b"shared-document-asset".to_vec()))
            .unwrap();

        insert_document(&storage, "document-main", &source_main.sha256);
        insert_document(&storage, "document-other", &source_other.sha256);
        insert_asset(
            &storage,
            "old-asset-row",
            "document-main",
            &old_asset.sha256,
        );
        insert_asset(
            &storage,
            "shared-asset-row",
            "document-other",
            &shared_asset.sha256,
        );

        let batch_id = "rollback-asset-batch-test";
        let batch_metadata = serde_json::json!({
            "documentId": "document-main",
            "expectedAssetCount": 1,
            "replacedAssetHashes": [&old_asset.sha256]
        })
        .to_string();
        storage
            .connection
            .execute(
                "INSERT INTO storage_operations (
                id, operation_type, state, metadata_json, created_at, updated_at
             ) VALUES (?1, 'document_assets_batch', 'PROCESSING', ?2, ?3, ?3)",
                params![batch_id, batch_metadata, timestamp],
            )
            .unwrap();

        let new_asset = storage
            .store_blob_with_parent(
                Cursor::new(b"new-document-asset".to_vec()),
                1024 * 1024,
                Some(batch_id),
            )
            .unwrap();

        storage
            .connection
            .execute_batch(
                "CREATE TRIGGER force_document_asset_insert_failure
             BEFORE INSERT ON document_assets
             BEGIN SELECT RAISE(ABORT, 'forced rollback regression failure'); END;",
            )
            .unwrap();
        let transaction = storage.connection.transaction().unwrap();
        transaction
            .execute(
                "DELETE FROM document_assets WHERE document_id = 'document-main'",
                [],
            )
            .unwrap();
        let failed_insert = transaction.execute(
            "INSERT INTO document_assets (
                id, document_id, section_id, blob_sha256,
                asset_type, ordinal, created_at
             ) VALUES ('new-asset-row', 'document-main', NULL, ?1, 'image', 0, 2)",
            params![&new_asset.sha256],
        );
        assert!(
            failed_insert.is_err(),
            "test trigger must abort the replacement insert"
        );
        drop(transaction); // Roll back the earlier DELETE as well.
        storage
            .connection
            .execute_batch("DROP TRIGGER force_document_asset_insert_failure;")
            .unwrap();

        let old_reference_exists: bool = storage
            .connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM document_assets
             WHERE document_id = 'document-main' AND blob_sha256 = ?1)",
                params![&old_asset.sha256],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            old_reference_exists,
            "the previous asset row must survive rollback"
        );

        let cleanup_errors = rollback_document_asset_batch(
            &mut storage,
            batch_id,
            std::slice::from_ref(&new_asset.sha256),
            std::slice::from_ref(&old_asset.sha256),
            "forced rollback regression failure",
            timestamp,
        );
        assert!(
            cleanup_errors.is_empty(),
            "rollback cleanup should finish: {cleanup_errors:?}"
        );

        for (hash, path, label) in [
            (
                &old_asset.sha256,
                &old_asset.relative_path,
                "previous asset",
            ),
            (
                &shared_asset.sha256,
                &shared_asset.relative_path,
                "shared asset",
            ),
        ] {
            let exists: bool = storage
                .connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM blobs WHERE sha256 = ?1)",
                    params![hash],
                    |row| row.get(0),
                )
                .unwrap();
            assert!(exists, "{label} metadata must remain");
            assert!(
                storage.root_dir.join(path).exists(),
                "{label} file must remain"
            );
        }

        let new_exists: bool = storage
            .connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM blobs WHERE sha256 = ?1)",
                params![&new_asset.sha256],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            !new_exists,
            "unreferenced new asset metadata must be removed"
        );
        assert!(
            !storage.root_dir.join(&new_asset.relative_path).exists(),
            "unreferenced new asset file must be removed"
        );

        let parent_state: String = storage
            .connection
            .query_row(
                "SELECT state FROM storage_operations WHERE id = ?1",
                params![batch_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(parent_state, "COMPLETED");
    }
}

#[cfg(test)]
mod document_summary_persistence_tests {
    use super::*;

    #[test]
    fn summary_part_upsert_persists_all_eight_columns() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE document_summary_parts (
                    document_id TEXT NOT NULL,
                    stage INTEGER NOT NULL,
                    ordinal INTEGER NOT NULL,
                    content TEXT NOT NULL,
                    source_chunk_count INTEGER NOT NULL,
                    model_name TEXT NOT NULL,
                    summarizer_version TEXT NOT NULL,
                    updated_at INTEGER NOT NULL,
                    PRIMARY KEY (document_id, stage, ordinal)
                );",
            )
            .unwrap();

        let changed = connection
            .execute(
                DOCUMENT_SUMMARY_PART_UPSERT_SQL,
                params![
                    "doc-test",
                    0_i64,
                    0_i64,
                    "Тестове резюме",
                    3_i64,
                    "sonne-gemma:latest",
                    "sonne-map-reduce-v1",
                    123_i64,
                ],
            )
            .expect("summary part SQL must bind all eight columns");
        assert_eq!(changed, 1);

        let row: (String, i64, i64, String, i64, String, String, i64) = connection
            .query_row(
                "SELECT document_id, stage, ordinal, content, source_chunk_count,
                        model_name, summarizer_version, updated_at
                 FROM document_summary_parts",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                        row.get(7)?,
                    ))
                },
            )
            .unwrap();

        assert_eq!(
            row,
            (
                "doc-test".to_string(),
                0,
                0,
                "Тестове резюме".to_string(),
                3,
                "sonne-gemma:latest".to_string(),
                "sonne-map-reduce-v1".to_string(),
                123,
            )
        );
    }
}

#[cfg(test)]
mod attachment_search_scope_tests {
    use super::*;
    use std::{fs, path::PathBuf};
    use uuid::Uuid;

    struct TestDirectory(PathBuf);

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn test_storage() -> (TestDirectory, Storage) {
        let directory = TestDirectory(std::env::temp_dir().join(format!(
            "sonne-attachment-scope-{}-{}",
            std::process::id(),
            Uuid::new_v4()
        )));
        fs::create_dir(&directory.0).expect("cannot create unique temporary test directory");
        let storage =
            Storage::initialize(&directory.0).expect("storage initialization must succeed");
        (directory, storage)
    }

    fn insert_message_document(
        connection: &Connection,
        chat_id: &str,
        message_id: &str,
        attachment_id: &str,
        version_id: &str,
        reference_id: &str,
        document_id: &str,
        chunk_id: &str,
        blob_hash: &str,
        content: &str,
        embedding: &[u8],
    ) {
        connection
            .execute(
                "INSERT INTO chats (id, title, created_at, updated_at)
             VALUES (?1, 'test', 1, 1)
             ON CONFLICT(id) DO NOTHING",
                [chat_id],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO messages (id, chat_id, role, content, created_at)
             VALUES (?1, ?2, 'user', 'test prompt', 1)",
                rusqlite::params![message_id, chat_id],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO blobs (
                sha256, relative_path, original_size, stored_size, storage_state,
                created_at, last_accessed_at
             ) VALUES (?1, ?2, 32, 32, 'original', 1, 1)",
                rusqlite::params![blob_hash, format!("blobs/{blob_hash}")],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO attachments (id, created_at, updated_at) VALUES (?1, 1, 1)",
                [attachment_id],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO attachment_versions (
                id, attachment_id, version_number, blob_sha256, file_name,
                extension, mime_type, created_at
             ) VALUES (?1, ?2, 1, ?3, 'same-name.txt', 'txt', 'text/plain', 1)",
                rusqlite::params![version_id, attachment_id, blob_hash],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO attachment_references (
                id, chat_id, attachment_version_id, message_id, purpose, position, created_at
             ) VALUES (?1, ?2, ?3, ?4, 'message', 0, 1)",
                rusqlite::params![reference_id, chat_id, version_id, message_id],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO extracted_documents (
                id, blob_sha256, extractor, extractor_version, status, created_at,
                updated_at, source_size, detected_format
             ) VALUES (?1, ?2, 'sonne-unified', 'scope-test-v1', 'completed', 1, 1, 32, 'txt')",
                rusqlite::params![document_id, blob_hash],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO document_chunks (id, document_id, ordinal, content, token_count)
             VALUES (?1, ?2, 0, ?3, 2)",
                rusqlite::params![chunk_id, document_id, content],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO document_vectors_nomic (chunk_id, embedding) VALUES (?1, ?2)",
                rusqlite::params![chunk_id, embedding],
            )
            .unwrap();
    }

    #[test]
    fn message_search_excludes_older_messages_and_other_chats_even_with_same_filename() {
        let (_directory, storage) = test_storage();
        let connection = storage.connection();
        let vector: Vec<f32> = (0..NOMIC_EMBEDDING_DIMENSIONS)
            .map(|index| if index == 0 { 1.0 } else { 0.0 })
            .collect();
        let embedding: Vec<u8> = vector
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect();

        insert_message_document(
            connection,
            "chat-a",
            "message-old",
            "attachment-old",
            "version-old",
            "reference-old",
            "document-old",
            "chunk-old",
            &"a".repeat(64),
            "needle from an older message",
            &embedding,
        );
        insert_message_document(
            connection,
            "chat-a",
            "message-current",
            "attachment-current",
            "version-current",
            "reference-current",
            "document-current",
            "chunk-current",
            &"b".repeat(64),
            "needle from the current message only",
            &embedding,
        );
        insert_message_document(
            connection,
            "chat-b",
            "message-other-chat",
            "attachment-other",
            "version-other",
            "reference-other",
            "document-other",
            "chunk-other",
            &"c".repeat(64),
            "needle from another chat",
            &embedding,
        );

        assert!(
            storage
                .get_preview_blob_exact("chat-a", "reference-current", "version-current", 1)
                .is_ok(),
            "the exact reference/version pair must resolve"
        );
        assert!(
            storage
                .get_preview_blob_exact("chat-a", "reference-current", "version-old", 1)
                .is_err(),
            "raw file reads must reject a mismatched immutable version"
        );

        let exact_message_blob = resolve_message_blob(
            &storage,
            "chat-a",
            "message-current",
            "reference-current",
            "version-current",
            1,
        )
        .expect("exact current-message identity must resolve");
        assert_eq!(exact_message_blob.0, "b".repeat(64));
        assert_eq!(exact_message_blob.1, 32);
        assert!(
            resolve_message_blob(
                &storage,
                "chat-a",
                "message-old",
                "reference-current",
                "version-current",
                1,
            )
            .is_err(),
            "a valid same-chat but different message ID must be rejected"
        );

        let results = search_attachment_documents_with_connection(
            connection,
            "chat-a",
            Some("message-current"),
            None,
            None,
            None,
            "sonne-unified",
            "scope-test-v1",
            "needle",
            &vector,
            10,
        )
        .expect("scoped hybrid search must succeed");

        assert!(
            !results.is_empty(),
            "current message document should be searchable"
        );
        assert!(
            results
                .iter()
                .all(|result| result.reference_id == "reference-current")
        );
        assert!(
            results
                .iter()
                .all(|result| result.attachment_version_id == "version-current")
        );
        assert!(
            results
                .iter()
                .all(|result| result.document_id == "document-current")
        );
        assert!(
            results
                .iter()
                .all(|result| result.content.contains("current message only"))
        );
    }

    #[test]
    fn attachment_search_rejects_a_mismatched_version_id() {
        let (_directory, storage) = test_storage();
        let connection = storage.connection();
        let vector: Vec<f32> = (0..NOMIC_EMBEDDING_DIMENSIONS)
            .map(|index| if index == 0 { 1.0 } else { 0.0 })
            .collect();
        let embedding: Vec<u8> = vector
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect();
        insert_message_document(
            connection,
            "chat-a",
            "message-current",
            "attachment-current",
            "version-current",
            "reference-current",
            "document-current",
            "chunk-current",
            &"b".repeat(64),
            "needle from the current message",
            &embedding,
        );

        let result = search_attachment_documents_with_connection(
            connection,
            "chat-a",
            Some("message-current"),
            Some("reference-current"),
            Some("version-from-another-file"),
            Some(1),
            "sonne-unified",
            "scope-test-v1",
            "needle",
            &vector,
            10,
        );

        assert!(
            result.is_err(),
            "a reference must never be searched through a different version ID"
        );
    }
}
