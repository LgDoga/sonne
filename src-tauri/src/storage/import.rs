use std::{
    fs::File,
    io::Cursor,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

use rusqlite::params;
use serde_json::json;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_dialog::DialogExt;
use uuid::Uuid;

use super::{
    AttachmentReferenceRecord, MAX_FILE_SIZE_BYTES, MAX_IMPORT_BATCH_SIZE_BYTES, MAX_IMPORT_FILES,
    Storage,
};

fn now_timestamp() -> Result<i64, String> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("System clock error: {error}"))?;

    i64::try_from(duration.as_millis()).map_err(|error| format!("Timestamp overflow: {error}"))
}

fn file_metadata(path: &Path) -> Result<(String, String, String), String> {
    let file_name = path
        .file_name()
        .ok_or_else(|| "Selected file has no filename".to_string())?
        .to_string_lossy()
        .into_owned();

    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    let mime_type = match extension.as_str() {
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "tif" | "tiff" => "image/tiff",
        "avif" => "image/avif",
        "heic" | "heif" => "image/heic",
        "txt" | "log" | "csv" | "tsv" => "text/plain",
        "md" | "markdown" => "text/markdown",
        "html" | "htm" => "text/html",
        "json" => "application/json",
        "xml" => "application/xml",
        "js" | "mjs" | "cjs" => "text/javascript",
        "ts" | "tsx" => "text/typescript",
        "css" => "text/css",
        "rs" => "text/rust",
        "py" => "text/x-python",
        "java" => "text/x-java-source",
        "c" | "h" => "text/x-c",
        "cpp" | "hpp" | "cc" => "text/x-c++",
        "sh" | "bash" => "text/x-shellscript",
        "yaml" | "yml" => "application/yaml",
        "toml" => "application/toml",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "odt" => "application/vnd.oasis.opendocument.text",
        "ods" => "application/vnd.oasis.opendocument.spreadsheet",
        "odp" => "application/vnd.oasis.opendocument.presentation",
        "epub" => "application/epub+zip",
        "zip" => "application/zip",
        "gz" => "application/gzip",
        "tar" => "application/x-tar",
        "rtf" => "application/rtf",
        _ => "application/octet-stream",
    };

    Ok((file_name, extension, mime_type.to_string()))
}

struct PreparedAttachment {
    blob_sha256: String,
    file_name: String,
    extension: String,
    mime_type: String,
}

