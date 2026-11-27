use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Component, Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use flate2::read::GzDecoder;
use rusqlite::{OptionalExtension, params};
use tar::{Archive as TarArchive, EntryType};
use uuid::Uuid;
use zip::ZipArchive;

use super::Storage;

pub(super) const MAX_ARCHIVE_ENTRIES: usize = 10_000;
pub(super) const MAX_ARCHIVE_TOTAL_SIZE: u64 = 1024 * 1024 * 1024;
pub(super) const MAX_ARCHIVE_ENTRY_SIZE: u64 = 512 * 1024 * 1024;
pub(super) const MAX_NESTED_ARCHIVE_DEPTH: u8 = 2;

#[derive(Debug)]
pub(super) struct ArchiveBudget {
    entries: usize,
    extracted_bytes: u64,
}

impl ArchiveBudget {
    pub(super) fn new() -> Self {
        Self {
            entries: 0,
            extracted_bytes: 0,
        }
    }

    fn register_entry(&mut self) -> Result<(), String> {
        self.entries = self
            .entries
            .checked_add(1)
            .ok_or_else(|| "Archive entry counter overflow".to_string())?;
        if self.entries > MAX_ARCHIVE_ENTRIES {
            return Err(format!("Archive exceeds {MAX_ARCHIVE_ENTRIES} entries"));
        }
        Ok(())
    }

    fn remaining(&self) -> u64 {
        MAX_ARCHIVE_TOTAL_SIZE.saturating_sub(self.extracted_bytes)
    }

    fn entry_limit(&self) -> u64 {
        self.remaining().min(MAX_ARCHIVE_ENTRY_SIZE)
    }

    fn register_bytes(&mut self, bytes: u64) -> Result<(), String> {
        let next = self
            .extracted_bytes
            .checked_add(bytes)
            .ok_or_else(|| "Archive extraction size overflow".to_string())?;
        if next > MAX_ARCHIVE_TOTAL_SIZE {
            return Err("Archive exceeds the 1 GiB extraction limit".to_string());
        }
        self.extracted_bytes = next;
        Ok(())
    }
}

pub(super) fn validate_archive_entry_path(raw: &str) -> Result<String, String> {
    if raw.is_empty()
        || raw.starts_with('/')
        || raw.starts_with("//")
        || raw.contains('\\')
        || raw.contains('\0')
    {
        return Err("Unsafe archive entry path".into());
    }

    let path = Path::new(raw);
    if path.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return Err("Archive entry attempts path traversal".into());
    }

    let mut parts = Vec::new();
    for part in raw.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." || part.contains(':') || part.chars().any(char::is_control) {
            return Err("Unsafe archive path component".into());
        }
        parts.push(part);
    }

    if parts.is_empty() {
        return Err("Empty archive entry path".into());
    }
    Ok(parts.join("/"))
}

fn timestamp() -> Result<i64, String> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| format!("System clock error: {e}"))?;
    i64::try_from(duration.as_millis()).map_err(|e| format!("Timestamp overflow: {e}"))
}

fn is_archive_name(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.ends_with(".zip")
        || name.ends_with(".tar")
        || name.ends_with(".tar.gz")
        || name.ends_with(".tgz")
        || name.ends_with(".gz")
}

pub(super) fn is_supported_archive_extension(extension: &str, file_name: &str) -> bool {
    matches!(extension, "zip" | "tar" | "gz" | "tgz")
        || file_name.to_ascii_lowercase().ends_with(".tar.gz")
}

fn safe_blob_path(storage: &Storage, sha256: &str) -> Result<PathBuf, String> {
    let relative: String = storage
        .connection
        .query_row(
            "SELECT relative_path FROM blobs WHERE sha256 = ?1",
            params![sha256],
            |row| row.get(0),
        )
        .map_err(|e| format!("Cannot locate archive blob: {e}"))?;

    let relative_path = Path::new(&relative);
    if relative_path
        .components()
        .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err("Invalid blob storage path".into());
    }

    let blobs_root = storage
        .root_dir
        .join("blobs")
        .canonicalize()
        .map_err(|e| format!("Cannot resolve blob directory: {e}"))?;
    let candidate = storage.root_dir.join(relative_path);
    let metadata = fs::symlink_metadata(&candidate)
        .map_err(|e| format!("Cannot inspect archive blob: {e}"))?;

    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("Archive blob is not a regular file".into());
    }

    let resolved = candidate
        .canonicalize()
        .map_err(|e| format!("Cannot resolve archive blob: {e}"))?;
    if !resolved.starts_with(&blobs_root) {
        return Err("Archive blob is outside the blob storage directory".into());
    }
    Ok(resolved)
}

