pub mod archive;
pub mod processing;
mod schema;

use std::{
    fs,
    path::{Path, PathBuf},
    sync::OnceLock,
    time::Duration,
};

use rusqlite::{Connection, OpenFlags};
use schema::{INITIAL_SCHEMA, MIGRATION_1_TO_2, MIGRATION_2_TO_3, MIGRATION_3_TO_4, MIGRATION_4_TO_5, SCHEMA_VERSION};

pub(super) const MAX_FILE_SIZE_BYTES: u64 = 512 * 1024 * 1024;
pub(super) const MAX_IMPORT_BATCH_SIZE_BYTES: u64 = 1024 * 1024 * 1024;
pub(super) const MAX_IMPORT_FILES: usize = 100;

pub struct Storage {
    connection: Connection,
    root_dir: PathBuf,
}

fn register_sqlite_vec() -> Result<(), String> {
    static REGISTRATION: OnceLock<Result<(), i32>> = OnceLock::new();

    REGISTRATION
        .get_or_init(|| unsafe {
            let result = rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute(
                sqlite_vec::sqlite3_vec_init as *const (),
            )));

            if result == rusqlite::ffi::SQLITE_OK {
                Ok(())
            } else {
                Err(result)
            }
        })
        .as_ref()
        .map(|_| ())
        .map_err(|code| format!("Cannot register sqlite-vec extension: SQLite error {code}"))
}

impl Storage {
    pub fn initialize(app_data_dir: &Path) -> Result<Self, String> {
        register_sqlite_vec()?;
        let root_dir = app_data_dir.join("storage");

        for directory in [
            root_dir.join("blobs"),
            root_dir.join("temp"),
            root_dir.join("extracted"),
            root_dir.join("indexes"),
        ] {
            fs::create_dir_all(&directory)
                .map_err(|error| format!("Cannot create storage directory: {error}"))?;
        }

        let database_path = root_dir.join("sonne.db");

        let mut connection = Connection::open_with_flags(
            &database_path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_FULL_MUTEX,
        )
        .map_err(|error| format!("Cannot open Sonne database: {error}"))?;

        let _: String = connection
            .query_row("SELECT vec_version()", [], |row| row.get(0))
            .map_err(|error| format!("sqlite-vec extension is unavailable: {error}"))?;

        connection
            .busy_timeout(Duration::from_secs(10))
            .map_err(|error| format!("Cannot configure database timeout: {error}"))?;

        connection
            .execute_batch(
                "
                PRAGMA foreign_keys = ON;
                PRAGMA journal_mode = WAL;
                PRAGMA synchronous = NORMAL;
                PRAGMA temp_store = MEMORY;
                PRAGMA mmap_size = 268435456;
                ",
            )
            .map_err(|error| format!("Cannot configure SQLite: {error}"))?;

        let current_version: i64 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .map_err(|error| format!("Cannot read database version: {error}"))?;

        if current_version > SCHEMA_VERSION {
            return Err(format!(
                "Database version {current_version} is newer than supported version {SCHEMA_VERSION}"
            ));
        }

        if current_version < SCHEMA_VERSION {
            let transaction = connection
                .transaction()
                .map_err(|error| format!("Cannot begin schema migration: {error}"))?;

            match current_version {
                0 => {
                    transaction
                        .execute_batch(INITIAL_SCHEMA)
                        .map_err(|error| format!("Cannot initialize database schema: {error}"))?;

                    transaction
                        .execute_batch(MIGRATION_1_TO_2)
                        .map_err(|error| {
                            format!("Cannot migrate database schema 1 to 2: {error}")
                        })?;
                    transaction
                        .execute_batch(MIGRATION_2_TO_3)
                        .map_err(|error| {
                            format!("Cannot migrate database schema 2 to 3: {error}")
                        })?;
                    transaction
                        .execute_batch(MIGRATION_3_TO_4)
                        .map_err(|error| {
                            format!("Cannot migrate database schema 3 to 4: {error}")
                        })?;
                    transaction
                        .execute_batch(MIGRATION_4_TO_5)
                        .map_err(|error| {
                            format!("Cannot migrate database schema 4 to 5: {error}")
                        })?;
                }
                1 => {
                    transaction
                        .execute_batch(MIGRATION_1_TO_2)
                        .map_err(|error| {
                            format!("Cannot migrate database schema 1 to 2: {error}")
                        })?;
                    transaction
                        .execute_batch(MIGRATION_2_TO_3)
                        .map_err(|error| {
                            format!("Cannot migrate database schema 2 to 3: {error}")
                        })?;
                    transaction
                        .execute_batch(MIGRATION_3_TO_4)
                        .map_err(|error| {
                            format!("Cannot migrate database schema 3 to 4: {error}")
                        })?;
                    transaction
                        .execute_batch(MIGRATION_4_TO_5)
                        .map_err(|error| {
                            format!("Cannot migrate database schema 4 to 5: {error}")
                        })?;
                }
                2 => {
                    transaction
                        .execute_batch(MIGRATION_2_TO_3)
                        .map_err(|error| {
                            format!("Cannot migrate database schema 2 to 3: {error}")
                        })?;
                    transaction
                        .execute_batch(MIGRATION_3_TO_4)
                        .map_err(|error| {
                            format!("Cannot migrate database schema 3 to 4: {error}")
                        })?;
                    transaction
                        .execute_batch(MIGRATION_4_TO_5)
                        .map_err(|error| {
                            format!("Cannot migrate database schema 4 to 5: {error}")
                        })?;
                }
                3 => {
                    transaction
                        .execute_batch(MIGRATION_3_TO_4)
                        .map_err(|error| {
                            format!("Cannot migrate database schema 3 to 4: {error}")
                        })?;
                    transaction
                        .execute_batch(MIGRATION_4_TO_5)
                        .map_err(|error| {
                            format!("Cannot migrate database schema 4 to 5: {error}")
                        })?;
                }
                4 => {
                    transaction
                        .execute_batch(MIGRATION_4_TO_5)
                        .map_err(|error| {
                            format!("Cannot migrate database schema 4 to 5: {error}")
                        })?;
                }
                version => {
                    return Err(format!("No migration path from database version {version}"));
                }
            }

            transaction
                .pragma_update(None, "user_version", SCHEMA_VERSION)
                .map_err(|error| format!("Cannot update database version: {error}"))?;

            transaction
                .commit()
                .map_err(|error| format!("Cannot commit schema migration: {error}"))?;
        }

        blobs::recover_interrupted_operations(&connection, &root_dir)?;
        processing::recover_interrupted_document_processing(&connection)?;
        Ok(Self {
            connection,
            root_dir,
        })
    }

