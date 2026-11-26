use rusqlite::{params, OptionalExtension};
use serde::Serialize;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

use super::Storage;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatRecord {
    pub id: String,
    pub title: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageRecord {
    pub id: String,
    pub chat_id: String,
    pub role: String,
    pub content: String,
    pub prompt_content: Option<String>,
    pub model_used: Option<String>,
    pub mode_used: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartTurnRecord {
    pub user_message: MessageRecord,
    pub assistant_message: MessageRecord,
    pub attachments: Vec<super::AttachmentReferenceRecord>,
}

fn now_timestamp() -> Result<i64, String> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("System clock error: {error}"))?;

    i64::try_from(duration.as_millis()).map_err(|error| format!("Timestamp overflow: {error}"))
}

impl Storage {
    pub fn create_chat(&mut self, title: &str) -> Result<ChatRecord, String> {
        let id = Uuid::new_v4().to_string();
        let timestamp = now_timestamp()?;

        self.connection
            .execute(
                "INSERT INTO chats (id, title, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?3)",
                params![id, title, timestamp],
            )
            .map_err(|error| format!("Cannot create chat: {error}"))?;

        Ok(ChatRecord {
            id,
            title: title.to_owned(),
            created_at: timestamp,
            updated_at: timestamp,
        })
    }

    pub fn get_chat(&self, chat_id: &str) -> Result<Option<ChatRecord>, String> {
        self.connection
            .query_row(
                "SELECT id, title, created_at, updated_at
                 FROM chats WHERE id = ?1",
                params![chat_id],
                |row| {
                    Ok(ChatRecord {
                        id: row.get(0)?,
                        title: row.get(1)?,
                        created_at: row.get(2)?,
                        updated_at: row.get(3)?,
                    })
                },
            )
            .optional()
            .map_err(|error| format!("Cannot retrieve chat: {error}"))
    }