fn create_document(
    storage: &Storage,
    sha256: &str,
    format: &str,
    parent_id: Option<&str>,
) -> Result<String, String> {
    let now = timestamp()?;
    let id = Uuid::new_v4().to_string();
    let version = format!("1:{}", parent_id.unwrap_or("root"));

    storage
        .connection
        .execute(
            "INSERT OR IGNORE INTO extracted_documents (
            id, blob_sha256, extractor, extractor_version, status,
            created_at, updated_at, detected_format, source_size
        )
        SELECT ?1, sha256, 'archive', ?2, 'processing',
               ?3, ?3, ?4, original_size
        FROM blobs WHERE sha256 = ?5",
            params![id, version, now, format, sha256],
        )
        .map_err(|e| format!("Cannot create archive document: {e}"))?;

    let document_id: String = storage
        .connection
        .query_row(
            "SELECT id FROM extracted_documents
         WHERE blob_sha256 = ?1 AND extractor = 'archive' AND extractor_version = ?2",
            params![sha256, version],
            |row| row.get(0),
        )
        .map_err(|e| format!("Cannot retrieve archive document: {e}"))?;

    storage
        .connection
        .execute(
            "UPDATE extracted_documents
         SET status = 'processing', detected_format = ?1,
             error_code = NULL, error_message = NULL,
             updated_at = ?2, processed_at = NULL
         WHERE id = ?3",
            params![format, now, document_id],
        )
        .map_err(|e| format!("Cannot prepare archive document: {e}"))?;

    storage
        .connection
        .execute(
            "DELETE FROM archive_entries WHERE document_id = ?1",
            params![document_id],
        )
        .map_err(|e| format!("Cannot reset archive entries: {e}"))?;

    Ok(document_id)
}

fn set_document_status(
    storage: &Storage,
    id: &str,
    status: &str,
    error: Option<&str>,
) -> Result<(), String> {
    let now = timestamp()?;
    storage
        .connection
        .execute(
            "UPDATE extracted_documents
         SET status = ?1,
             error_code = CASE WHEN ?2 IS NULL THEN NULL ELSE 'ARCHIVE_PROCESSING' END,
             error_message = ?2, updated_at = ?3, processed_at = ?3
         WHERE id = ?4",
            params![status, error, now, id],
        )
        .map_err(|e| format!("Cannot update archive status: {e}"))?;
    Ok(())
}

fn insert_entry(
    storage: &Storage,
    document_id: &str,
    parent_id: Option<&str>,
    path: &str,
    kind: &str,
    depth: u8,
    compression: Option<&str>,
    compressed_size: Option<u64>,
    size: Option<u64>,
    blob_sha256: Option<&str>,
    status: &str,
    error: Option<&str>,
) -> Result<String, String> {
    let id = Uuid::new_v4().to_string();
    let now = timestamp()?;
    let name = path
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(path);
    let compressed = compressed_size
        .map(i64::try_from)
        .transpose()
        .map_err(|e| format!("Compressed size overflow: {e}"))?;
    let uncompressed = size
        .map(i64::try_from)
        .transpose()
        .map_err(|e| format!("Entry size overflow: {e}"))?;

    storage
        .connection
        .execute(
            "INSERT INTO archive_entries (
            id, document_id, parent_id, entry_path, entry_name,
            entry_type, nested_depth, compression_method,
            compressed_size, uncompressed_size, blob_sha256,
            extraction_status, error_message, ordinal, created_at
        ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
            (SELECT COUNT(*) FROM archive_entries WHERE document_id = ?2), ?14
        )",
            params![
                id,
                document_id,
                parent_id,
                path,
                name,
                kind,
                i64::from(depth),
                compression,
                compressed,
                uncompressed,
                blob_sha256,
                status,
                error,
                now
            ],
        )
        .map_err(|e| format!("Cannot save archive entry: {e}"))?;

    Ok(id)
}

