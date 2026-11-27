use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use serde_json::Value;
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

fn canonical_real_directory(path: &Path, label: &str) -> Result<PathBuf, String> {
    let metadata =
        fs::symlink_metadata(path).map_err(|error| format!("Cannot inspect {label}: {error}"))?;

    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!("{label} is not a regular directory"));
    }

    path.canonicalize()
        .map_err(|error| format!("Cannot resolve {label}: {error}"))
}

fn ensure_child_directory(
    parent: &Path,
    name: &str,
    create_if_missing: bool,
) -> Result<PathBuf, String> {
    let mut components = Path::new(name).components();

    if !matches!(components.next(), Some(Component::Normal(_))) || components.next().is_some() {
        return Err("Invalid storage directory component".to_string());
    }

    let parent = canonical_real_directory(parent, "storage parent")?;
    let child = parent.join(name);

    match fs::symlink_metadata(&child) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && create_if_missing => {
            match fs::create_dir(&child) {
                Ok(()) => {}
                Err(create_error) if create_error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(create_error) => {
                    return Err(format!(
                        "Cannot create storage directory {}: {create_error}",
                        child.display()
                    ));
                }
            }
        }
        Err(error) => {
            return Err(format!(
                "Cannot inspect storage directory {}: {error}",
                child.display()
            ));
        }
    }

    let metadata = fs::symlink_metadata(&child)
        .map_err(|error| format!("Cannot inspect storage directory: {error}"))?;

    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!(
            "Storage directory is not a regular directory: {}",
            child.display()
        ));
    }

    let canonical = child
        .canonicalize()
        .map_err(|error| format!("Cannot resolve storage directory: {error}"))?;

    if canonical.parent() != Some(parent.as_path()) {
        return Err(format!(
            "Storage directory escapes its expected parent: {}",
            child.display()
        ));
    }

    Ok(canonical)
}

pub(super) fn ensure_storage_directories(root: &Path) -> Result<(), String> {
    let parent = root
        .parent()
        .ok_or_else(|| "Storage root has no parent directory".to_string())?;

    fs::create_dir_all(parent)
        .map_err(|error| format!("Cannot create application data directory: {error}"))?;

    let root_name = root
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "Invalid storage root directory name".to_string())?;

    let canonical_root = ensure_child_directory(parent, root_name, true)?;

    for name in ["blobs", "temp", "extracted", "indexes"] {
        ensure_child_directory(&canonical_root, name, true)?;
    }

    Ok(())
}

fn parse_blob_relative_path(relative: &str) -> Result<(String, String), String> {
    if relative.contains('\\') {
        return Err("Blob path contains an invalid separator".to_string());
    }

    let parts: Vec<&str> = relative.split('/').collect();

    if parts.len() != 3 || parts[0] != "blobs" {
        return Err("Invalid internal blob path".to_string());
    }

    let hash = parts[2]
        .strip_suffix(".blob")
        .ok_or_else(|| "Invalid blob filename".to_string())?;

    if hash.len() != 64
        || !hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("Blob filename does not contain a lowercase SHA-256".to_string());
    }

    if parts[1].len() != 2 || parts[1] != &hash[..2] {
        return Err("Blob directory does not match its SHA-256".to_string());
    }

    Ok((parts[1].to_string(), hash.to_string()))
}

fn ensure_blob_target_path(
    root: &Path,
    relative: &str,
    create_prefix: bool,
) -> Result<PathBuf, String> {
    let (prefix, hash) = parse_blob_relative_path(relative)?;
    let storage_root = canonical_real_directory(root, "storage root")?;
    let blobs_root = ensure_child_directory(&storage_root, "blobs", false)?;
    let prefix_root = ensure_child_directory(&blobs_root, &prefix, create_prefix)?;
    let candidate = prefix_root.join(format!("{hash}.blob"));

    match fs::symlink_metadata(&candidate) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(format!(
                "Blob target is a symbolic link: {}",
                candidate.display()
            ));
        }
        Ok(metadata) if !metadata.is_file() => {
            return Err(format!(
                "Blob target is not a regular file: {}",
                candidate.display()
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(format!(
                "Cannot inspect blob target {}: {error}",
                candidate.display()
            ));
        }
    }

    Ok(candidate)
}