impl Storage {
    fn import_selected_paths(
        &mut self,
        chat_id: &str,
        paths: Vec<PathBuf>,
    ) -> Result<Vec<AttachmentReferenceRecord>, String> {
        if paths.is_empty() {
            return self.list_composer_attachments(chat_id);
        }

        if paths.len() > MAX_IMPORT_FILES {
            return Err(format!(
                "An import batch cannot contain more than {MAX_IMPORT_FILES} files"
            ));
        }

        let mut declared_batch_size = 0u64;

        for path in &paths {
            let metadata = std::fs::metadata(path)
                .map_err(|error| format!("Cannot inspect file {}: {error}", path.display()))?;

            if !metadata.is_file() {
                return Err(format!("Not a regular file: {}", path.display()));
            }

            let file_size = metadata.len();

            if file_size > MAX_FILE_SIZE_BYTES {
                return Err(format!(
                    "File {} exceeds the maximum size of {} MiB",
                    path.display(),
                    MAX_FILE_SIZE_BYTES / (1024 * 1024)
                ));
            }

            declared_batch_size = declared_batch_size
                .checked_add(file_size)
                .ok_or_else(|| "Import batch size overflow".to_string())?;

            if declared_batch_size > MAX_IMPORT_BATCH_SIZE_BYTES {
                return Err(format!(
                    "The total import batch exceeds the maximum size of {} MiB",
                    MAX_IMPORT_BATCH_SIZE_BYTES / (1024 * 1024)
                ));
            }
        }

        let timestamp = now_timestamp()?;
        let operation_id = Uuid::new_v4().to_string();

        let chat_exists: bool = self
            .connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM chats WHERE id = ?1)",
                params![chat_id],
                |row| row.get(0),
            )
            .map_err(|error| format!("Cannot verify import chat: {error}"))?;

        if !chat_exists {
            return Err(format!("Chat not found: {chat_id}"));
        }

        let existing = self.list_composer_attachments(chat_id)?;
        let mut position = i64::try_from(existing.len())
            .map_err(|error| format!("Attachment position overflow: {error}"))?;

        self.connection
            .execute(
                "INSERT INTO storage_operations (
                    id, operation_type, state, metadata_json,
                    created_at, updated_at
                ) VALUES (?1, 'import_batch', 'PENDING', ?2, ?3, ?3)",
                params![
                    operation_id,
                    json!({
                        "chatId": chat_id,
                        "files": []
                    })
                    .to_string(),
                    timestamp
                ],
            )
            .map_err(|error| format!("Cannot create import journal: {error}"))?;

        self.connection
            .execute(
                "UPDATE storage_operations
                 SET state = 'PROCESSING', updated_at = ?1
                 WHERE id = ?2",
                params![now_timestamp()?, operation_id],
            )
            .map_err(|error| format!("Cannot start import journal: {error}"))?;

        let mut prepared = Vec::with_capacity(paths.len());
        let preparation = (|| -> Result<(), String> {
            let mut imported_size = 0u64;

            for path in paths {
                let metadata = std::fs::metadata(&path)
                    .map_err(|error| format!("Cannot inspect file {}: {error}", path.display()))?;

                if !metadata.is_file() {
                    return Err(format!("Not a regular file: {}", path.display()));
                }

                let (file_name, extension, mime_type) = file_metadata(&path)?;
                let file = File::open(&path)
                    .map_err(|error| format!("Cannot open {file_name}: {error}"))?;

                let remaining_batch_size =
                    MAX_IMPORT_BATCH_SIZE_BYTES.saturating_sub(imported_size);
                let effective_limit = MAX_FILE_SIZE_BYTES.min(remaining_batch_size);
                let blob = self.store_blob_with_limit(file, effective_limit)?;

                imported_size = imported_size
                    .checked_add(
                        u64::try_from(blob.original_size)
                            .map_err(|error| format!("Invalid stored blob size: {error}"))?,
                    )
                    .ok_or_else(|| "Import batch size overflow".to_string())?;

                prepared.push(PreparedAttachment {
                    blob_sha256: blob.sha256,
                    file_name,
                    extension,
                    mime_type,
                });

                let files_json: Vec<_> = prepared
                    .iter()
                    .map(|item| {
                        json!({
                            "blobSha256": item.blob_sha256,
                            "fileName": item.file_name,
                            "extension": item.extension,
                            "mimeType": item.mime_type
                        })
                    })
                    .collect();

                self.connection
                    .execute(
                        "UPDATE storage_operations
                         SET metadata_json = ?1, updated_at = ?2
                         WHERE id = ?3",
                        params![
                            json!({
                                "chatId": chat_id,
                                "files": files_json
                            })
                            .to_string(),
                            now_timestamp()?,
                            operation_id
                        ],
                    )
                    .map_err(|error| format!("Cannot update import journal: {error}"))?;
            }

            Ok(())
        })();

        if let Err(error) = preparation {
            let _ = self.connection.execute(
                "UPDATE storage_operations
                 SET state = 'FAILED', error_message = ?1, updated_at = ?2
                 WHERE id = ?3",
                params![error, now_timestamp().unwrap_or(timestamp), operation_id],
            );

            for item in &prepared {
                if let Err(cleanup_error) = self.delete_unreferenced_blob(&item.blob_sha256) {
                    eprintln!(
                        "Failed to clean up partial import blob {}: {}",
                        item.blob_sha256, cleanup_error
                    );
                }
            }

            return Err(error);
        }

        let transaction = self
            .connection
            .transaction()
            .map_err(|error| format!("Cannot start atomic attachment transaction: {error}"))?;

        let commit_result = (|| -> Result<(), String> {
            for item in &prepared {
                let version = super::attachments::create_attachment_version_in_transaction(
                    &transaction,
                    None,
                    &item.blob_sha256,
                    &item.file_name,
                    &item.extension,
                    &item.mime_type,
                    timestamp,
                )?;

                super::attachments::add_attachment_reference_in_transaction(
                    &transaction,
                    chat_id,
                    &version.id,
                    None,
                    "composer",
                    position,
                    timestamp,
                )?;

                position = position
                    .checked_add(1)
                    .ok_or_else(|| "Attachment position overflow".to_string())?;
            }

            transaction
                .execute(
                    "UPDATE storage_operations
                     SET state = 'COMPLETED', updated_at = ?1, error_message = NULL
                     WHERE id = ?2",
                    params![now_timestamp()?, operation_id],
                )
                .map_err(|error| format!("Cannot complete import journal: {error}"))?;

            Ok(())
        })();

        if let Err(error) = commit_result {
            drop(transaction);

            let _ = self.connection.execute(
                "UPDATE storage_operations
                 SET state = 'FAILED', error_message = ?1, updated_at = ?2
                 WHERE id = ?3",
                params![error, now_timestamp().unwrap_or(timestamp), operation_id],
            );

            for item in &prepared {
                if let Err(cleanup_error) = self.delete_unreferenced_blob(&item.blob_sha256) {
                    eprintln!("Failed to clean up import blob {}: {}", item.blob_sha256, cleanup_error);
                }
            }

            return Err(error);
        }

        if let Err(error) = transaction.commit() {
            let message = format!("Cannot commit imported attachments: {error}");
            let _ = self.connection.execute(
                "UPDATE storage_operations SET state = 'FAILED', error_message = ?1, updated_at = ?2
                 WHERE id = ?3",
                params![message, now_timestamp().unwrap_or(timestamp), operation_id],
            );
            for item in &prepared {
                if let Err(cleanup_error) = self.delete_unreferenced_blob(&item.blob_sha256) {
                    eprintln!("Failed to clean up import blob {}: {}", item.blob_sha256, cleanup_error);
                }
            }
            return Err(message);
        }

        for item in &prepared {
            if super::archive::is_supported_archive_extension(&item.extension, &item.file_name) {
                if let Err(error) = super::archive::index_imported_archive(
                    self,
                    &item.blob_sha256,
                    &item.file_name,
                    &item.extension,
                ) {
                    eprintln!("Archive indexing failed for {}: {}", item.file_name, error);
                }
            }
        }

        self.list_composer_attachments(chat_id)
    }
}

