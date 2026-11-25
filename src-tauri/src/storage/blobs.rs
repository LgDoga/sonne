use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::{MAX_FILE_SIZE_BYTES, Storage};

const BUFFER_SIZE: usize = 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BlobRecord {
    pub sha256: String,
    pub relative_path: String,
    pub original_size: i64,
    pub stored_size: i64,
}

fn now_timestamp() -> Result<i64, String> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("System clock error: {error}"))?;

    i64::try_from(duration.as_millis()).map_err(|error| format!("Timestamp overflow: {error}"))
}

fn digest_to_hex(digest: &[u8]) -> String {
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn hash_file(path: &Path) -> Result<String, String> {
    let mut file =
        File::open(path).map_err(|error| format!("Cannot open existing blob: {error}"))?;

    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; BUFFER_SIZE];

    loop {
        let bytes_read = file
            .read(&mut buffer)
            .map_err(|error| format!("Cannot verify existing blob: {error}"))?;

        if bytes_read == 0 {
            break;
        }

        hasher.update(&buffer[..bytes_read]);
    }

    Ok(digest_to_hex(&hasher.finalize()))
}

fn resolve_relative_path(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let path = Path::new(relative);

    if path.is_absolute()
        || !path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
        || path.components().next() != Some(Component::Normal("blobs".as_ref()))
    {
        return Err("Invalid internal blob path".to_string());
    }

    Ok(root.join(path))
}

impl Storage {
    pub fn resolve_blob_path(&self, relative: &str) -> Result<PathBuf, String> {
        let candidate = resolve_relative_path(&self.root_dir, relative)?;

        let storage_root = self
            .root_dir
            .canonicalize()
            .map_err(|error| format!("Cannot resolve storage directory: {error}"))?;

        let blobs_root = self
            .root_dir
            .join("blobs")
            .canonicalize()
            .map_err(|error| format!("Cannot resolve blob storage directory: {error}"))?;

        if !blobs_root.starts_with(&storage_root) {
            return Err("Blob storage directory escapes storage root".to_string());
        }

        let canonical = candidate
            .canonicalize()
            .map_err(|error| format!("Cannot resolve blob file: {error}"))?;

        if !canonical.starts_with(&blobs_root) {
            return Err("Blob path escapes storage directory".to_string());
        }

        let metadata = fs::symlink_metadata(&candidate)
            .map_err(|error| format!("Cannot inspect blob file: {error}"))?;

        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err("Blob path is not a regular file".to_string());
        }

