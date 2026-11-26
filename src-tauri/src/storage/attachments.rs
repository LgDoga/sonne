use rusqlite::{params, OptionalExtension, Transaction};
use serde::Serialize;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

use super::Storage;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentVersionRecord {
    pub id: String,
    pub attachment_id: String,
    pub version_number: i64,
    pub blob_sha256: String,
    pub file_name: String,
    pub extension: String,
    pub mime_type: String,
    pub original_size: i64,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentReferenceRecord {
    pub id: String,
    pub chat_id: String,
    pub message_id: Option<String>,
    pub attachment_version_id: String,
    pub attachment_id: String,
    pub version_number: i64,
    pub file_name: String,
    pub extension: String,
    pub mime_type: String,
    pub blob_sha256: String,
    pub original_size: i64,
    pub position: i64,
    pub purpose: String,
    pub created_at: i64,
}

fn now_timestamp() -> Result<i64, String> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("System clock error: {error}"))?;

    i64::try_from(duration.as_millis()).map_err(|error| format!("Timestamp overflow: {error}"))
}

pub(super) fn create_attachment_version_in_transaction(
    tx: &Transaction<'_>,
    attachment_id: Option<&str>,
    blob_sha256: &str,
    file_name: &str,
    extension: &str,
    mime_type: &str,
    timestamp: i64,
) -> Result<AttachmentVersionRecord, String> {
    if file_name.trim().is_empty() {
        return Err("Attachment filename cannot be empty".to_string());
    }

    let original_size: i64 = tx
        .query_row(
            "SELECT original_size FROM blobs WHERE sha256 = ?1",
            params![blob_sha256],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| format!("Cannot verify attachment blob: {e}"))?
        .ok_or_else(|| "Attachment blob does not exist".to_string())?;

    let resolved_id = match attachment_id {
        Some(id) => {
            let exists: bool = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM attachments WHERE id = ?1)",
                    params![id],
                    |row| row.get(0),
                )
                .map_err(|e| format!("Cannot verify attachment: {e}"))?;
            if !exists {
                return Err(format!("Attachment not found: {id}"));
            }
            id.to_owned()
        }
        None => {
            let id = Uuid::new_v4().to_string();
            tx.execute(
                "INSERT INTO attachments (id, created_at, updated_at) VALUES (?1, ?2, ?2)",
                params![id, timestamp],
            )
            .map_err(|e| format!("Cannot create attachment: {e}"))?;
            id
        }
    };

    let version_number: i64 = tx
        .query_row(
            "SELECT COALESCE(MAX(version_number), 0) + 1
         FROM attachment_versions WHERE attachment_id = ?1",
            params![resolved_id],
            |row| row.get(0),
        )
        .map_err(|e| format!("Cannot determine attachment version: {e}"))?;

    let version_id = Uuid::new_v4().to_string();

    tx.execute(
        "INSERT INTO attachment_versions (
            id, attachment_id, version_number, blob_sha256,
            file_name, extension, mime_type, created_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            version_id,
            resolved_id,
            version_number,
            blob_sha256,
            file_name,
            extension,
            mime_type,
            timestamp
        ],
    )
    .map_err(|e| format!("Cannot save attachment version: {e}"))?;

    tx.execute(
        "UPDATE attachments SET updated_at = ?1 WHERE id = ?2",
        params![timestamp, resolved_id],
    )
    .map_err(|e| format!("Cannot update attachment timestamp: {e}"))?;

    Ok(AttachmentVersionRecord {
        id: version_id,
        attachment_id: resolved_id,
        version_number,
        blob_sha256: blob_sha256.to_owned(),
        file_name: file_name.to_owned(),
        extension: extension.to_owned(),
        mime_type: mime_type.to_owned(),
        original_size,
        created_at: timestamp,
    })
}