impl Storage {
    fn import_clipboard_image(
        &mut self,
        chat_id: &str,
        requested_mime: &str,
        data: Vec<u8>,
    ) -> Result<Vec<AttachmentReferenceRecord>, String> {
        if data.is_empty() {
            return Err("Clipboard image is empty".to_string());
        }

        let data_size = u64::try_from(data.len())
            .map_err(|error| format!("Invalid clipboard image size: {error}"))?;

        if data_size > MAX_FILE_SIZE_BYTES {
            return Err(format!(
                "Clipboard image exceeds the maximum size of {} MiB",
                MAX_FILE_SIZE_BYTES / (1024 * 1024)
            ));
        }

        let (extension, mime_type) = match requested_mime.trim().to_ascii_lowercase().as_str() {
            "image/png" => ("png", "image/png"),
            "image/jpeg" => ("jpg", "image/jpeg"),
            "image/webp" => ("webp", "image/webp"),
            "image/gif" => ("gif", "image/gif"),
            "image/bmp" => ("bmp", "image/bmp"),
            "image/tiff" => ("tif", "image/tiff"),
            "image/avif" => ("avif", "image/avif"),
            "image/heic" => ("heic", "image/heic"),
            "image/heif" => ("heif", "image/heif"),
            _ => return Err("Unsupported clipboard image format".to_string()),
        };

        let chat_exists: bool = self
            .connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM chats WHERE id = ?1)",
                params![chat_id],
                |row| row.get(0),
            )
            .map_err(|error| format!("Cannot verify clipboard target chat: {error}"))?;

        if !chat_exists {
            return Err(format!("Chat not found: {chat_id}"));
        }

        let timestamp = now_timestamp()?;
        let file_name = format!("clipboard-{timestamp}.{extension}");
        let existing = self.list_composer_attachments(chat_id)?;
        let position = i64::try_from(existing.len())
            .map_err(|error| format!("Attachment position overflow: {error}"))?;

        let blob = self.store_blob_with_limit(Cursor::new(data), MAX_FILE_SIZE_BYTES)?;

        let transaction = self
            .connection
            .transaction()
            .map_err(|error| format!("Cannot start clipboard attachment transaction: {error}"))?;

        let result = (|| -> Result<(), String> {
            let version = super::attachments::create_attachment_version_in_transaction(
                &transaction,
                None,
                &blob.sha256,
                &file_name,
                extension,
                mime_type,
                timestamp,
            )?;

            super::attachments::add_attachment_reference_in_transaction(
                &transaction,
                chat_id,
                &version.id,
                None,
                "composer",
                position,
                timestamp,
            )?;

            Ok(())
        })();

        if let Err(error) = result {
            drop(transaction);
            let _ = self.delete_unreferenced_blob(&blob.sha256);
            return Err(error);
        }

        if let Err(error) = transaction.commit() {
            let _ = self.delete_unreferenced_blob(&blob.sha256);
            return Err(format!("Cannot commit clipboard image attachment: {error}"));
        }

        self.list_composer_attachments(chat_id)
    }
}

