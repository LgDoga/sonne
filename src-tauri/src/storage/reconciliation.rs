use rusqlite::Connection;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::Read,
    path::{Component, Path},
};

#[derive(Debug, Default, Clone)]
pub struct ReconciliationReport {
    pub scanned_files: usize,
    pub registered_blobs: usize,
    pub verified_blobs: usize,
    pub orphan_files: Vec<String>,
    pub missing_files: Vec<String>,
    pub invalid_paths: Vec<String>,
    pub hash_mismatches: Vec<String>,
    pub size_mismatches: Vec<String>,
    pub symlinks: Vec<String>,
    pub scan_errors: Vec<String>,
}

struct RegisteredBlob {
    sha256: String,
    relative_path: String,
    original_size: i64,
    stored_size: i64,
}

fn hash_file(path: &Path) -> Result<(String, u64), String> {
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 1024 * 1024];
    let mut size = 0u64;

    loop {
        let read = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        size = size.checked_add(read as u64)
            .ok_or_else(|| "File size overflow".to_string())?;
    }

    let hash = hasher.finalize().iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();

    Ok((hash, size))
}

fn validate_blob_relative_path(relative: &str) -> bool {
    let path = Path::new(relative);

    if path.is_absolute() || relative.contains('\\') {
        return false;
    }

    let components: Vec<_> = path.components().collect();
    if components.len() != 3
        || !matches!(components[0], Component::Normal(name) if name == "blobs")
    {
        return false;
    }

    let folder = match components[1] {
        Component::Normal(folder) => match folder.to_str() {
            Some(folder) => folder,
            None => return false,
        },
        _ => return false,
    };

    let filename = match components[2] {
        Component::Normal(filename) => match filename.to_str() {
            Some(filename) => filename,
            None => return false,
        },
        _ => return false,
    };

    let Some(hash) = filename.strip_suffix(".blob") else {
        return false;
    };

    folder.len() == 2
        && folder.chars().all(|character| character.is_ascii_hexdigit())
        && hash.len() == 64
        && hash.chars().all(|character| character.is_ascii_hexdigit())
        && hash == hash.to_ascii_lowercase()
        && folder == &hash[..2]
}

fn validate_blob_identity(relative: &str, sha256: &str) -> bool {
    if sha256.len() != 64
        || !sha256
            .chars()
            .all(|character| character.is_ascii_hexdigit())
        || sha256 != sha256.to_ascii_lowercase()
    {
        return false;
    }

    let expected = format!("blobs/{}/{}.blob", &sha256[..2], sha256);

    relative == expected && validate_blob_relative_path(relative)
}

fn scan_directory(
    directory: &Path,
    blobs_root: &Path,
    registered_paths: &std::collections::HashSet<String>,
    report: &mut ReconciliationReport,
) {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) => {
            report.scan_errors.push(format!("{}: {}", directory.display(), error));
            return;
        }
    };

    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                report.scan_errors.push(error.to_string());
                continue;
            }
        };

        let path = entry.path();

        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) => {
                report.scan_errors.push(format!("{}: {}", path.display(), error));
                continue;
            }
        };

        if metadata.file_type().is_symlink() {
            report.symlinks.push(path.display().to_string());
            continue;
        }

        if metadata.is_dir() {
            scan_directory(&path, blobs_root, registered_paths, report);
            continue;
        }

        if !metadata.is_file() {
            continue;
        }

        report.scanned_files += 1;

        let relative = match path.strip_prefix(blobs_root) {
            Ok(relative) => format!("blobs/{}", relative.to_string_lossy()),
            Err(_) => {
                report.invalid_paths.push(path.display().to_string());
                continue;
            }
        };

        if !registered_paths.contains(&relative) {
            report.orphan_files.push(relative);
        }
    }
}