impl Storage {
    pub fn resolve_blob_path(&self, relative: &str) -> Result<PathBuf, String> {
        let candidate = ensure_blob_target_path(&self.root_dir, relative, false)?;
        let canonical = candidate
            .canonicalize()
            .map_err(|error| format!("Cannot resolve blob file: {error}"))?;
        let canonical_parent = candidate
            .parent()
            .ok_or_else(|| "Blob path has no parent directory".to_string())?
            .canonicalize()
            .map_err(|error| format!("Cannot resolve blob directory: {error}"))?;

        if canonical.parent() != Some(canonical_parent.as_path()) {
            return Err("Blob path escapes its validated directory".to_string());
        }

        Ok(canonical)
    }

    pub fn store_blob<R: Read>(&mut self, reader: R) -> Result<BlobRecord, String> {
        self.store_blob_with_limit(reader, MAX_FILE_SIZE_BYTES)
    }

    pub(super) fn store_blob_with_limit<R: Read>(
        &mut self,
        reader: R,
        max_size: u64,
    ) -> Result<BlobRecord, String> {
        self.store_blob_with_parent(reader, max_size, None)
    }

    pub(super) fn store_blob_with_parent<R: Read>(
        &mut self,
        mut reader: R,
        max_size: u64,
        parent_operation_id: Option<&str>,
    ) -> Result<BlobRecord, String> {
        let operation_id = Uuid::new_v4().to_string();
        let timestamp = now_timestamp()?;

        let storage_root = canonical_real_directory(&self.root_dir, "storage root")?;
        ensure_child_directory(&storage_root, "blobs", false)?;
        let temp_root = ensure_child_directory(&storage_root, "temp", false)?;

        let temp_relative = format!("temp/{operation_id}.part");
        let temp_path = temp_root.join(format!("{operation_id}.part"));

        self.connection
            .execute(
                "INSERT INTO storage_operations (
                    id, operation_type, state, metadata_json, created_at, updated_at
                 ) VALUES (?1, 'store_blob', 'PENDING', ?2, ?3, ?3)",
                params![
                    operation_id,
                    parent_operation_id.map(|parent| {
                        serde_json::json!({
                            "parentOperationId": parent
                        })
                        .to_string()
                    }),
                    timestamp
                ],
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
            let existing_path = ensure_blob_target_path(&storage_root, &existing_relative, true)?;

            let existing_is_valid = match fs::symlink_metadata(&existing_path) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    return Err("Existing blob target is a symbolic link".to_string());
                }
                Ok(metadata) if !metadata.is_file() => {
                    return Err("Existing blob target is not a regular file".to_string());
                }
                Ok(_) => hash_file(&existing_path)
                    .map(|hash| hash == sha256)
                    .unwrap_or(false),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
                Err(error) => {
                    return Err(format!("Cannot inspect existing blob: {error}"));
                }
            };

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

            self.connection
                .execute(
                    "UPDATE storage_operations
                     SET blob_sha256 = ?1, state = 'COMMITTING',
                         target_path = ?2, updated_at = ?3
                     WHERE id = ?4",
                    params![sha256, existing_relative, now_timestamp()?, operation_id],
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
            let target_path = ensure_blob_target_path(&storage_root, &relative, true)?;

            self.connection
                .execute(
                    "UPDATE storage_operations
                     SET blob_sha256 = ?1, state = 'VERIFYING',
                         target_path = ?2, updated_at = ?3
                     WHERE id = ?4",
                    params![sha256, relative, now_timestamp()?, operation_id],
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
                   )
                   AND NOT EXISTS (
                       SELECT 1 FROM extracted_documents WHERE blob_sha256 = ?1
                   )",
                params![sha256],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| format!("Cannot resolve unused blob: {error}"))?;

        let Some(relative_path) = relative_path else {
            // Do not mistake a registered-but-referenced blob for a missing
            // blobs row. Shared assets must keep their physical CAS file.
            let blob_is_registered: bool = transaction
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM blobs WHERE sha256 = ?1)",
                    params![sha256],
                    |row| row.get(0),
                )
                .map_err(|error| {
                    format!("Cannot inspect blob registration during cleanup: {error}")
                })?;
            transaction
                .commit()
                .map_err(|error| format!("Cannot finish blob cleanup check: {error}"))?;
            if !blob_is_registered {
                let _ = remove_unregistered_blob_file(&self.root_dir, sha256)?;
            }
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
                   )
                   AND NOT EXISTS (
                       SELECT 1 FROM extracted_documents WHERE blob_sha256 = ?1
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

pub(super) fn remove_recovered_blob_file(root: &Path, relative: &str) -> Result<(), String> {
    // Creating a missing hash-prefix directory keeps cleanup idempotent.
    let candidate = ensure_blob_target_path(root, relative, true)?;

    let metadata = match fs::symlink_metadata(&candidate) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("Cannot inspect blob during cleanup: {error}")),
    };

    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("Blob cleanup target is not a regular file".to_string());
    }

    let canonical = candidate
        .canonicalize()
        .map_err(|error| format!("Cannot resolve blob cleanup target: {error}"))?;
    let canonical_parent = candidate
        .parent()
        .ok_or_else(|| "Blob cleanup target has no parent directory".to_string())?
        .canonicalize()
        .map_err(|error| format!("Cannot resolve blob cleanup directory: {error}"))?;

    if canonical.parent() != Some(canonical_parent.as_path()) {
        return Err("Blob cleanup target escapes its validated directory".to_string());
    }

    fs::remove_file(&candidate)
        .map_err(|error| format!("Cannot remove unused physical blob: {error}"))
}