        Ok(canonical)
    }

    pub fn store_blob<R: Read>(&mut self, reader: R) -> Result<BlobRecord, String> {
        self.store_blob_with_limit(reader, MAX_FILE_SIZE_BYTES)
    }

    pub(super) fn store_blob_with_limit<R: Read>(
        &mut self,
        mut reader: R,
        max_size: u64,
    ) -> Result<BlobRecord, String> {
        let operation_id = Uuid::new_v4().to_string();
        let timestamp = now_timestamp()?;

        let temp_relative = format!("temp/{operation_id}.part");
        let temp_path = self.root_dir.join(&temp_relative);

        self.connection
            .execute(
                "INSERT INTO storage_operations (
                    id, operation_type, state, created_at, updated_at
                 ) VALUES (?1, 'store_blob', 'PENDING', ?2, ?2)",
                params![operation_id, timestamp],
            )
            .map_err(|error| format!("Cannot create storage journal entry: {error}"))?;

        self.connection
            .execute(
                "UPDATE storage_operations
                 SET state = 'PROCESSING', source_path = ?1, updated_at = ?2
                 WHERE id = ?3",
                params![temp_relative, timestamp, operation_id],
            )
            .map_err(|error| format!("Cannot update storage journal: {error}"))?;

        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
            .map_err(|error| format!("Cannot create temporary blob: {error}"))?;

        let mut hasher = Sha256::new();
        let mut buffer = vec![0u8; BUFFER_SIZE];
        let mut total_size: u64 = 0;

        let write_result = (|| -> Result<(), String> {
            loop {
                let bytes_read = reader
                    .read(&mut buffer)
                    .map_err(|error| format!("Cannot read incoming file: {error}"))?;

                if bytes_read == 0 {
                    break;
                }

                total_size = total_size
                    .checked_add(bytes_read as u64)
                    .ok_or_else(|| "File size overflow".to_string())?;

                if total_size > max_size {
                    return Err(format!(
                        "File exceeds the allowed size limit of {} MiB",
                        max_size / (1024 * 1024)
                    ));
                }

                output
                    .write_all(&buffer[..bytes_read])
                    .map_err(|error| format!("Cannot write temporary blob: {error}"))?;

                hasher.update(&buffer[..bytes_read]);
            }

            output
                .sync_all()
                .map_err(|error| format!("Cannot synchronize temporary blob: {error}"))?;

            Ok(())
        })();

        drop(output);

        if let Err(error) = write_result {
            let _ = fs::remove_file(&temp_path);
            let _ = self.connection.execute(
                "UPDATE storage_operations
                 SET state = 'FAILED', error_message = ?1, updated_at = ?2
                 WHERE id = ?3",
                params![error, now_timestamp().unwrap_or(timestamp), operation_id],
            );
            return Err(error);
        }

        let original_size = i64::try_from(total_size)
            .map_err(|error| format!("File is too large for SQLite metadata: {error}"))?;

        let sha256 = digest_to_hex(&hasher.finalize());

        let existing: Option<(String, i64)> = self
            .connection
            .query_row(
                "SELECT relative_path, original_size
                 FROM blobs WHERE sha256 = ?1",
                params![sha256],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|error| format!("Cannot check blob deduplication: {error}"))?;

        let relative_path = if let Some((existing_relative, _)) = existing {
            let existing_path = resolve_relative_path(&self.root_dir, &existing_relative)?;

            let existing_is_valid = existing_path.is_file()
                && hash_file(&existing_path)
                    .map(|hash| hash == sha256)
                    .unwrap_or(false);

            if existing_is_valid {
                fs::remove_file(&temp_path)
                    .map_err(|error| format!("Cannot remove duplicate temporary file: {error}"))?;

                self.connection
                    .execute(
                        "UPDATE blobs SET last_accessed_at = ?1 WHERE sha256 = ?2",
                        params![now_timestamp()?, sha256],
                    )
                    .map_err(|error| format!("Cannot update blob access time: {error}"))?;

                self.connection
                    .execute(
                        "UPDATE storage_operations
                         SET blob_sha256 = ?1, state = 'COMPLETED', updated_at = ?2
                         WHERE id = ?3",
                        params![sha256, now_timestamp()?, operation_id],
                    )
                    .map_err(|error| format!("Cannot complete deduplication journal: {error}"))?;

                return Ok(BlobRecord {
                    sha256,
                    relative_path: existing_relative,
                    original_size,
                    stored_size: original_size,
                });
            }

            if let Some(parent) = existing_path.parent() {
                fs::create_dir_all(parent)
                    .map_err(|error| format!("Cannot recreate blob directory: {error}"))?;
            }

            self.connection
                .execute(
                    "UPDATE storage_operations
                     SET state = 'COMMITTING', target_path = ?1, updated_at = ?2
                     WHERE id = ?3",
                    params![existing_relative, now_timestamp()?, operation_id],
                )
                .map_err(|error| format!("Cannot update repair journal: {error}"))?;

            fs::rename(&temp_path, &existing_path)
                .map_err(|error| format!("Cannot repair existing blob: {error}"))?;

            self.connection
                .execute(
                    "UPDATE blobs
                     SET original_size = ?1, stored_size = ?1,
                         storage_state = 'original',
                         compression_format = NULL,
                         last_accessed_at = ?2,
                         verified_at = ?2
                     WHERE sha256 = ?3",
                    params![original_size, now_timestamp()?, sha256],
                )
                .map_err(|error| format!("Cannot update repaired blob metadata: {error}"))?;

            existing_relative
        } else {
            let prefix = &sha256[..2];
            let relative = format!("blobs/{prefix}/{sha256}.blob");
            let target_path = resolve_relative_path(&self.root_dir, &relative)?;

            if let Some(parent) = target_path.parent() {
                fs::create_dir_all(parent)
                    .map_err(|error| format!("Cannot create blob directory: {error}"))?;
            }

            self.connection
                .execute(
                    "UPDATE storage_operations
                     SET state = 'VERIFYING', target_path = ?1, updated_at = ?2
                     WHERE id = ?3",
                    params![relative, now_timestamp()?, operation_id],
                )
                .map_err(|error| format!("Cannot update verification journal: {error}"))?;

            let orphan_is_valid = target_path.is_file()
                && hash_file(&target_path)
                    .map(|hash| hash == sha256)
                    .unwrap_or(false);

            if orphan_is_valid {
                fs::remove_file(&temp_path)
                    .map_err(|error| format!("Cannot remove duplicate orphan blob: {error}"))?;
            } else {
                self.connection
                    .execute(
                        "UPDATE storage_operations
                         SET state = 'COMMITTING', updated_at = ?1
                         WHERE id = ?2",
                        params![now_timestamp()?, operation_id],
                    )
                    .map_err(|error| format!("Cannot update commit journal: {error}"))?;

                fs::rename(&temp_path, &target_path)
                    .map_err(|error| format!("Cannot commit blob file: {error}"))?;
            }

            relative
        };

        let timestamp = now_timestamp()?;

        let transaction = self
            .connection
            .transaction()
            .map_err(|error| format!("Cannot start blob metadata transaction: {error}"))?;

        transaction
            .execute(
                "INSERT INTO blobs (
                    sha256, relative_path, original_size, stored_size,
                    storage_state, created_at, last_accessed_at, verified_at
                 ) VALUES (?1, ?2, ?3, ?3, 'original', ?4, ?4, ?4)
                 ON CONFLICT(sha256) DO UPDATE SET
                    relative_path = excluded.relative_path,
                    original_size = excluded.original_size,
                    stored_size = excluded.stored_size,
                    storage_state = 'original',
                    compression_format = NULL,
                    last_accessed_at = excluded.last_accessed_at,
                    verified_at = excluded.verified_at",
                params![sha256, relative_path, original_size, timestamp],
            )
            .map_err(|error| format!("Cannot save blob metadata: {error}"))?;

        transaction
            .execute(
                "UPDATE storage_operations
                 SET blob_sha256 = ?1, state = 'COMPLETED', updated_at = ?2
                 WHERE id = ?3",
                params![sha256, timestamp, operation_id],
            )
            .map_err(|error| format!("Cannot complete blob journal: {error}"))?;

        transaction
            .commit()
            .map_err(|error| format!("Cannot commit blob metadata: {error}"))?;

        Ok(BlobRecord {
            sha256,
            relative_path,
            original_size,
            stored_size: original_size,
        })
    }

    pub(super) fn delete_unreferenced_blob(&mut self, sha256: &str) -> Result<(), String> {
        let timestamp = now_timestamp()?;
        let transaction = self
            .connection
            .transaction()
            .map_err(|error| format!("Cannot start blob cleanup transaction: {error}"))?;

        let relative_path: Option<String> = transaction
            .query_row(
                "SELECT relative_path FROM blobs
                 WHERE sha256 = ?1
                   AND NOT EXISTS (
                       SELECT 1 FROM attachment_versions WHERE blob_sha256 = ?1
                   )
                   AND NOT EXISTS (
                       SELECT 1 FROM document_assets WHERE blob_sha256 = ?1
                   )
                   AND NOT EXISTS (
                       SELECT 1 FROM archive_entries WHERE blob_sha256 = ?1
                   )",
                params![sha256],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| format!("Cannot resolve unused blob: {error}"))?;

        let Some(relative_path) = relative_path else {
            transaction
                .commit()
                .map_err(|error| format!("Cannot finish blob cleanup check: {error}"))?;
            return Ok(());
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
            .map_err(|error| format!("Cannot journal blob deletion: {error}"))?;

        let deleted = transaction
            .execute(
                "DELETE FROM blobs
                 WHERE sha256 = ?1
                   AND NOT EXISTS (
                       SELECT 1 FROM attachment_versions WHERE blob_sha256 = ?1
                   )
                   AND NOT EXISTS (
                       SELECT 1 FROM document_assets WHERE blob_sha256 = ?1
                   )
                   AND NOT EXISTS (
                       SELECT 1 FROM archive_entries WHERE blob_sha256 = ?1
                   )",
                params![sha256],
            )
            .map_err(|error| format!("Cannot remove unused blob metadata: {error}"))?;

        if deleted == 0 {
            transaction
                .execute(
                    "UPDATE storage_operations SET state = 'COMPLETED', updated_at = ?1
                     WHERE id = ?2",
                    params![now_timestamp()?, operation_id],
                )
                .map_err(|error| format!("Cannot finish unused blob journal: {error}"))?;
        }

        transaction
            .commit()
            .map_err(|error| format!("Cannot commit blob cleanup: {error}"))?;

        if deleted == 0 {
            return Ok(());
        }

        remove_recovered_blob_file(&self.root_dir, &relative_path)?;

        self.connection
            .execute(
                "UPDATE storage_operations
                 SET state = 'COMPLETED', error_message = NULL, updated_at = ?1
                 WHERE id = ?2",
                params![now_timestamp()?, operation_id],
            )
            .map_err(|error| format!("Cannot complete blob deletion journal: {error}"))?;

        Ok(())
    }
}