pub fn reconcile(
    connection: &Connection,
    storage_root: &Path,
) -> Result<ReconciliationReport, String> {
    let blobs_root = storage_root.join("blobs");

    let mut statement = connection.prepare(
        "SELECT sha256, relative_path, original_size, stored_size FROM blobs"
    ).map_err(|e| format!("Cannot query blob registry: {e}"))?;

    let rows = statement.query_map([], |row| {
        Ok(RegisteredBlob {
            sha256: row.get(0)?,
            relative_path: row.get(1)?,
            original_size: row.get(2)?,
            stored_size: row.get(3)?,
        })
    }).map_err(|e| format!("Cannot read blob registry: {e}"))?;

    let registered: Vec<RegisteredBlob> = rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Cannot collect blob registry: {e}"))?;

    let registered_paths: std::collections::HashSet<String> = registered
        .iter()
        .filter(|blob| validate_blob_identity(&blob.relative_path, &blob.sha256))
        .map(|blob| blob.relative_path.clone())
        .collect();

    let mut report = ReconciliationReport {
        registered_blobs: registered.len(),
        ..Default::default()
    };

    let mut scan_blobs_root = false;
    let mut skip_registered_blob_checks = false;

    match fs::symlink_metadata(&blobs_root) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            report.symlinks.push(blobs_root.display().to_string());
            skip_registered_blob_checks = true;
        }
        Ok(metadata) if metadata.is_dir() => {
            scan_blobs_root = true;
        }
        Ok(_) => {
            report.invalid_paths.push(blobs_root.display().to_string());
            skip_registered_blob_checks = true;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            report.scan_errors.push(format!("{}: {}", blobs_root.display(), error));
            skip_registered_blob_checks = true;
        }
    }

    for blob in &registered {
        if skip_registered_blob_checks {
            continue;
        }
        if !validate_blob_identity(&blob.relative_path, &blob.sha256) {
            report.invalid_paths.push(blob.relative_path.clone());
            continue;
        }

        let path = storage_root.join(&blob.relative_path);

        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                report.missing_files.push(blob.relative_path.clone());
                continue;
            }
            Err(error) => {
                report.scan_errors.push(format!("{}: {}", blob.relative_path, error));
                continue;
            }
        };

        if metadata.file_type().is_symlink() {
            report.symlinks.push(blob.relative_path.clone());
            continue;
        }

        if !metadata.is_file() {
            report.invalid_paths.push(blob.relative_path.clone());
            continue;
        }

        match hash_file(&path) {
            Ok((hash, actual_size)) => {
                if hash != blob.sha256 {
                    report.hash_mismatches.push(blob.relative_path.clone());
                    continue;
                }

                report.verified_blobs += 1;

                if actual_size != blob.original_size as u64
                    || actual_size != blob.stored_size as u64
                {
                    report.size_mismatches.push(blob.relative_path.clone());
                }
            }
            Err(error) => {
                report.scan_errors.push(format!("{}: {}", blob.relative_path, error));
            }
        }
    }



    if scan_blobs_root {
        scan_directory(
            &blobs_root,
            &blobs_root,
            &registered_paths,
            &mut report,
        );
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, path::PathBuf};

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "sonne-reconciliation-{}-{}",
                std::process::id(),
                uuid::Uuid::new_v4()
            ));
            fs::create_dir(&path).unwrap_or_else(|error| {
                panic!(
                    "Cannot create test directory {}: {error}",
                    path.display()
                )
            });
            Self(path)
        }
    }
    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn test_hash(data: &[u8]) -> String {
        Sha256::digest(data).iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    fn setup() -> (TestDirectory, Connection) {
        let directory = TestDirectory::new();
        fs::create_dir_all(directory.0.join("blobs")).unwrap();

        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch(
            "CREATE TABLE blobs (
                sha256 TEXT PRIMARY KEY,
                relative_path TEXT NOT NULL UNIQUE,
                original_size INTEGER NOT NULL,
                stored_size INTEGER NOT NULL
            );"
        ).unwrap();

        (directory, connection)
    }

    fn register(connection: &Connection, data: &[u8], relative: &str) -> String {
        let hash = test_hash(data);
        connection.execute(
            "INSERT INTO blobs (sha256, relative_path, original_size, stored_size)
             VALUES (?1, ?2, ?3, ?3)",
            rusqlite::params![hash, relative, data.len() as i64],
        ).unwrap();
        hash
    }

    #[test]
    fn detects_valid_registered_blob() {
        let (directory, connection) = setup();
        let data = b"valid blob";
        let hash = test_hash(data);
        let relative = format!("blobs/{}/{}.blob", &hash[..2], hash);

        let path = directory.0.join(&relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, data).unwrap();
        register(&connection, data, &relative);

        let report = reconcile(&connection, &directory.0).unwrap();
        assert_eq!(report.verified_blobs, 1);
        assert!(report.missing_files.is_empty());
        assert!(report.hash_mismatches.is_empty());
        assert!(report.orphan_files.is_empty());
    }

    #[test]
    fn detects_missing_registered_blob() {
        let (directory, connection) = setup();
        let data = b"missing blob";
        let hash = test_hash(data);
        let relative = format!("blobs/{}/{}.blob", &hash[..2], hash);
        register(&connection, data, &relative);

        let report = reconcile(&connection, &directory.0).unwrap();
        assert_eq!(report.missing_files, vec![relative]);
    }

    #[test]
    fn detects_corrupted_blob_without_modifying_it() {
        let (directory, connection) = setup();
        let original = b"original content";
        let hash = test_hash(original);
        let relative = format!("blobs/{}/{}.blob", &hash[..2], hash);

        let path = directory.0.join(&relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"corrupted content").unwrap();
        register(&connection, original, &relative);

        let report = reconcile(&connection, &directory.0).unwrap();
        assert_eq!(report.hash_mismatches, vec![relative]);
        assert_eq!(fs::read(path).unwrap(), b"corrupted content");
    }

    #[test]
    fn detects_orphan_blob_without_deleting_it() {
        let (directory, connection) = setup();
        let data = b"orphan";
        let hash = test_hash(data);
        let relative = format!("blobs/{}/{}.blob", &hash[..2], hash);

        let path = directory.0.join(&relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, data).unwrap();

        let report = reconcile(&connection, &directory.0).unwrap();
        assert_eq!(report.orphan_files, vec![relative]);
        assert!(path.exists());
    }

    #[test]
    fn rejects_traversal_paths() {
        let hash = "a".repeat(64);
        let valid_relative = format!("blobs/{}/{}.blob", &hash[..2], hash);

        assert!(!validate_blob_relative_path("../outside.blob"));
        assert!(!validate_blob_relative_path("blobs/../../outside.blob"));
        assert!(!validate_blob_relative_path("/blobs/aa/file.blob"));
        assert!(!validate_blob_relative_path("blobs/aa/file.blob"));
        assert!(!validate_blob_relative_path(&format!("blobs/aa/{}.blob", "g".repeat(64))));
        assert!(!validate_blob_relative_path(&format!("blobs/bb/{hash}.blob")));
        assert!(validate_blob_relative_path(&valid_relative));
        assert!(validate_blob_identity(&valid_relative, &hash));
        assert!(!validate_blob_identity(
            &format!("blobs/bb/{hash}.blob"),
            &hash
        ));
    }

    #[cfg(unix)]
    #[test]
    fn refuses_to_scan_symlinked_blobs_root() {
        use std::os::unix::fs::symlink;

        let (directory, connection) = setup();
        let blobs_root = directory.0.join("blobs");
        let outside = directory.0.join("outside");

        fs::remove_dir(&blobs_root).unwrap();
        fs::create_dir_all(&outside).unwrap();

        let data = b"outside blob";
        let hash = test_hash(data);
        let relative = format!("blobs/{}/{}.blob", &hash[..2], hash);
        let outside_file = outside.join(format!("{}/{}.blob", &hash[..2], hash));

        fs::create_dir_all(outside_file.parent().unwrap()).unwrap();
        fs::write(&outside_file, data).unwrap();
        register(&connection, data, &relative);

        symlink(&outside, &blobs_root).unwrap();

        let report = reconcile(&connection, &directory.0).unwrap();

        assert!(report.symlinks.contains(&blobs_root.display().to_string()));
        assert_eq!(report.verified_blobs, 0);
        assert_eq!(report.scanned_files, 0);
        assert!(report.missing_files.is_empty());
        assert!(report.orphan_files.is_empty());
        assert!(outside_file.exists());
        assert_eq!(fs::read(&outside_file).unwrap(), data);
    }

    #[test]
    fn detects_size_mismatch_when_hash_is_valid() {
        let (directory, connection) = setup();
        let data = b"valid content";
        let hash = test_hash(data);
        let relative = format!("blobs/{}/{}.blob", &hash[..2], hash);

        let path = directory.0.join(&relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, data).unwrap();

        connection.execute(
            "INSERT INTO blobs (sha256, relative_path, original_size, stored_size)
             VALUES (?1, ?2, 999, 999)",
            rusqlite::params![hash, relative],
        ).unwrap();

        let report = reconcile(&connection, &directory.0).unwrap();
        assert_eq!(report.verified_blobs, 1);
        assert_eq!(report.size_mismatches, vec![relative]);
    }
}