fn resolve_recovery_temp_path(root: &Path, relative: &str) -> Result<Option<PathBuf>, String> {
    if relative.contains('\\') {
        return Err("Temporary path contains an invalid separator".to_string());
    }

    let parts: Vec<&str> = relative.split('/').collect();
    if parts.len() != 2 || parts[0] != "temp" {
        return Err("Temporary path is outside the temp directory".to_string());
    }

    let filename = parts[1];
    let uuid = filename
        .strip_suffix(".part")
        .ok_or_else(|| "Temporary filename has an invalid extension".to_string())?;

    Uuid::parse_str(uuid)
        .map_err(|_| "Temporary filename does not contain a valid operation UUID".to_string())?;

    let storage_root = canonical_real_directory(root, "storage root")?;
    let temp_root = ensure_child_directory(&storage_root, "temp", false)?;
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

pub(super) fn collect_child_blob_hashes(
    connection: &Connection,
    parent_operation_id: &str,
) -> Result<Vec<String>, String> {
    let mut statement = connection
        .prepare(
            "SELECT state, blob_sha256, metadata_json
             FROM storage_operations
             WHERE operation_type = 'store_blob'",
        )
        .map_err(|error| format!("Cannot inspect child blob journals: {error}"))?;

    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })
        .map_err(|error| format!("Cannot read child blob journals: {error}"))?;

    let mut hashes = Vec::new();

    for row in rows {
        let (state, blob_sha256, metadata_json) =
            row.map_err(|error| format!("Cannot read child blob journal row: {error}"))?;

        let Some(raw_metadata) = metadata_json else {
            continue;
        };

        // Malformed metadata belonging to unrelated operations must not
        // prevent recovery of this import.
        let Ok(metadata) = serde_json::from_str::<Value>(&raw_metadata) else {
            continue;
        };

        if metadata.get("parentOperationId").and_then(Value::as_str) != Some(parent_operation_id) {
            continue;
        }

        match blob_sha256 {
            Some(hash) => {
                if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                    return Err(format!(
                        "Child store_blob journal contains an invalid SHA-256: {hash}"
                    ));
                }
                hashes.push(hash);
            }
            None if state == "COMPLETED" => {
                return Err("Completed child store_blob journal has no blob SHA-256".to_string());
            }
            None => {}
        }
    }

    Ok(hashes)
}