pub(super) fn add_attachment_reference_in_transaction(
    tx: &Transaction<'_>,
    chat_id: &str,
    version_id: &str,
    message_id: Option<&str>,
    purpose: &str,
    position: i64,
    timestamp: i64,
) -> Result<String, String> {
    if !matches!(purpose, "composer" | "message") {
        return Err("Invalid attachment reference purpose".to_string());
    }
    if position < 0 {
        return Err("Attachment position cannot be negative".to_string());
    }
    match (purpose, message_id) {
        ("composer", None) | ("message", Some(_)) => {}
        _ => {
            return Err(
                "Composer references cannot have a message ID, and message references require one"
                    .to_string(),
            );
        }
    }

    let chat_exists: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM chats WHERE id = ?1)",
            params![chat_id],
            |row| row.get(0),
        )
        .map_err(|e| format!("Cannot verify reference chat: {e}"))?;
    if !chat_exists {
        return Err(format!("Chat not found: {chat_id}"));
    }

    let version_exists: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM attachment_versions WHERE id = ?1)",
            params![version_id],
            |row| row.get(0),
        )
        .map_err(|e| format!("Cannot verify attachment version: {e}"))?;
    if !version_exists {
        return Err(format!("Attachment version not found: {version_id}"));
    }

    if let Some(message_id) = message_id {
        let belongs: bool = tx
            .query_row(
                "SELECT EXISTS(
                SELECT 1 FROM messages WHERE id = ?1 AND chat_id = ?2
            )",
                params![message_id, chat_id],
                |row| row.get(0),
            )
            .map_err(|e| format!("Cannot verify target message: {e}"))?;
        if !belongs {
            return Err("Target message does not belong to the specified chat".to_string());
        }
    }

    let reference_id = Uuid::new_v4().to_string();
    tx.execute(
        "INSERT INTO attachment_references (
            id, chat_id, attachment_version_id, message_id,
            purpose, position, created_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            reference_id,
            chat_id,
            version_id,
            message_id,
            purpose,
            position,
            timestamp
        ],
    )
    .map_err(|e| format!("Cannot create attachment reference: {e}"))?;

    tx.execute(
        "UPDATE chats SET updated_at = ?1 WHERE id = ?2",
        params![timestamp, chat_id],
    )
    .map_err(|e| format!("Cannot update chat timestamp: {e}"))?;

    Ok(reference_id)
}

impl Storage {
    /// Creates a new attachment or adds a version to an existing attachment.
    /// The referenced physical blob must already exist.
    pub fn create_attachment_version(
        &mut self,
        attachment_id: Option<&str>,
        blob_sha256: &str,
        file_name: &str,
        extension: &str,
        mime_type: &str,
    ) -> Result<AttachmentVersionRecord, String> {
        let timestamp = now_timestamp()?;
        let tx = self
            .connection
            .transaction()
            .map_err(|e| format!("Cannot start attachment transaction: {e}"))?;

        let version = create_attachment_version_in_transaction(
            &tx,
            attachment_id,
            blob_sha256,
            file_name,
            extension,
            mime_type,
            timestamp,
        )?;

        tx.commit()
            .map_err(|e| format!("Cannot commit attachment version: {e}"))?;
        Ok(version)
    }

    /// Links an attachment version to a chat draft or a sent message.
    pub fn add_attachment_reference(
        &mut self,
        chat_id: &str,
        attachment_version_id: &str,
        message_id: Option<&str>,
        purpose: &str,
        position: i64,
    ) -> Result<String, String> {
        let timestamp = now_timestamp()?;
        let tx = self
            .connection
            .transaction()
            .map_err(|e| format!("Cannot start reference transaction: {e}"))?;

        let reference_id = add_attachment_reference_in_transaction(
            &tx,
            chat_id,
            attachment_version_id,
            message_id,
            purpose,
            position,
            timestamp,
        )?;

        tx.commit()
            .map_err(|e| format!("Cannot commit attachment reference: {e}"))?;
        Ok(reference_id)
    }