#[tauri::command]
pub fn import_clipboard_image(
    state: tauri::State<'_, Mutex<Storage>>,
    chat_id: String,
    mime_type: String,
    data: Vec<u8>,
) -> Result<Vec<AttachmentReferenceRecord>, String> {
    let mut storage = state
        .lock()
        .map_err(|error| format!("Storage lock poisoned: {error}"))?;

    storage.import_clipboard_image(&chat_id, &mime_type, data)
}

fn process_selected_paths(app: AppHandle, paths: Vec<PathBuf>, chat_id: String) {
    tauri::async_runtime::spawn_blocking(move || {
        let result = (|| -> Result<Vec<AttachmentReferenceRecord>, String> {
            let state = app.state::<Mutex<Storage>>();
            let mut storage = state
                .lock()
                .map_err(|error| format!("Storage lock poisoned: {error}"))?;

            storage.import_selected_paths(&chat_id, paths)
        })();

        match result {
            Ok(attachments) => {
                let _ = app.emit("attachments-imported", attachments);
            }
            Err(error) => {
                // Keep the frontend draft synchronized even after a failed batch.
                let current = (|| {
                    let state = app.state::<Mutex<Storage>>();
                    let storage = state.lock().ok()?;
                    storage.list_composer_attachments(&chat_id).ok()
                })();

                if let Some(attachments) = current {
                    let _ = app.emit("attachments-imported", attachments);
                }

                let _ = app.emit("attachments-import-error", error);
            }
        }
    });
}

#[tauri::command]
pub fn import_attachments(app: AppHandle, chat_id: String) -> Result<(), String> {
    let callback_app = app.clone();

    app.dialog().file().pick_files(move |selected_files| {
        if let Some(selected_files) = selected_files {
            let paths = selected_files
                .into_iter()
                .map(|file| file.into_path())
                .collect::<Result<Vec<_>, _>>();

            match paths {
                Ok(paths) => process_selected_paths(callback_app, paths, chat_id),
                Err(error) => {
                    let _ = callback_app.emit(
                        "attachments-import-error",
                        format!("Cannot resolve selected files: {error}"),
                    );
                }
            }
        }
    });

    Ok(())
}

#[tauri::command]
pub fn import_files(app: AppHandle, chat_id: String, paths: Vec<String>) -> Result<(), String> {
    if paths.is_empty() {
        return Ok(());
    }

    process_selected_paths(app, paths.into_iter().map(PathBuf::from).collect(), chat_id);

    Ok(())
}