fn parse_replaced_asset_hashes(metadata_json: Option<&str>) -> Result<Vec<String>, String> {
    let raw_metadata =
        metadata_json.ok_or_else(|| "Document asset batch metadata is missing".to_string())?;
    let metadata = serde_json::from_str::<Value>(raw_metadata)
        .map_err(|error| format!("Cannot parse document asset batch metadata: {error}"))?;
    let object = metadata
        .as_object()
        .ok_or_else(|| "Document asset batch metadata must be a JSON object".to_string())?;
    // Older/interrupted batches may predate this optional field. Treat an
    // absent field as no previous assets, but reject a present malformed value.
    let Some(values) = object.get("replacedAssetHashes") else {
        return Ok(Vec::new());
    };
    let values = values
        .as_array()
        .ok_or_else(|| "replacedAssetHashes must be a JSON array".to_string())?;

    let mut hashes = Vec::with_capacity(values.len());
    for (index, value) in values.iter().enumerate() {
        let hash = value
            .as_str()
            .ok_or_else(|| format!("Replaced asset hash {index} is not a string"))?;
        if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(format!(
                "Replaced asset hash {index} is not a valid SHA-256"
            ));
        }
        hashes.push(hash.to_owned());
    }

    Ok(hashes)
}

fn remove_unregistered_blob_file(root: &Path, sha256: &str) -> Result<bool, String> {
    if sha256.len() != 64 || !sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("Invalid SHA-256 for unregistered blob cleanup".into());
    }
    let normalized_hash = sha256.to_ascii_lowercase();
    let prefix = &normalized_hash[..2];
    let relative = format!("blobs/{prefix}/{normalized_hash}.blob");
    let prefix_path = root.join("blobs").join(prefix);

    match fs::symlink_metadata(&prefix_path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(format!(
                "Cannot inspect unregistered blob directory: {error}"
            ));
        }
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err("Unregistered blob directory is not a regular directory".into());
        }
        Ok(_) => {}
    }

    let candidate = ensure_blob_target_path(root, &relative, false)?;
    match fs::symlink_metadata(&candidate) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(format!("Cannot inspect unregistered blob file: {error}")),
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            return Err("Unregistered blob target is not a regular file".into());
        }
        Ok(_) => {}
    }

    let actual_hash = hash_file(&candidate)?;
    if actual_hash != normalized_hash {
        return Err(
            "Unregistered blob content does not match its journaled SHA-256; file preserved".into(),
        );
    }

    remove_recovered_blob_file(root, &relative)?;
    Ok(true)
}