    /// Returns attachments belonging to a specific message.
    pub fn list_message_attachments(
        &self,
        message_id: &str,
    ) -> Result<Vec<AttachmentReferenceRecord>, String> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT
                    r.id,
                    r.chat_id,
                    r.message_id,
                    r.attachment_version_id,
                    v.attachment_id,
                    v.version_number,
                    v.file_name,
                    v.extension,
                    v.mime_type,
                    v.blob_sha256,
                    b.original_size,
                    r.position,
                    r.purpose,
                    r.created_at
                 FROM attachment_references r
                 JOIN attachment_versions v
                    ON v.id = r.attachment_version_id
                 JOIN blobs b
                    ON b.sha256 = v.blob_sha256
                 WHERE r.message_id = ?1
                 ORDER BY r.position ASC, r.created_at ASC",
            )
            .map_err(|error| format!("Cannot prepare attachment query: {error}"))?;

        let rows = statement
            .query_map(params![message_id], |row| {
                Ok(AttachmentReferenceRecord {
                    id: row.get(0)?,
                    chat_id: row.get(1)?,
                    message_id: row.get(2)?,
                    attachment_version_id: row.get(3)?,
                    attachment_id: row.get(4)?,
                    version_number: row.get(5)?,
                    file_name: row.get(6)?,
                    extension: row.get(7)?,
                    mime_type: row.get(8)?,
                    blob_sha256: row.get(9)?,
                    original_size: row.get(10)?,
                    position: row.get(11)?,
                    purpose: row.get(12)?,
                    created_at: row.get(13)?,
                })
            })
            .map_err(|error| format!("Cannot list message attachments: {error}"))?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("Cannot read attachment records: {error}"))
    }

    /// Returns draft attachments belonging to a chat.
    pub fn list_composer_attachments(
        &self,
        chat_id: &str,
    ) -> Result<Vec<AttachmentReferenceRecord>, String> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT
                    r.id,
                    r.chat_id,
                    r.message_id,
                    r.attachment_version_id,
                    v.attachment_id,
                    v.version_number,
                    v.file_name,
                    v.extension,
                    v.mime_type,
                    v.blob_sha256,
                    b.original_size,
                    r.position,
                    r.purpose,
                    r.created_at
                 FROM attachment_references r
                 JOIN attachment_versions v
                    ON v.id = r.attachment_version_id
                 JOIN blobs b
                    ON b.sha256 = v.blob_sha256
                 WHERE r.chat_id = ?1 AND r.purpose = 'composer'
                 ORDER BY r.position ASC, r.created_at ASC",
            )
            .map_err(|error| format!("Cannot prepare draft attachment query: {error}"))?;

        let rows = statement
            .query_map(params![chat_id], |row| {
                Ok(AttachmentReferenceRecord {
                    id: row.get(0)?,
                    chat_id: row.get(1)?,
                    message_id: row.get(2)?,
                    attachment_version_id: row.get(3)?,
                    attachment_id: row.get(4)?,
                    version_number: row.get(5)?,
                    file_name: row.get(6)?,
                    extension: row.get(7)?,
                    mime_type: row.get(8)?,
                    blob_sha256: row.get(9)?,
                    original_size: row.get(10)?,
                    position: row.get(11)?,
                    purpose: row.get(12)?,
                    created_at: row.get(13)?,
                })
            })
            .map_err(|error| format!("Cannot list draft attachments: {error}"))?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("Cannot read draft attachment records: {error}"))
    }

    /// Removes a reference and safely cleans up unreferenced attachment data.
    pub fn remove_attachment_reference(&mut self, reference_id: &str) -> Result<bool, String> {
        let transaction = self
            .connection
            .transaction()
            .map_err(|error| format!("Cannot start attachment removal: {error}"))?;

        let target: Option<(String, String, String)> = transaction
            .query_row(
                "SELECT v.id, v.attachment_id, v.blob_sha256
                 FROM attachment_references r
                 JOIN attachment_versions v ON v.id = r.attachment_version_id
                 WHERE r.id = ?1",
                params![reference_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .map_err(|error| format!("Cannot resolve attachment reference: {error}"))?;

        let Some((version_id, attachment_id, blob_sha256)) = target else {
            return Ok(false);
        };

        transaction
            .execute(
                "DELETE FROM attachment_references WHERE id = ?1",
                params![reference_id],
            )
            .map_err(|error| format!("Cannot remove attachment reference: {error}"))?;

        let version_is_referenced: bool = transaction
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM attachment_references
                    WHERE attachment_version_id = ?1
                )",
                params![version_id],
                |row| row.get(0),
            )
            .map_err(|error| format!("Cannot check attachment version references: {error}"))?;

        if !version_is_referenced {
            transaction
                .execute(
                    "DELETE FROM attachment_versions WHERE id = ?1",
                    params![version_id],
                )
                .map_err(|error| format!("Cannot remove unused attachment version: {error}"))?;

            let attachment_has_versions: bool = transaction
                .query_row(
                    "SELECT EXISTS(
                        SELECT 1 FROM attachment_versions WHERE attachment_id = ?1
                    )",
                    params![attachment_id],
                    |row| row.get(0),
                )
                .map_err(|error| format!("Cannot check attachment versions: {error}"))?;

            if !attachment_has_versions {
                transaction
                    .execute(
                        "DELETE FROM attachments WHERE id = ?1",
                        params![attachment_id],
                    )
                    .map_err(|error| format!("Cannot remove empty attachment: {error}"))?;
            }
        }

        transaction
            .commit()
            .map_err(|error| format!("Cannot commit attachment removal: {error}"))?;

        self.delete_unreferenced_blob(&blob_sha256)?;

        Ok(true)
    }
}