    pub fn list_chats(&self) -> Result<Vec<ChatRecord>, String> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT id, title, created_at, updated_at
                 FROM chats
                 ORDER BY updated_at DESC",
            )
            .map_err(|error| format!("Cannot prepare chat query: {error}"))?;

        let rows = statement
            .query_map([], |row| {
                Ok(ChatRecord {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    created_at: row.get(2)?,
                    updated_at: row.get(3)?,
                })
            })
            .map_err(|error| format!("Cannot list chats: {error}"))?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("Cannot read chat records: {error}"))
    }

    pub fn start_turn(
        &mut self,
        chat_id: &str,
        rewind_message_id: Option<&str>,
        user_content: &str,
        prompt_content: &str,
        assistant_content: &str,
        assistant_model: Option<&str>,
        assistant_mode: Option<&str>,
    ) -> Result<StartTurnRecord, String> {
        let timestamp = now_timestamp()?;

        let user_id = Uuid::new_v4().to_string();
        let assistant_id = Uuid::new_v4().to_string();

        let transaction = self
            .connection
            .transaction()
            .map_err(|e| format!("Cannot start turn transaction: {e}"))?;

        let chat_exists: bool = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM chats WHERE id = ?1)",
                params![chat_id],
                |row| row.get(0),
            )
            .map_err(|e| format!("Cannot verify chat: {e}"))?;

        if !chat_exists {
            return Err(format!("Chat not found: {chat_id}"));
        }

        if let Some(message_id) = rewind_message_id {
            let (target_rowid, role): (i64, String) = transaction
                .query_row(
                    "SELECT rowid, role FROM messages WHERE id = ?1 AND chat_id = ?2",
                    params![message_id, chat_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .map_err(|e| format!("Cannot find message to edit: {e}"))?;

            if role != "user" {
                return Err("Only user messages can be edited".to_string());
            }

            transaction
                .execute(
                    "UPDATE attachment_references
                 SET message_id = NULL, purpose = 'composer'
                 WHERE message_id = ?1 AND chat_id = ?2",
                    params![message_id, chat_id],
                )
                .map_err(|e| format!("Cannot restore edited attachments: {e}"))?;

            transaction
                .execute(
                    "DELETE FROM messages WHERE chat_id = ?1 AND rowid >= ?2",
                    params![chat_id, target_rowid],
                )
                .map_err(|e| format!("Cannot truncate conversation: {e}"))?;
        }

        transaction
            .execute(
                "INSERT INTO messages (
                id, chat_id, role, content, prompt_content,
                model_used, mode_used, created_at
            ) VALUES (?1, ?2, 'user', ?3, ?4, NULL, NULL, ?5)",
                params![user_id, chat_id, user_content, prompt_content, timestamp],
            )
            .map_err(|e| format!("Cannot save user message: {e}"))?;

        transaction
            .execute(
                "UPDATE attachment_references
             SET message_id = ?1, purpose = 'message'
             WHERE chat_id = ?2 AND purpose = 'composer'",
                params![user_id, chat_id],
            )
            .map_err(|e| format!("Cannot promote attachments: {e}"))?;

        transaction
            .execute(
                "INSERT INTO messages (
                id, chat_id, role, content, prompt_content,
                model_used, mode_used, created_at
            ) VALUES (?1, ?2, 'assistant', ?3, NULL, ?4, ?5, ?6)",
                params![
                    assistant_id,
                    chat_id,
                    assistant_content,
                    assistant_model,
                    assistant_mode,
                    timestamp
                ],
            )
            .map_err(|e| format!("Cannot create assistant message: {e}"))?;

        transaction
            .execute(
                "UPDATE chats SET updated_at = ?1 WHERE id = ?2",
                params![timestamp, chat_id],
            )
            .map_err(|e| format!("Cannot update chat timestamp: {e}"))?;

        let attachments = {
            let mut statement = transaction
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
                .map_err(|e| format!("Cannot prepare turn attachment query: {e}"))?;

            let rows = statement
                .query_map(params![user_id], |row| {
                    Ok(super::AttachmentReferenceRecord {
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
                .map_err(|e| format!("Cannot query turn attachments: {e}"))?;

            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("Cannot read turn attachments: {e}"))?
        };

        transaction
            .commit()
            .map_err(|e| format!("Cannot commit turn: {e}"))?;

        Ok(StartTurnRecord {
            user_message: MessageRecord {
                id: user_id,
                chat_id: chat_id.to_owned(),
                role: "user".to_string(),
                content: user_content.to_owned(),
                prompt_content: Some(prompt_content.to_owned()),
                model_used: None,
                mode_used: None,
                created_at: timestamp,
            },
            assistant_message: MessageRecord {
                id: assistant_id,
                chat_id: chat_id.to_owned(),
                role: "assistant".to_string(),
                content: assistant_content.to_owned(),
                prompt_content: None,
                model_used: assistant_model.map(str::to_owned),
                mode_used: assistant_mode.map(str::to_owned),
                created_at: timestamp,
            },
            attachments,
        })
    }

    pub fn save_message(
        &mut self,
        chat_id: &str,
        role: &str,
        content: &str,
        prompt_content: Option<&str>,
        model_used: Option<&str>,
        mode_used: Option<&str>,
    ) -> Result<MessageRecord, String> {
        if !matches!(role, "user" | "assistant" | "system") {
            return Err(format!("Unsupported message role: {role}"));
        }

        let id = Uuid::new_v4().to_string();
        let timestamp = now_timestamp()?;

        let transaction = self
            .connection
            .transaction()
            .map_err(|error| format!("Cannot start message transaction: {error}"))?;

        let chat_exists: bool = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM chats WHERE id = ?1)",
                params![chat_id],
                |row| row.get(0),
            )
            .map_err(|error| format!("Cannot verify chat: {error}"))?;

        if !chat_exists {
            return Err(format!("Chat not found: {chat_id}"));
        }

        transaction
            .execute(
                "INSERT INTO messages (
                    id, chat_id, role, content, prompt_content,
                    model_used, mode_used, created_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    id,
                    chat_id,
                    role,
                    content,
                    prompt_content,
                    model_used,
                    mode_used,
                    timestamp
                ],
            )
            .map_err(|error| format!("Cannot save message: {error}"))?;

        transaction
            .execute(
                "UPDATE chats SET updated_at = ?1 WHERE id = ?2",
                params![timestamp, chat_id],
            )
            .map_err(|error| format!("Cannot update chat timestamp: {error}"))?;

        transaction
            .commit()
            .map_err(|error| format!("Cannot commit message transaction: {error}"))?;

        Ok(MessageRecord {
            id,
            chat_id: chat_id.to_owned(),
            role: role.to_owned(),
            content: content.to_owned(),
            prompt_content: prompt_content.map(str::to_owned),
            model_used: model_used.map(str::to_owned),
            mode_used: mode_used.map(str::to_owned),
            created_at: timestamp,
        })
    }

    pub fn list_messages(&self, chat_id: &str) -> Result<Vec<MessageRecord>, String> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT id, chat_id, role, content, prompt_content,
                        model_used, mode_used, created_at
                 FROM messages
                 WHERE chat_id = ?1
                 ORDER BY created_at ASC, rowid ASC",
            )
            .map_err(|error| format!("Cannot prepare message query: {error}"))?;

        let rows = statement
            .query_map(params![chat_id], |row| {
                Ok(MessageRecord {
                    id: row.get(0)?,
                    chat_id: row.get(1)?,
                    role: row.get(2)?,
                    content: row.get(3)?,
                    prompt_content: row.get(4)?,
                    model_used: row.get(5)?,
                    mode_used: row.get(6)?,
                    created_at: row.get(7)?,
                })
            })
            .map_err(|error| format!("Cannot list messages: {error}"))?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("Cannot read message records: {error}"))
    }

    pub fn update_message(
        &mut self,
        message_id: &str,
        content: &str,
        prompt_content: Option<&str>,
        model_used: Option<&str>,
        mode_used: Option<&str>,
    ) -> Result<MessageRecord, String> {
        let timestamp = now_timestamp()?;
        let transaction = self
            .connection
            .transaction()
            .map_err(|error| format!("Cannot start message update: {error}"))?;

        let chat_id: String = transaction
            .query_row(
                "SELECT chat_id FROM messages WHERE id = ?1",
                params![message_id],
                |row| row.get(0),
            )
            .map_err(|error| format!("Cannot find message: {error}"))?;

        transaction
            .execute(
                "UPDATE messages
             SET content = ?1, prompt_content = ?2, model_used = ?3, mode_used = ?4
             WHERE id = ?5",
                params![content, prompt_content, model_used, mode_used, message_id],
            )
            .map_err(|error| format!("Cannot update message: {error}"))?;

        transaction
            .execute(
                "UPDATE chats SET updated_at = ?1 WHERE id = ?2",
                params![timestamp, chat_id],
            )
            .map_err(|error| format!("Cannot update chat timestamp: {error}"))?;

        let message = transaction
            .query_row(
                "SELECT id, chat_id, role, content, prompt_content,
                    model_used, mode_used, created_at
             FROM messages WHERE id = ?1",
                params![message_id],
                |row| {
                    Ok(MessageRecord {
                        id: row.get(0)?,
                        chat_id: row.get(1)?,
                        role: row.get(2)?,
                        content: row.get(3)?,
                        prompt_content: row.get(4)?,
                        model_used: row.get(5)?,
                        mode_used: row.get(6)?,
                        created_at: row.get(7)?,
                    })
                },
            )
            .map_err(|error| format!("Cannot retrieve updated message: {error}"))?;

        transaction
            .commit()
            .map_err(|error| format!("Cannot commit message update: {error}"))?;

        Ok(message)
    }

    pub fn rewind_chat(&mut self, chat_id: &str, message_id: &str) -> Result<(), String> {
        let timestamp = now_timestamp()?;
        let transaction = self
            .connection
            .transaction()
            .map_err(|error| format!("Cannot start chat rewind: {error}"))?;

        let (target_rowid, role): (i64, String) = transaction
            .query_row(
                "SELECT rowid, role FROM messages WHERE id = ?1 AND chat_id = ?2",
                params![message_id, chat_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|error| format!("Cannot find message in chat: {error}"))?;

        if role != "user" {
            return Err("Only user messages can be edited".to_string());
        }

        transaction
            .execute(
                "UPDATE attachment_references
             SET message_id = NULL, purpose = 'composer'
             WHERE message_id = ?1 AND chat_id = ?2",
                params![message_id, chat_id],
            )
            .map_err(|error| format!("Cannot restore message attachments: {error}"))?;

        transaction
            .execute(
                "DELETE FROM messages WHERE chat_id = ?1 AND rowid >= ?2",
                params![chat_id, target_rowid],
            )
            .map_err(|error| format!("Cannot remove later messages: {error}"))?;

        transaction
            .execute(
                "UPDATE chats SET updated_at = ?1 WHERE id = ?2",
                params![timestamp, chat_id],
            )
            .map_err(|error| format!("Cannot update chat timestamp: {error}"))?;

        transaction
            .commit()
            .map_err(|error| format!("Cannot commit chat rewind: {error}"))?;

        Ok(())
    }

    pub fn delete_chat(&mut self, chat_id: &str) -> Result<bool, String> {
        let transaction = self
            .connection
            .transaction()
            .map_err(|error| format!("Cannot start chat deletion: {error}"))?;

        let affected = {
            let mut statement = transaction
                .prepare(
                    "SELECT DISTINCT v.id, v.attachment_id, v.blob_sha256
                 FROM attachment_references r
                 JOIN attachment_versions v ON v.id = r.attachment_version_id
                 WHERE r.chat_id = ?1",
                )
                .map_err(|error| format!("Cannot prepare chat attachment cleanup: {error}"))?;

            let rows = statement
                .query_map(params![chat_id], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                })
                .map_err(|error| format!("Cannot query chat attachments: {error}"))?;

            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|error| format!("Cannot collect chat attachments: {error}"))?
        };

        let deleted = transaction
            .execute("DELETE FROM chats WHERE id = ?1", params![chat_id])
            .map_err(|error| format!("Cannot delete chat: {error}"))?;

        if deleted == 0 {
            transaction
                .commit()
                .map_err(|error| format!("Cannot finish chat deletion: {error}"))?;
            return Ok(false);
        }

        let mut candidate_blobs = affected
            .iter()
            .map(|(_, _, sha256)| sha256.clone())
            .collect::<Vec<_>>();
        candidate_blobs.sort();
        candidate_blobs.dedup();

        for (version_id, attachment_id, _) in affected {
            transaction
                .execute(
                    "DELETE FROM attachment_versions
                 WHERE id = ?1
                   AND NOT EXISTS (
                       SELECT 1 FROM attachment_references
                       WHERE attachment_version_id = ?1
                   )",
                    params![version_id],
                )
                .map_err(|error| format!("Cannot remove orphaned attachment version: {error}"))?;

            transaction
                .execute(
                    "DELETE FROM attachments
                 WHERE id = ?1
                   AND NOT EXISTS (
                       SELECT 1 FROM attachment_versions
                       WHERE attachment_id = ?1
                   )",
                    params![attachment_id],
                )
                .map_err(|error| format!("Cannot remove empty attachment: {error}"))?;
        }

        let timestamp = now_timestamp()?;
        let mut pending_deletions = Vec::new();

        for sha256 in candidate_blobs {
            let relative_path: Option<String> = transaction
                .query_row(
                    "SELECT relative_path FROM blobs
                 WHERE sha256 = ?1
                   AND NOT EXISTS (SELECT 1 FROM attachment_versions WHERE blob_sha256 = ?1)
                   AND NOT EXISTS (SELECT 1 FROM document_assets WHERE blob_sha256 = ?1)
                   AND NOT EXISTS (SELECT 1 FROM archive_entries WHERE blob_sha256 = ?1)
                   AND NOT EXISTS (SELECT 1 FROM extracted_documents WHERE blob_sha256 = ?1)",
                    params![sha256],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|error| format!("Cannot resolve unreferenced chat blob: {error}"))?;

            let Some(relative_path) = relative_path else {
                continue;
            };

            let operation_id = Uuid::new_v4().to_string();

            transaction
                .execute(
                    "INSERT INTO storage_operations (
                    id, operation_type, blob_sha256, target_path,
                    state, created_at, updated_at
                 ) VALUES (?1, 'delete_blob', ?2, ?3, 'PENDING', ?4, ?4)",
                    params![operation_id, sha256, relative_path, timestamp],
                )
                .map_err(|error| format!("Cannot journal chat blob deletion: {error}"))?;

            let removed = transaction
                .execute(
                    "DELETE FROM blobs
                 WHERE sha256 = ?1
                   AND NOT EXISTS (SELECT 1 FROM attachment_versions WHERE blob_sha256 = ?1)
                   AND NOT EXISTS (SELECT 1 FROM document_assets WHERE blob_sha256 = ?1)
                   AND NOT EXISTS (SELECT 1 FROM archive_entries WHERE blob_sha256 = ?1)
                   AND NOT EXISTS (SELECT 1 FROM extracted_documents WHERE blob_sha256 = ?1)",
                    params![sha256],
                )
                .map_err(|error| format!("Cannot remove chat blob metadata: {error}"))?;

            if removed == 1 {
                pending_deletions.push((operation_id, relative_path));
            } else {
                transaction
                    .execute(
                        "UPDATE storage_operations
                     SET state = 'COMPLETED', updated_at = ?1
                     WHERE id = ?2",
                        params![timestamp, operation_id],
                    )
                    .map_err(|error| format!("Cannot close unused deletion journal: {error}"))?;
            }
        }

        transaction
            .commit()
            .map_err(|error| format!("Cannot commit chat deletion: {error}"))?;

        for (operation_id, relative_path) in pending_deletions {
            super::blobs::remove_recovered_blob_file(&self.root_dir, &relative_path)?;

            self.connection
                .execute(
                    "UPDATE storage_operations
                 SET state = 'COMPLETED', error_message = NULL, updated_at = ?1
                 WHERE id = ?2 AND state = 'PENDING'",
                    params![now_timestamp()?, operation_id],
                )
                .map_err(|error| format!("Cannot complete chat blob deletion journal: {error}"))?;
        }

        Ok(true)
    }
}