fn recover_unreferenced_blob(
    connection: &Connection,
    root_dir: &Path,
    sha256: &str,
) -> Result<bool, String> {
    if sha256.len() != 64 || !sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("Invalid SHA-256 in interrupted batch journal".to_string());
    }
    let timestamp = now_timestamp()?;
    let transaction = connection
        .unchecked_transaction()
        .map_err(|error| format!("Cannot start batch recovery cleanup: {error}"))?;
    let relative_path: Option<String> = transaction
        .query_row(
            "SELECT relative_path FROM blobs WHERE sha256 = ?1
           AND NOT EXISTS (SELECT 1 FROM attachment_versions WHERE blob_sha256 = ?1)
           AND NOT EXISTS (SELECT 1 FROM document_assets WHERE blob_sha256 = ?1)
           AND NOT EXISTS (SELECT 1 FROM archive_entries WHERE blob_sha256 = ?1)
           AND NOT EXISTS (SELECT 1 FROM extracted_documents WHERE blob_sha256 = ?1)",
            params![sha256],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("Cannot inspect interrupted batch blob: {error}"))?;
    let Some(relative_path) = relative_path else {
        // A missing path from the reference-filtered query is ambiguous:
        // the blob may still be registered but referenced by an attachment,
        // document asset, or archive entry. Only physical orphans whose
        // blobs row is absent may be removed here.
        let blob_is_registered: bool = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM blobs WHERE sha256 = ?1)",
                params![sha256],
                |row| row.get(0),
            )
            .map_err(|error| format!("Cannot inspect batch blob registration: {error}"))?;
        transaction
            .commit()
            .map_err(|error| format!("Cannot finish batch recovery check: {error}"))?;
        if blob_is_registered {
            return Ok(false);
        }
        return remove_unregistered_blob_file(root_dir, sha256);
    };
    let operation_id = Uuid::new_v4().to_string();
    transaction
        .execute(
            "INSERT INTO storage_operations (
            id, operation_type, blob_sha256, target_path, state, created_at, updated_at
         ) VALUES (?1, 'delete_blob', ?2, ?3, 'PENDING', ?4, ?4)",
            params![operation_id, sha256, relative_path, timestamp],
        )
        .map_err(|error| format!("Cannot journal recovered blob cleanup: {error}"))?;
    let deleted = transaction
        .execute(
            "DELETE FROM blobs WHERE sha256 = ?1
           AND NOT EXISTS (SELECT 1 FROM attachment_versions WHERE blob_sha256 = ?1)
           AND NOT EXISTS (SELECT 1 FROM document_assets WHERE blob_sha256 = ?1)
           AND NOT EXISTS (SELECT 1 FROM archive_entries WHERE blob_sha256 = ?1)
           AND NOT EXISTS (SELECT 1 FROM extracted_documents WHERE blob_sha256 = ?1)",
            params![sha256],
        )
        .map_err(|error| format!("Cannot remove recovered orphan blob metadata: {error}"))?;
    transaction
        .commit()
        .map_err(|error| format!("Cannot commit recovered batch blob cleanup: {error}"))?;
    if deleted == 0 {
        return Ok(false);
    }
    remove_recovered_blob_file(root_dir, &relative_path)?;
    connection.execute(
        "UPDATE storage_operations SET state = 'COMPLETED', error_message = NULL, updated_at = ?1
         WHERE id = ?2",
        params![now_timestamp()?, operation_id]
    ).map_err(|error| format!("Cannot complete recovered batch blob cleanup: {error}"))?;
    Ok(true)
}