fn save_unsupported(
    storage: &Storage,
    budget: &mut ArchiveBudget,
    document_id: &str,
    parent_id: Option<&str>,
    raw_path: &str,
    depth: u8,
    compression: Option<&str>,
    compressed_size: Option<u64>,
    size: Option<u64>,
    reason: &str,
) -> Result<(), String> {
    budget.register_entry()?;
    let path = validate_archive_entry_path(raw_path)
        .unwrap_or_else(|_| format!("unsupported-entry-{}", Uuid::new_v4()));

    insert_entry(
        storage,
        document_id,
        parent_id,
        &path,
        "unsupported",
        depth,
        compression,
        compressed_size,
        size,
        None,
        "unsupported",
        Some(reason),
    )?;
    Ok(())
}

fn extract_to_blob<R: Read>(
    storage: &mut Storage,
    reader: &mut R,
    budget: &mut ArchiveBudget,
) -> Result<(String, u64), String> {
    let limit = budget.entry_limit();
    if limit == 0 {
        return Err("Archive extraction budget exhausted".into());
    }

    let temp_dir = storage.root_dir.join("temp");
    let temp_path = temp_dir.join(format!("{}.part", Uuid::new_v4()));

    let result = (|| -> Result<u64, String> {
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
            .map_err(|e| format!("Cannot create extraction temp file: {e}"))?;

        let mut bounded = reader.take(limit.saturating_add(1));
        let copied = io::copy(&mut bounded, &mut output)
            .map_err(|e| format!("Cannot extract archive entry: {e}"))?;

        if copied > limit {
            return Err("Archive entry exceeds its extraction limit".into());
        }

        output
            .flush()
            .map_err(|e| format!("Cannot flush extracted entry: {e}"))?;
        output
            .sync_all()
            .map_err(|e| format!("Cannot sync extracted entry: {e}"))?;
        Ok(copied)
    })();

    let size = match result {
        Ok(size) => size,
        Err(error) => {
            let _ = fs::remove_file(&temp_path);
            return Err(error);
        }
    };

    if let Err(error) = budget.register_bytes(size) {
        let _ = fs::remove_file(&temp_path);
        return Err(error);
    }

    let stored = File::open(&temp_path)
        .map_err(|e| format!("Cannot reopen extracted entry: {e}"))
        .and_then(|file| storage.store_blob_with_limit(file, limit));
    let _ = fs::remove_file(&temp_path);

    Ok((stored?.sha256, size))
}

fn update_nested(
    storage: &Storage,
    entry_id: &str,
    nested_id: Option<&str>,
    status: &str,
    error: Option<&str>,
) -> Result<(), String> {
    storage
        .connection
        .execute(
            "UPDATE archive_entries
         SET nested_document_id = ?1, extraction_status = ?2, error_message = ?3
         WHERE id = ?4",
            params![nested_id, status, error, entry_id],
        )
        .map_err(|e| format!("Cannot update nested archive: {e}"))?;
    Ok(())
}

fn process_nested(
    storage: &mut Storage,
    entry_id: &str,
    blob_sha256: &str,
    name: &str,
    depth: u8,
    budget: &mut ArchiveBudget,
) -> Result<(), String> {
    if !is_archive_name(name) {
        return Ok(());
    }

    if depth >= MAX_NESTED_ARCHIVE_DEPTH {
        return update_nested(
            storage,
            entry_id,
            None,
            "unsupported",
            Some("Maximum nested archive depth reached"),
        );
    }

    let lower = name.to_ascii_lowercase();
    let extension = if lower.ends_with(".tar.gz") {
        "tar.gz"
    } else if lower.ends_with(".tgz") {
        "tgz"
    } else {
        Path::new(name)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
    };

    match process_document(
        storage,
        blob_sha256,
        name,
        extension,
        depth + 1,
        Some(entry_id),
        budget,
    ) {
        Ok(id) => update_nested(storage, entry_id, Some(&id), "completed", None),
        Err(error) => update_nested(storage, entry_id, None, "failed", Some(&error)),
    }
}