#[derive(Debug, Clone)]
pub struct PreviewBlobRecord {
    pub relative_path: String,
    pub file_name: String,
    pub mime_type: String,
    pub original_size: i64,
}

impl Storage {
    /// Resolves a currently authorized attachment reference for preview.
    pub fn get_preview_blob(
        &self,
        chat_id: &str,
        reference_id: &str,
    ) -> Result<PreviewBlobRecord, String> {
        self.connection
            .query_row(
                "SELECT
                    b.relative_path,
                    v.file_name,
                    v.mime_type,
                    b.original_size
                 FROM attachment_references r
                 JOIN attachment_versions v
                    ON v.id = r.attachment_version_id
                 JOIN blobs b
                    ON b.sha256 = v.blob_sha256
                 WHERE r.id = ?1
                   AND r.chat_id = ?2
                   AND b.storage_state = 'original'",
                params![reference_id, chat_id],
                |row| {
                    Ok(PreviewBlobRecord {
                        relative_path: row.get(0)?,
                        file_name: row.get(1)?,
                        mime_type: row.get(2)?,
                        original_size: row.get(3)?,
                    })
                },
            )
            .optional()
            .map_err(|error| format!("Cannot resolve preview attachment: {error}"))?
            .ok_or_else(|| "Attachment is unavailable for preview".to_string())
    }
}

impl Storage {
    pub fn promote_composer_attachments(
        &mut self,
        chat_id: &str,
        message_id: &str,
    ) -> Result<Vec<AttachmentReferenceRecord>, String> {
        let transaction = self
            .connection
            .transaction()
            .map_err(|error| format!("Cannot start attachment promotion: {error}"))?;

        let message_exists: bool = transaction
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM messages
                    WHERE id = ?1 AND chat_id = ?2
                )",
                rusqlite::params![message_id, chat_id],
                |row| row.get(0),
            )
            .map_err(|error| format!("Cannot verify target message: {error}"))?;

        if !message_exists {
            return Err("Target message does not belong to the specified chat".into());
        }

        transaction
            .execute(
                "UPDATE attachment_references
                 SET message_id = ?1, purpose = 'message'
                 WHERE chat_id = ?2 AND purpose = 'composer'",
                rusqlite::params![message_id, chat_id],
            )
            .map_err(|error| format!("Cannot promote composer attachments: {error}"))?;

        transaction
            .commit()
            .map_err(|error| format!("Cannot commit attachment promotion: {error}"))?;

        self.list_message_attachments(message_id)
    }
}

#[tauri::command]
pub fn promote_composer_attachments(
    state: tauri::State<'_, std::sync::Mutex<Storage>>,
    chat_id: String,
    message_id: String,
) -> Result<Vec<AttachmentReferenceRecord>, String> {
    let mut storage = state
        .lock()
        .map_err(|error| format!("Storage lock poisoned: {error}"))?;

    storage.promote_composer_attachments(&chat_id, &message_id)
}

#[tauri::command]
pub fn read_attachment_chunk(
    state: tauri::State<'_, std::sync::Mutex<Storage>>,
    chat_id: String,
    reference_id: String,
    offset: u64,
    length: usize,
) -> Result<Vec<u8>, String> {
    const MAX_CHUNK_SIZE: usize = 256 * 1024;

    if length == 0 || length > MAX_CHUNK_SIZE {
        return Err(format!(
            "Chunk length must be between 1 and {} bytes",
            MAX_CHUNK_SIZE
        ));
    }

    let storage = state
        .lock()
        .map_err(|error| format!("Storage lock poisoned: {error}"))?;

    // The reference is resolved within the requested chat.
    let blob = storage.get_preview_blob(&chat_id, &reference_id)?;
    let path = storage.resolve_blob_path(&blob.relative_path)?;

    use std::io::{Read, Seek, SeekFrom};

    let mut file =
        std::fs::File::open(&path).map_err(|error| format!("Cannot open attachment: {error}"))?;

    let file_size = file
        .metadata()
        .map_err(|error| format!("Cannot inspect attachment: {error}"))?
        .len();

    if offset > file_size {
        return Err("Attachment offset is out of bounds".to_owned());
    }

    let bytes_to_read = (file_size - offset).min(length as u64) as usize;

    file.seek(SeekFrom::Start(offset))
        .map_err(|error| format!("Cannot seek attachment: {error}"))?;

    let mut buffer = vec![0u8; bytes_to_read];
    file.read_exact(&mut buffer)
        .map_err(|error| format!("Cannot read attachment chunk: {error}"))?;

    Ok(buffer)
}