fn remove_recovered_blob_file(root: &Path, relative: &str) -> Result<(), String> {
    let candidate = resolve_relative_path(root, relative)?;

    let metadata = match fs::symlink_metadata(&candidate) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("Cannot inspect blob during cleanup: {error}")),
    };

    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("Blob cleanup target is not a regular file".to_string());
    }

    let storage_root = root
        .canonicalize()
        .map_err(|error| format!("Cannot resolve storage root during cleanup: {error}"))?;
    let blobs_root = root
        .join("blobs")
        .canonicalize()
        .map_err(|error| format!("Cannot resolve blobs directory during cleanup: {error}"))?;
    let canonical = candidate
        .canonicalize()
        .map_err(|error| format!("Cannot resolve blob cleanup target: {error}"))?;

    if !blobs_root.starts_with(&storage_root) || !canonical.starts_with(&blobs_root) {
        return Err("Blob cleanup target escapes storage directory".to_string());
    }

    fs::remove_file(canonical)
        .map_err(|error| format!("Cannot remove unused physical blob: {error}"))
}

fn resolve_recovery_temp_path(root: &Path, relative: &str) -> Result<Option<PathBuf>, String> {
    let mut components = Path::new(relative).components();

    if components.next() != Some(Component::Normal("temp".as_ref())) {
        return Err("Temporary path is outside the temp directory".to_string());
    }

    let filename = components
        .next()
        .and_then(|component| component.as_os_str().to_str())
        .ok_or_else(|| "Invalid temporary filename".to_string())?;

    if components.next().is_some() {
        return Err("Temporary path contains unexpected components".to_string());
    }

    let uuid = filename
        .strip_suffix(".part")
        .ok_or_else(|| "Temporary filename has an invalid extension".to_string())?;

    Uuid::parse_str(uuid)
        .map_err(|_| "Temporary filename does not contain a valid operation UUID".to_string())?;

    let temp_root = root.join("temp");
    let root_metadata = fs::symlink_metadata(&temp_root)
        .map_err(|error| format!("Cannot inspect temp directory: {error}"))?;

    if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
        return Err("Temp directory is not a regular directory".to_string());
    }

    let candidate = temp_root.join(filename);

    match fs::symlink_metadata(&candidate) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err("Temporary path is not a regular file".to_string());
            }

            Ok(Some(candidate))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("Cannot inspect temporary file: {error}")),
    }
}