fn save_file<R: Read>(
    storage: &mut Storage,
    reader: &mut R,
    budget: &mut ArchiveBudget,
    document_id: &str,
    parent_id: Option<&str>,
    raw_path: &str,
    depth: u8,
    compression: Option<&str>,
    compressed_size: Option<u64>,
    declared_size: Option<u64>,
) -> Result<(), String> {
    budget.register_entry()?;

    let path = match validate_archive_entry_path(raw_path) {
        Ok(path) => path,
        Err(error) => {
            let fallback = format!("unsupported-entry-{}", Uuid::new_v4());
            insert_entry(
                storage,
                document_id,
                parent_id,
                &fallback,
                "unsupported",
                depth,
                compression,
                compressed_size,
                declared_size,
                None,
                "unsupported",
                Some(&error),
            )?;
            return Ok(());
        }
    };

    if declared_size.is_some_and(|size| size > budget.entry_limit()) {
        insert_entry(
            storage,
            document_id,
            parent_id,
            &path,
            "unsupported",
            depth,
            compression,
            compressed_size,
            declared_size,
            None,
            "failed",
            Some("Entry exceeds extraction size limit"),
        )?;
        return Ok(());
    }

    let (sha256, actual_size) = match extract_to_blob(storage, reader, budget) {
        Ok(result) => result,
        Err(error) => {
            insert_entry(
                storage,
                document_id,
                parent_id,
                &path,
                "unsupported",
                depth,
                compression,
                compressed_size,
                declared_size,
                None,
                "failed",
                Some(&error),
            )?;
            return Ok(());
        }
    };

    let nested = is_archive_name(&path);
    let kind = if nested { "archive" } else { "file" };

    let entry_id = insert_entry(
        storage,
        document_id,
        parent_id,
        &path,
        kind,
        depth,
        compression,
        compressed_size,
        Some(actual_size),
        Some(&sha256),
        "completed",
        None,
    )?;

    if nested {
        process_nested(storage, &entry_id, &sha256, &path, depth, budget)?;
    }
    Ok(())
}

fn process_zip(
    storage: &mut Storage,
    document_id: &str,
    source: &Path,
    depth: u8,
    parent_id: Option<&str>,
    budget: &mut ArchiveBudget,
) -> Result<(), String> {
    let file = File::open(source).map_err(|e| format!("Cannot open ZIP: {e}"))?;
    let mut archive = ZipArchive::new(file).map_err(|e| format!("Invalid ZIP: {e}"))?;

    if archive.len() > MAX_ARCHIVE_ENTRIES {
        return Err("ZIP archive contains too many entries".into());
    }

    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|e| format!("Cannot read ZIP entry: {e}"))?;

        let name = entry
            .name()
            .map_err(|e| format!("Cannot read ZIP entry name: {e}"))?
            .into_owned();
        let compressed = entry.compressed_size();
        let size = entry.size();
        let method = format!("{:?}", entry.compression());

        if entry.encrypted() {
            save_unsupported(
                storage,
                budget,
                document_id,
                parent_id,
                &name,
                depth,
                Some(&method),
                Some(compressed),
                Some(size),
                "Encrypted ZIP entry requires a password",
            )?;
            continue;
        }

        if entry.is_symlink() || entry.enclosed_name().is_none() {
            save_unsupported(
                storage,
                budget,
                document_id,
                parent_id,
                &name,
                depth,
                Some(&method),
                Some(compressed),
                Some(size),
                "Unsafe ZIP path or symbolic link",
            )?;
            continue;
        }

        if entry.is_dir() {
            budget.register_entry()?;
            if let Ok(path) = validate_archive_entry_path(&name) {
                insert_entry(
                    storage,
                    document_id,
                    parent_id,
                    &path,
                    "directory",
                    depth,
                    Some(&method),
                    Some(compressed),
                    Some(size),
                    None,
                    "completed",
                    None,
                )?;
            } else {
                save_unsupported(
                    storage,
                    budget,
                    document_id,
                    parent_id,
                    &name,
                    depth,
                    Some(&method),
                    Some(compressed),
                    Some(size),
                    "Unsafe directory path",
                )?;
            }
            continue;
        }

        save_file(
            storage,
            &mut entry,
            budget,
            document_id,
            parent_id,
            &name,
            depth,
            Some(&method),
            Some(compressed),
            Some(size),
        )?;
    }
    Ok(())
}