pub(super) fn recover_interrupted_operations(
    connection: &Connection,
    root_dir: &Path,
) -> Result<(), String> {
    let operations = {
        let mut statement = connection
            .prepare(
                "SELECT id, operation_type, source_path, target_path, blob_sha256, metadata_json
                 FROM storage_operations
                 WHERE state IN ('PENDING', 'PROCESSING', 'VERIFYING', 'COMMITTING')
                    OR (operation_type IN ('import_batch', 'document_assets_batch') AND state = 'FAILED')
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
                    row.get::<_, Option<String>>(5)?,
                ))
            })
            .map_err(|error| format!("Cannot query interrupted operations: {error}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("Cannot read interrupted operations: {error}"))?
    };

    for (operation_id, operation_type, source_path, target_path, blob_sha256, metadata_json) in
        operations
    {
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
                               )
                               AND NOT EXISTS (
                                   SELECT 1 FROM extracted_documents WHERE blob_sha256 = ?1
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

                if let Some(relative) = target_path.as_deref() {
                    let registered: bool = connection
                        .query_row(
                            "SELECT EXISTS(SELECT 1 FROM blobs WHERE relative_path = ?1)",
                            params![relative],
                            |row| row.get(0),
                        )
                        .map_err(|error| format!("Cannot check recovered blob target: {error}"))?;

                    if !registered {
                        let candidate = ensure_blob_target_path(root_dir, relative, true)?;
                        let filename = candidate
                            .file_name()
                            .and_then(|name| name.to_str())
                            .ok_or_else(|| "Recovered blob filename is invalid".to_string())?;
                        let expected_hash = filename
                            .strip_suffix(".blob")
                            .filter(|hash| {
                                hash.len() == 64
                                    && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
                            })
                            .ok_or_else(|| {
                                "Recovered blob filename does not contain a SHA-256 hash"
                                    .to_string()
                            })?;

                        if candidate.exists() {
                            if hash_file(&candidate)? != expected_hash {
                                return Err("Uncommitted blob target failed SHA-256 verification; file was preserved".to_string());
                            }
                            remove_recovered_blob_file(root_dir, relative)?;
                            recovery_message
                                .push_str(" Uncommitted target blob was verified and removed.");
                        } else {
                            recovery_message
                                .push_str(" Uncommitted target blob was already absent.");
                        }
                    } else {
                        recovery_message.push_str(" Registered target blob was preserved.");
                    }
                } else {
                    recovery_message.push_str(" No target path was recorded.");
                }
            }
            "import_batch" => {
                let hashes_result = (|| -> Result<Vec<String>, String> {
                    let raw_metadata = metadata_json
                        .as_deref()
                        .ok_or_else(|| "Import journal metadata is missing".to_string())?;

                    let value = serde_json::from_str::<Value>(raw_metadata).map_err(|error| {
                        format!("Cannot parse import journal metadata: {error}")
                    })?;

                    let files = value
                        .get("files")
                        .and_then(Value::as_array)
                        .ok_or_else(|| {
                            "Import journal files is missing or is not an array".to_string()
                        })?;

                    let mut hashes = Vec::new();

                    for (index, file) in files.iter().enumerate() {
                        let sha256 =
                            file.get("blobSha256")
                                .and_then(Value::as_str)
                                .ok_or_else(|| {
                                    format!(
                                        "Import journal file entry {index} has no valid blobSha256"
                                    )
                                })?;

                        if sha256.len() != 64
                            || !sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
                        {
                            return Err(format!(
                                "Import journal file entry {index} contains an invalid SHA-256"
                            ));
                        }

                        hashes.push(sha256.to_owned());
                    }

                    // Also recover blobs whose child journal completed before
                    // the parent import journal could record the SHA.
                    hashes.extend(collect_child_blob_hashes(connection, &operation_id)?);

                    hashes.sort();
                    hashes.dedup();
                    Ok(hashes)
                })();

                match hashes_result {
                    Err(error) => {
                        recovery_message.push_str(&format!(
                            " Import recovery deferred because journal data is invalid: {error}"
                        ));
                    }
                    Ok(hashes) => {
                        let mut cleaned = 0usize;
                        let mut preserved = 0usize;
                        let mut cleanup_failed = false;

                        for sha256 in hashes {
                            match recover_unreferenced_blob(connection, root_dir, &sha256) {
                                Ok(true) => cleaned += 1,
                                Ok(false) => preserved += 1,
                                Err(error) => {
                                    cleanup_failed = true;
                                    recovery_message.push_str(&format!(
                                        " Cleanup deferred for blob {sha256}: {error}"
                                    ));
                                }
                            }
                        }

                        recovery_message.push_str(&format!(
                            " Interrupted import recovery removed {cleaned} unreferenced blobs and preserved {preserved} referenced or already unavailable blobs."
                        ));

                        recovered_state = if cleanup_failed {
                            "FAILED"
                        } else {
                            "COMPLETED"
                        };
                    }
                }
            }
            "document_assets_batch" => {
                let hashes_result = (|| -> Result<Vec<String>, String> {
                    let mut hashes = collect_child_blob_hashes(connection, &operation_id)?;
                    hashes.extend(parse_replaced_asset_hashes(metadata_json.as_deref())?);
                    hashes.sort();
                    hashes.dedup();
                    Ok(hashes)
                })();

                match hashes_result {
                    Err(error) => {
                        recovery_message.push_str(&format!(
                            " Document asset recovery deferred because child journal data is invalid: {error}"
                        ));
                    }
                    Ok(hashes) => {
                        let mut cleaned = 0usize;
                        let mut preserved = 0usize;
                        let mut cleanup_failed = false;

                        for sha256 in hashes {
                            match recover_unreferenced_blob(connection, root_dir, &sha256) {
                                Ok(true) => cleaned += 1,
                                Ok(false) => preserved += 1,
                                Err(error) => {
                                    cleanup_failed = true;
                                    recovery_message.push_str(&format!(
                                        " Cleanup deferred for asset blob {sha256}: {error}"
                                    ));
                                }
                            }
                        }

                        recovery_message.push_str(&format!(
                            " Interrupted document asset recovery removed {cleaned} unreferenced blobs and preserved {preserved} referenced or already unavailable blobs."
                        ));

                        recovered_state = if cleanup_failed {
                            "FAILED"
                        } else {
                            "COMPLETED"
                        };
                    }
                }
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
                   AND (
                       state IN ('PENDING', 'PROCESSING', 'VERIFYING', 'COMMITTING')
                       OR (operation_type IN ('import_batch', 'document_assets_batch') AND state = 'FAILED')
                   )",
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

#[cfg(test)]
mod path_safety_tests {
    use super::*;
    use std::path::PathBuf;

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            for _ in 0..8 {
                let path = std::env::temp_dir().join(format!(
                    "sonne-blob-path-safety-{}-{}",
                    std::process::id(),
                    uuid::Uuid::new_v4()
                ));

                match fs::create_dir(&path) {
                    Ok(()) => return Self(path),
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(error) => {
                        panic!("Cannot create test directory {}: {error}", path.display())
                    }
                }
            }

            panic!("Could not allocate a unique blob path-safety test directory");
        }

        fn storage_root(&self) -> PathBuf {
            self.0.join("storage")
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn prepare_storage(directory: &TestDirectory) -> PathBuf {
        let root = directory.storage_root();
        ensure_storage_directories(&root).unwrap();
        root
    }

    fn test_relative(hash: &str) -> String {
        format!("blobs/{}/{}.blob", &hash[..2], hash)
    }

    #[test]
    fn creates_expected_storage_directories() {
        let directory = TestDirectory::new();
        let root = prepare_storage(&directory);

        for name in ["blobs", "temp", "extracted", "indexes"] {
            let metadata = fs::symlink_metadata(root.join(name)).unwrap();
            assert!(metadata.is_dir());
            assert!(!metadata.file_type().is_symlink());
        }
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlinked_storage_root() {
        use std::os::unix::fs::symlink;

        let directory = TestDirectory::new();
        let root = directory.storage_root();
        let outside = directory.0.join("outside");
        fs::create_dir_all(&outside).unwrap();
        symlink(&outside, &root).unwrap();

        assert!(ensure_storage_directories(&root).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlinked_blobs_root() {
        use std::os::unix::fs::symlink;

        let directory = TestDirectory::new();
        let root = directory.storage_root();
        fs::create_dir_all(&root).unwrap();
        let outside = directory.0.join("outside");
        fs::create_dir_all(&outside).unwrap();
        symlink(&outside, root.join("blobs")).unwrap();

        assert!(ensure_storage_directories(&root).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlinked_temp_root() {
        use std::os::unix::fs::symlink;

        let directory = TestDirectory::new();
        let root = prepare_storage(&directory);
        let temp_root = root.join("temp");
        fs::remove_dir(&temp_root).unwrap();
        let outside = directory.0.join("outside-temp");
        fs::create_dir_all(&outside).unwrap();
        symlink(&outside, &temp_root).unwrap();

        assert!(ensure_storage_directories(&root).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlinked_hash_prefix_directory() {
        use std::os::unix::fs::symlink;

        let directory = TestDirectory::new();
        let root = prepare_storage(&directory);
        let hash = format!("ab{}", "0".repeat(62));
        let relative = test_relative(&hash);
        let outside = directory.0.join("outside-prefix");
        fs::create_dir_all(&outside).unwrap();
        symlink(&outside, root.join("blobs").join("ab")).unwrap();

        assert!(ensure_blob_target_path(&root, &relative, false).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlinked_blob_file() {
        use std::os::unix::fs::symlink;

        let directory = TestDirectory::new();
        let root = prepare_storage(&directory);
        let hash = format!("cd{}", "1".repeat(62));
        let prefix = root.join("blobs").join("cd");
        fs::create_dir(&prefix).unwrap();

        let outside = directory.0.join("outside.blob");
        fs::write(&outside, b"must not be followed").unwrap();
        symlink(&outside, prefix.join(format!("{hash}.blob"))).unwrap();

        assert!(ensure_blob_target_path(&root, &test_relative(&hash), false).is_err());
    }

    #[test]
    fn creates_a_valid_blob_target_inside_the_hash_prefix() {
        let directory = TestDirectory::new();
        let root = prepare_storage(&directory);
        let hash = format!("ef{}", "2".repeat(62));
        let relative = test_relative(&hash);

        let target = ensure_blob_target_path(&root, &relative, true).unwrap();

        assert_eq!(
            target.parent().unwrap(),
            root.join("blobs").join("ef").canonicalize().unwrap()
        );
        assert!(!target.exists());
    }
}

#[cfg(test)]
mod document_asset_batch_recovery_tests {
    use super::*;

    #[test]
    fn replaced_asset_hashes_must_be_an_array() {
        let error = parse_replaced_asset_hashes(Some(r#"{"replacedAssetHashes":"not-an-array"}"#))
            .unwrap_err();
        assert!(error.contains("must be a JSON array"));
    }

    #[test]
    fn replaced_asset_hashes_must_contain_valid_sha256_values() {
        let error =
            parse_replaced_asset_hashes(Some(r#"{"replacedAssetHashes":["short"]}"#)).unwrap_err();
        assert!(error.contains("valid SHA-256"));
    }

    #[test]
    fn missing_replaced_asset_hashes_is_compatible_with_legacy_batches() {
        assert_eq!(
            parse_replaced_asset_hashes(Some(r#"{"documentId":"legacy-batch"}"#)).unwrap(),
            Vec::<String>::new()
        );
    }

    #[test]
    fn child_blob_collection_includes_hash_before_blob_row_commit() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE storage_operations (
                    operation_type TEXT NOT NULL,
                    state TEXT NOT NULL,
                    blob_sha256 TEXT,
                    metadata_json TEXT
                );",
            )
            .unwrap();
        let hash = "b".repeat(64);
        let metadata = serde_json::json!({
            "parentOperationId": "asset-batch-test"
        })
        .to_string();
        connection
            .execute(
                "INSERT INTO storage_operations (operation_type,state,blob_sha256,metadata_json)
             VALUES ('store_blob','COMMITTING',?1,?2)",
                params![hash, metadata],
            )
            .unwrap();

        assert_eq!(
            collect_child_blob_hashes(&connection, "asset-batch-test").unwrap(),
            vec![hash]
        );
    }

    #[test]
    fn replaced_asset_hashes_accepts_an_empty_array() {
        assert_eq!(
            parse_replaced_asset_hashes(Some(r#"{"replacedAssetHashes":[]}"#)).unwrap(),
            Vec::<String>::new()
        );
    }

    #[test]
    fn child_blob_collection_includes_durable_child_hashes() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE storage_operations (
                    operation_type TEXT NOT NULL,
                    state TEXT NOT NULL,
                    blob_sha256 TEXT,
                    metadata_json TEXT
                );",
            )
            .unwrap();

        let hash = "a".repeat(64);
        let metadata = serde_json::json!({ "parentOperationId": "asset-batch-test" }).to_string();
        connection
            .execute(
                "INSERT INTO storage_operations (
                    operation_type, state, blob_sha256, metadata_json
                 ) VALUES ('store_blob', 'COMPLETED', ?1, ?2)",
                rusqlite::params![hash, metadata],
            )
            .unwrap();

        assert_eq!(
            collect_child_blob_hashes(&connection, "asset-batch-test").unwrap(),
            vec![hash]
        );
    }
}