pub(super) fn recover_interrupted_operations(
    connection: &Connection,
    root_dir: &Path,
) -> Result<(), String> {
    let operations = {
        let mut statement = connection
            .prepare(
                "SELECT id, operation_type, source_path, target_path, blob_sha256
                 FROM storage_operations
                 WHERE state IN ('PENDING', 'PROCESSING', 'VERIFYING', 'COMMITTING')
                 ORDER BY created_at ASC",
            )
            .map_err(|error| format!("Cannot prepare interrupted operation query: {error}"))?;

        statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            })
            .map_err(|error| format!("Cannot query interrupted operations: {error}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("Cannot read interrupted operations: {error}"))?
    };

    for (operation_id, operation_type, source_path, target_path, blob_sha256) in operations {
        let mut recovery_message =
            "Operation was interrupted before completion and recovered during startup.".to_string();
        let mut recovered_state = "FAILED";

        match operation_type.as_str() {
            "delete_blob" => {
                let can_delete = if let Some(sha256) = blob_sha256.as_deref() {
                    let deleted = connection
                        .execute(
                            "DELETE FROM blobs
                             WHERE sha256 = ?1
                               AND NOT EXISTS (
                                   SELECT 1 FROM attachment_versions WHERE blob_sha256 = ?1
                               )
                               AND NOT EXISTS (
                                   SELECT 1 FROM document_assets WHERE blob_sha256 = ?1
                               )
                               AND NOT EXISTS (
                                   SELECT 1 FROM archive_entries WHERE blob_sha256 = ?1
                               )",
                            params![sha256],
                        )
                        .map_err(|error| {
                            format!("Cannot recover interrupted blob metadata deletion: {error}")
                        })?;
                    deleted > 0
                        || !connection
                            .query_row(
                                "SELECT EXISTS(SELECT 1 FROM blobs WHERE sha256 = ?1)",
                                params![sha256],
                                |row| row.get::<_, bool>(0),
                            )
                            .map_err(|error| {
                                format!("Cannot verify recovered blob metadata: {error}")
                            })?
                } else {
                    false
                };

                if can_delete {
                    if let Some(relative) = target_path.as_deref() {
                        remove_recovered_blob_file(root_dir, relative)?;
                        recovery_message
                            .push_str(" Unreferenced physical blob cleanup was completed.");
                        recovered_state = "COMPLETED";
                    } else {
                        recovery_message.push_str(
                            " No safe target path was recorded; filesystem data was preserved.",
                        );
                    }
                } else {
                    recovery_message.push_str(
                        " Blob is still referenced or its identifier is unavailable; the physical file was preserved.",
                    );
                }
            }
            "store_blob" => {
                if let Some(relative) = source_path {
                    match resolve_recovery_temp_path(root_dir, &relative) {
                        Ok(Some(path)) => match fs::remove_file(&path) {
                            Ok(()) => {
                                recovery_message.push_str(" Incomplete temporary file was removed.")
                            }
                            Err(error) => recovery_message.push_str(&format!(
                                " Temporary file was preserved because cleanup failed: {error}"
                            )),
                        },
                        Ok(None) => {
                            recovery_message.push_str(" Temporary file was already absent.")
                        }
                        Err(error) => recovery_message.push_str(&format!(
                            " Temporary file was preserved because its path was unsafe: {error}"
                        )),
                    }
                } else {
                    recovery_message.push_str(" No temporary source path was recorded.");
                }

                recovery_message
                    .push_str(" Any committed target blob was preserved for later reconciliation.");
            }
            "import_batch" => {
                recovery_message.push_str(
                    " The attachment transaction was not completed; stored blobs were preserved.",
                );
            }
            _ => {
                recovery_message
                    .push_str(" Unknown operation type; filesystem data was left untouched.");
            }
        }

        connection
            .execute(
                "UPDATE storage_operations
                 SET state = ?1, error_message = ?2, updated_at = ?3
                 WHERE id = ?4
                   AND state IN ('PENDING', 'PROCESSING', 'VERIFYING', 'COMMITTING')",
                params![
                    recovered_state,
                    recovery_message,
                    now_timestamp()?,
                    operation_id
                ],
            )
            .map_err(|error| format!("Cannot mark interrupted operation as recovered: {error}"))?;
    }

    Ok(())
}