fn process_tar<R: Read>(
    storage: &mut Storage,
    reader: R,
    document_id: &str,
    depth: u8,
    parent_id: Option<&str>,
    budget: &mut ArchiveBudget,
) -> Result<(), String> {
    let mut archive = TarArchive::new(reader);
    let entries = archive
        .entries()
        .map_err(|e| format!("Cannot read TAR: {e}"))?;

    for item in entries {
        let mut entry = item.map_err(|e| format!("Cannot read TAR entry: {e}"))?;
        let name = entry
            .path()
            .map_err(|e| format!("Invalid TAR path: {e}"))?
            .to_string_lossy()
            .into_owned();
        let size = entry.size();
        let kind: EntryType = entry.header().entry_type();

        if kind.is_dir() {
            budget.register_entry()?;
            if let Ok(path) = validate_archive_entry_path(&name) {
                insert_entry(
                    storage,
                    document_id,
                    parent_id,
                    &path,
                    "directory",
                    depth,
                    Some("tar"),
                    None,
                    Some(size),
                    None,
                    "completed",
                    None,
                )?;
            } else {
                save_unsupported(
                    storage,
                    budget,
                    document_id,
                    parent_id,
                    &name,
                    depth,
                    Some("tar"),
                    None,
                    Some(size),
                    "Unsafe TAR directory path",
                )?;
            }
            continue;
        }

        if !kind.is_file() {
            let reason = if kind.is_symlink() {
                "TAR symbolic links are not extracted"
            } else if kind.is_hard_link() {
                "TAR hard links are not extracted"
            } else if kind.is_fifo() || kind.is_block_special() || kind.is_character_special() {
                "TAR special filesystem entries are not extracted"
            } else {
                "Unsupported TAR entry type"
            };
            save_unsupported(
                storage,
                budget,
                document_id,
                parent_id,
                &name,
                depth,
                Some("tar"),
                None,
                Some(size),
                reason,
            )?;
            continue;
        }

        save_file(
            storage,
            &mut entry,
            budget,
            document_id,
            parent_id,
            &name,
            depth,
            Some("tar"),
            None,
            Some(size),
        )?;
    }
    Ok(())
}

fn process_gzip(
    storage: &mut Storage,
    document_id: &str,
    source: &Path,
    name: &str,
    depth: u8,
    parent_id: Option<&str>,
    budget: &mut ArchiveBudget,
) -> Result<(), String> {
    let file = File::open(source).map_err(|e| format!("Cannot open GZIP: {e}"))?;
    let mut decoder = GzDecoder::new(file);
    let output_name = name
        .strip_suffix(".gz")
        .or_else(|| name.strip_suffix(".GZ"))
        .unwrap_or("extracted-content");

    save_file(
        storage,
        &mut decoder,
        budget,
        document_id,
        parent_id,
        output_name,
        depth,
        Some("gzip"),
        None,
        None,
    )
}