    pub fn connection(&self) -> &Connection {
        &self.connection
    }

    pub fn root_dir(&self) -> &Path {
        &self.root_dir
    }
}

mod records;
pub use records::{ChatRecord, MessageRecord, StartTurnRecord};

pub mod commands;

mod blobs;
pub use blobs::BlobRecord;

pub mod attachments;
pub use attachments::{AttachmentReferenceRecord, AttachmentVersionRecord};

pub mod import;

mod preview;
pub use preview::handle_preview_request;

#[cfg(test)]
mod sqlite_vec_integration_tests {
    use super::Storage;
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

    #[test]
    fn initializes_sqlite_vec_and_performs_vector_search() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock error")
            .as_nanos();

        let directory = TestDirectory(
            std::env::temp_dir().join(format!("sonne-sqlite-vec-{}-{nonce}", std::process::id())),
        );

        fs::create_dir_all(&directory.0).expect("cannot create temporary test directory");

        let storage = Storage::initialize(&directory.0)
            .expect("storage initialization and migrations must succeed");

        let connection = storage.connection();

        let schema_version: i64 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .expect("cannot read schema version");

        assert_eq!(
            schema_version,
            super::schema::SCHEMA_VERSION,
            "database schema version must match the current version"
        );

        for table in ["document_summaries", "document_summary_parts"] {
            let exists: bool = connection
                .query_row(
                    "SELECT EXISTS(
                        SELECT 1 FROM sqlite_master
                        WHERE type = 'table' AND name = ?1
                    )",
                    [table],
                    |row| row.get(0),
                )
                .expect("failed to check schema v4 tables");

            assert!(exists, "missing schema v4 table: {table}");
        }

        let extension_version: String = connection
            .query_row("SELECT vec_version()", [], |row| row.get(0))
            .expect("sqlite-vec must be loaded");

        assert!(
            !extension_version.is_empty(),
            "sqlite-vec version must not be empty"
        );

        let table_exists: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type = 'table' AND name = 'document_vectors_nomic'",
                [],
                |row| row.get(0),
            )
            .expect("cannot inspect vector table");

        assert_eq!(table_exists, 1, "vec0 table must exist");

        let vector: Vec<f32> = (0..768)
            .map(|index| if index == 0 { 1.0 } else { 0.0 })
            .collect();

        let vector_bytes: Vec<u8> = vector
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect();

        connection
            .execute(
                "INSERT INTO document_vectors_nomic (chunk_id, embedding)
                 VALUES (?1, ?2)",
                rusqlite::params!["test-chunk-001", vector_bytes],
            )
            .expect("vec0 must accept a 768-dimensional embedding");

        let matched_chunk: String = connection
            .query_row(
                "SELECT chunk_id
                 FROM document_vectors_nomic
                 WHERE embedding MATCH ?1 AND k = 1
                 ORDER BY distance",
                [vector_bytes.as_slice()],
                |row| row.get(0),
            )
            .expect("vector similarity search must succeed");

        assert_eq!(matched_chunk, "test-chunk-001");

        drop(storage);

        println!("sqlite-vec version: {extension_version}");
        println!("schema version: {schema_version}");
        println!("vector table: available");
        println!("768-dimensional insert and similarity search: passed");
    }
}