fn process_document(
    storage: &mut Storage,
    sha256: &str,
    name: &str,
    extension: &str,
    depth: u8,
    parent_id: Option<&str>,
    budget: &mut ArchiveBudget,
) -> Result<String, String> {
    let document_id = create_document(storage, sha256, extension, parent_id)?;
    let source = safe_blob_path(storage, sha256)?;
    let lower = name.to_ascii_lowercase();

    let result = if extension == "zip" || lower.ends_with(".zip") {
        process_zip(storage, &document_id, &source, depth, parent_id, budget)
    } else if extension == "tar" || lower.ends_with(".tar") {
        let file = File::open(&source).map_err(|e| format!("Cannot open TAR: {e}"))?;
        process_tar(storage, file, &document_id, depth, parent_id, budget)
    } else if extension == "tar.gz"
        || extension == "tgz"
        || lower.ends_with(".tar.gz")
        || lower.ends_with(".tgz")
    {
        let file = File::open(&source).map_err(|e| format!("Cannot open compressed TAR: {e}"))?;
        process_tar(
            storage,
            GzDecoder::new(file),
            &document_id,
            depth,
            parent_id,
            budget,
        )
    } else if extension == "gz" || lower.ends_with(".gz") {
        process_gzip(
            storage,
            &document_id,
            &source,
            name,
            depth,
            parent_id,
            budget,
        )
    } else {
        Err("Unsupported archive format".into())
    };

    match result {
        Ok(()) => {
            set_document_status(storage, &document_id, "completed", None)?;
            Ok(document_id)
        }
        Err(error) => {
            let _ = set_document_status(storage, &document_id, "failed", Some(&error));
            Err(error)
        }
    }
}

pub(super) fn index_imported_archive(
    storage: &mut Storage,
    sha256: &str,
    file_name: &str,
    extension: &str,
) -> Result<(), String> {
    let mut budget = ArchiveBudget::new();
    process_document(storage, sha256, file_name, extension, 0, None, &mut budget)?;
    Ok(())
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveEntryRecord {
    pub id: String,
    pub document_id: String,
    pub parent_id: Option<String>,
    pub entry_path: String,
    pub entry_name: String,
    pub entry_type: String,
    pub nested_depth: i64,
    pub compression_method: Option<String>,
    pub compressed_size: Option<i64>,
    pub uncompressed_size: Option<i64>,
    pub blob_sha256: Option<String>,
    pub nested_document_id: Option<String>,
    pub extraction_status: String,
    pub error_message: Option<String>,
    pub ordinal: i64,
    pub created_at: i64,
}

fn query_archive_entries(
    storage: &Storage,
    document_id: &str,
) -> Result<Vec<ArchiveEntryRecord>, String> {
    let mut statement = storage
        .connection
        .prepare(
            "SELECT
            id, document_id, parent_id, entry_path, entry_name,
            entry_type, nested_depth, compression_method,
            compressed_size, uncompressed_size, blob_sha256,
            nested_document_id, extraction_status, error_message,
            ordinal, created_at
         FROM archive_entries
         WHERE document_id = ?1
         ORDER BY ordinal ASC, entry_path COLLATE NOCASE ASC",
        )
        .map_err(|e| format!("Cannot prepare archive entry query: {e}"))?;

    let rows = statement
        .query_map(params![document_id], |row| {
            Ok(ArchiveEntryRecord {
                id: row.get(0)?,
                document_id: row.get(1)?,
                parent_id: row.get(2)?,
                entry_path: row.get(3)?,
                entry_name: row.get(4)?,
                entry_type: row.get(5)?,
                nested_depth: row.get(6)?,
                compression_method: row.get(7)?,
                compressed_size: row.get(8)?,
                uncompressed_size: row.get(9)?,
                blob_sha256: row.get(10)?,
                nested_document_id: row.get(11)?,
                extraction_status: row.get(12)?,
                error_message: row.get(13)?,
                ordinal: row.get(14)?,
                created_at: row.get(15)?,
            })
        })
        .map_err(|e| format!("Cannot query archive entries: {e}"))?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Cannot read archive entries: {e}"))
}

#[tauri::command]
pub fn list_archive_entries(
    state: tauri::State<'_, std::sync::Mutex<Storage>>,
    chat_id: String,
    reference_id: String,
    nested_entry_id: Option<String>,
) -> Result<Vec<ArchiveEntryRecord>, String> {
    let storage = state
        .lock()
        .map_err(|e| format!("Storage lock poisoned: {e}"))?;

    let root_document_id: String = storage
        .connection
        .query_row(
            "SELECT d.id
         FROM attachment_references r
         JOIN attachment_versions v ON v.id = r.attachment_version_id
         JOIN extracted_documents d ON d.blob_sha256 = v.blob_sha256
         WHERE r.id = ?1
           AND r.chat_id = ?2
           AND d.extractor = 'archive'
           AND d.extractor_version = '1:root'
         ORDER BY d.updated_at DESC
         LIMIT 1",
            params![reference_id, chat_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| format!("Cannot resolve archive attachment: {e}"))?
        .ok_or_else(|| "No indexed archive exists for this attachment in this chat".to_string())?;

    let selected_document_id = match nested_entry_id {
        None => root_document_id,
        Some(entry_id) => {
            let nested_document_id: Option<String> = storage
                .connection
                .query_row(
                    "WITH RECURSIVE reachable_documents(id) AS (
                    SELECT ?1
                    UNION
                    SELECT ae.nested_document_id
                    FROM archive_entries ae
                    JOIN reachable_documents rd ON ae.document_id = rd.id
                    WHERE ae.nested_document_id IS NOT NULL
                )
                SELECT ae.nested_document_id
                FROM archive_entries ae
                WHERE ae.id = ?2
                  AND ae.document_id IN (SELECT id FROM reachable_documents)",
                    params![root_document_id, entry_id],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|e| format!("Cannot resolve nested archive: {e}"))?
                .flatten();

            nested_document_id.ok_or_else(|| {
                "Nested archive entry does not belong to this attachment or is not indexed"
                    .to_string()
            })?
        }
    };

    query_archive_entries(&storage, &selected_document_id)
}

#[tauri::command]
pub fn read_archive_entry_chunk(
    state: tauri::State<'_, std::sync::Mutex<Storage>>,
    chat_id: String,
    reference_id: String,
    entry_id: String,
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
        .map_err(|e| format!("Storage lock poisoned: {e}"))?;

    let root_document_id: String = storage
        .connection
        .query_row(
            "SELECT d.id
         FROM attachment_references r
         JOIN attachment_versions v ON v.id = r.attachment_version_id
         JOIN extracted_documents d ON d.blob_sha256 = v.blob_sha256
         WHERE r.id = ?1
           AND r.chat_id = ?2
           AND d.extractor = 'archive'
           AND d.extractor_version = '1:root'
         ORDER BY d.updated_at DESC
         LIMIT 1",
            params![reference_id, chat_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| format!("Cannot resolve archive attachment: {e}"))?
        .ok_or_else(|| "Archive is not available in this chat".to_string())?;

    let blob_sha256: Option<String> = storage
        .connection
        .query_row(
            "WITH RECURSIVE reachable_documents(id) AS (
            SELECT ?1
            UNION
            SELECT ae.nested_document_id
            FROM archive_entries ae
            JOIN reachable_documents rd ON ae.document_id = rd.id
            WHERE ae.nested_document_id IS NOT NULL
         )
         SELECT ae.blob_sha256
         FROM archive_entries ae
         WHERE ae.id = ?2
           AND ae.document_id IN (SELECT id FROM reachable_documents)
           AND ae.entry_type IN ('file', 'archive')
           AND ae.extraction_status = 'completed'",
            params![root_document_id, entry_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| format!("Cannot resolve archive entry: {e}"))?
        .flatten();

    let sha256 = blob_sha256.ok_or_else(|| {
        "Archive entry is unavailable, unsupported, or does not belong to this archive".to_string()
    })?;

    let path = safe_blob_path(&storage, &sha256)?;
    let mut file = File::open(path).map_err(|e| format!("Cannot open archive entry: {e}"))?;

    let file_size = file
        .metadata()
        .map_err(|e| format!("Cannot inspect archive entry: {e}"))?
        .len();

    if offset > file_size {
        return Err("Archive entry offset is out of bounds".into());
    }

    let bytes_to_read = (file_size - offset).min(length as u64) as usize;

    file.seek(SeekFrom::Start(offset))
        .map_err(|e| format!("Cannot seek archive entry: {e}"))?;

    let mut buffer = vec![0u8; bytes_to_read];
    file.read_exact(&mut buffer)
        .map_err(|e| format!("Cannot read archive entry: {e}"))?;

    Ok(buffer)
}
