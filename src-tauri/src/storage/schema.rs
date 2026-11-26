pub const SCHEMA_VERSION: i64 = 5;

pub const INITIAL_SCHEMA: &str = r#"
PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS chats (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL DEFAULT '',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS messages (
    id TEXT PRIMARY KEY,
    chat_id TEXT NOT NULL,
    role TEXT NOT NULL CHECK (role IN ('user', 'assistant', 'system')),
    content TEXT NOT NULL DEFAULT '',
    prompt_content TEXT,
    model_used TEXT,
    mode_used TEXT,
    created_at INTEGER NOT NULL,
    FOREIGN KEY (chat_id) REFERENCES chats(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_messages_chat_created
ON messages(chat_id, created_at);

-- One physical file per SHA-256 hash.
CREATE TABLE IF NOT EXISTS blobs (
    sha256 TEXT PRIMARY KEY,
    relative_path TEXT NOT NULL UNIQUE,
    original_size INTEGER NOT NULL CHECK (original_size >= 0),
    stored_size INTEGER NOT NULL CHECK (stored_size >= 0),
    storage_state TEXT NOT NULL DEFAULT 'original'
        CHECK (storage_state IN ('original', 'compressed', 'processing')),
    compression_format TEXT,
    created_at INTEGER NOT NULL,
    last_accessed_at INTEGER NOT NULL,
    verified_at INTEGER
);

CREATE TABLE IF NOT EXISTS attachments (
    id TEXT PRIMARY KEY,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS attachment_versions (
    id TEXT PRIMARY KEY,
    attachment_id TEXT NOT NULL,
    version_number INTEGER NOT NULL CHECK (version_number > 0),
    blob_sha256 TEXT NOT NULL,
    file_name TEXT NOT NULL,
    extension TEXT NOT NULL DEFAULT '',
    mime_type TEXT NOT NULL DEFAULT 'application/octet-stream',
    created_at INTEGER NOT NULL,
    UNIQUE (attachment_id, version_number),
    FOREIGN KEY (attachment_id) REFERENCES attachments(id) ON DELETE CASCADE,
    FOREIGN KEY (blob_sha256) REFERENCES blobs(sha256) ON DELETE RESTRICT
);

CREATE INDEX IF NOT EXISTS idx_attachment_versions_blob
ON attachment_versions(blob_sha256);

-- A reference may belong to a composer draft or a sent message.
-- Chat ownership allows safe cleanup when a chat is permanently deleted.
CREATE TABLE IF NOT EXISTS attachment_references (
    id TEXT PRIMARY KEY,
    chat_id TEXT NOT NULL,
    attachment_version_id TEXT NOT NULL,
    message_id TEXT,
    purpose TEXT NOT NULL CHECK (purpose IN ('composer', 'message')),
    position INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    FOREIGN KEY (chat_id) REFERENCES chats(id) ON DELETE CASCADE,
    FOREIGN KEY (message_id) REFERENCES messages(id) ON DELETE CASCADE,
    FOREIGN KEY (attachment_version_id)
        REFERENCES attachment_versions(id) ON DELETE RESTRICT
);

CREATE INDEX IF NOT EXISTS idx_attachment_refs_chat
ON attachment_references(chat_id, purpose, position);

CREATE INDEX IF NOT EXISTS idx_attachment_refs_version
ON attachment_references(attachment_version_id);

CREATE INDEX IF NOT EXISTS idx_attachment_refs_message
ON attachment_references(message_id);

-- Extraction results are reusable for identical physical files.
CREATE TABLE IF NOT EXISTS extracted_documents (
    id TEXT PRIMARY KEY,
    blob_sha256 TEXT NOT NULL,
    extractor TEXT NOT NULL,
    extractor_version TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'processing', 'completed', 'failed')),
    language TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE (blob_sha256, extractor, extractor_version),
    FOREIGN KEY (blob_sha256) REFERENCES blobs(sha256) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS document_chunks (
    id TEXT PRIMARY KEY,
    document_id TEXT NOT NULL,
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    content TEXT NOT NULL,
    token_count INTEGER,
    start_offset INTEGER,
    end_offset INTEGER,
    UNIQUE (document_id, ordinal),
    FOREIGN KEY (document_id) REFERENCES extracted_documents(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_document_chunks_document
ON document_chunks(document_id, ordinal);

-- Embeddings are versioned by model name; vectors are stored as float32 BLOBs.
CREATE TABLE IF NOT EXISTS embeddings (
    chunk_id TEXT NOT NULL,
    model_name TEXT NOT NULL,
    dimensions INTEGER NOT NULL CHECK (dimensions > 0),
    vector BLOB NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (chunk_id, model_name),
    FOREIGN KEY (chunk_id) REFERENCES document_chunks(id) ON DELETE CASCADE
);

-- Full-text search is kept in sync with document_chunks.
CREATE VIRTUAL TABLE IF NOT EXISTS chunk_search USING fts5(
    chunk_id UNINDEXED,
    content,
    tokenize = 'unicode61 remove_diacritics 2'
);

CREATE TRIGGER IF NOT EXISTS document_chunks_ai
AFTER INSERT ON document_chunks
BEGIN
    INSERT INTO chunk_search(chunk_id, content)
    VALUES (new.id, new.content);
END;

CREATE TRIGGER IF NOT EXISTS document_chunks_ad
AFTER DELETE ON document_chunks
BEGIN
    DELETE FROM chunk_search WHERE chunk_id = old.id;
END;

CREATE TRIGGER IF NOT EXISTS document_chunks_au
AFTER UPDATE OF content ON document_chunks
BEGIN
    DELETE FROM chunk_search WHERE chunk_id = old.id;
    INSERT INTO chunk_search(chunk_id, content)
    VALUES (new.id, new.content);
END;

-- Crash-safe journal for filesystem operations.
CREATE TABLE IF NOT EXISTS storage_operations (
    id TEXT PRIMARY KEY,
    operation_type TEXT NOT NULL,
    blob_sha256 TEXT,
    source_path TEXT,
    target_path TEXT,
    state TEXT NOT NULL
        CHECK (state IN (
            'PENDING',
            'PROCESSING',
            'VERIFYING',
            'COMMITTING',
            'COMPLETED',
            'FAILED'
        )),
    metadata_json TEXT,
    error_message TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    FOREIGN KEY (blob_sha256) REFERENCES blobs(sha256) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_storage_operations_state
ON storage_operations(state, updated_at);

CREATE TABLE IF NOT EXISTS storage_metadata (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

INSERT OR IGNORE INTO storage_metadata(key, value)
VALUES ('schema_version', '1');
"#;

pub const MIGRATION_2_TO_3: &str = r#"
CREATE VIRTUAL TABLE IF NOT EXISTS document_vectors_nomic USING vec0(
    chunk_id TEXT PRIMARY KEY,
    embedding float[768] distance_metric=cosine
);
"#;

pub const MIGRATION_1_TO_2: &str = r#"
ALTER TABLE extracted_documents ADD COLUMN detected_format TEXT;
ALTER TABLE extracted_documents ADD COLUMN metadata_json TEXT;
ALTER TABLE extracted_documents ADD COLUMN content_relative_path TEXT;
ALTER TABLE extracted_documents ADD COLUMN preview_relative_path TEXT;
ALTER TABLE extracted_documents ADD COLUMN error_code TEXT;
ALTER TABLE extracted_documents ADD COLUMN error_message TEXT;
ALTER TABLE extracted_documents ADD COLUMN processed_at INTEGER;
ALTER TABLE extracted_documents ADD COLUMN source_size INTEGER;

CREATE TABLE document_sections (
    id TEXT PRIMARY KEY,
    document_id TEXT NOT NULL,
    parent_id TEXT,
    section_type TEXT NOT NULL CHECK (
        section_type IN (
            'root', 'page', 'sheet', 'slide', 'chapter', 'table',
            'paragraph', 'header', 'footer', 'notes', 'archive_entry'
        )
    ),
    title TEXT,
    ordinal INTEGER NOT NULL DEFAULT 0,
    metadata_json TEXT,
    created_at INTEGER NOT NULL,
    FOREIGN KEY (document_id) REFERENCES extracted_documents(id) ON DELETE CASCADE,
    FOREIGN KEY (parent_id) REFERENCES document_sections(id) ON DELETE CASCADE
);

CREATE INDEX idx_document_sections_document
ON document_sections(document_id, ordinal);

CREATE INDEX idx_document_sections_parent
ON document_sections(parent_id, ordinal);

ALTER TABLE document_chunks ADD COLUMN section_id TEXT REFERENCES document_sections(id) ON DELETE SET NULL;
ALTER TABLE document_chunks ADD COLUMN chunk_type TEXT NOT NULL DEFAULT 'text';
ALTER TABLE document_chunks ADD COLUMN metadata_json TEXT;

CREATE INDEX idx_document_chunks_section
ON document_chunks(section_id, ordinal);

CREATE TABLE document_assets (
    id TEXT PRIMARY KEY,
    document_id TEXT NOT NULL,
    section_id TEXT,
    blob_sha256 TEXT,
    asset_type TEXT NOT NULL,
    mime_type TEXT,
    file_name TEXT,
    metadata_json TEXT,
    ordinal INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    FOREIGN KEY (document_id) REFERENCES extracted_documents(id) ON DELETE CASCADE,
    FOREIGN KEY (section_id) REFERENCES document_sections(id) ON DELETE SET NULL,
    FOREIGN KEY (blob_sha256) REFERENCES blobs(sha256) ON DELETE SET NULL
);

CREATE INDEX idx_document_assets_document
ON document_assets(document_id, ordinal);

CREATE TABLE archive_entries (
    id TEXT PRIMARY KEY,
    document_id TEXT NOT NULL,
    parent_id TEXT,
    entry_path TEXT NOT NULL,
    entry_name TEXT NOT NULL,
    entry_type TEXT NOT NULL CHECK (
        entry_type IN ('file', 'directory', 'archive', 'unsupported')
    ),
    nested_depth INTEGER NOT NULL DEFAULT 0 CHECK (nested_depth BETWEEN 0 AND 2),
    compression_method TEXT,
    compressed_size INTEGER,
    uncompressed_size INTEGER,
    blob_sha256 TEXT,
    nested_document_id TEXT,
    extraction_status TEXT NOT NULL DEFAULT 'pending',
    error_message TEXT,
    ordinal INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    FOREIGN KEY (document_id) REFERENCES extracted_documents(id) ON DELETE CASCADE,
    FOREIGN KEY (parent_id) REFERENCES archive_entries(id) ON DELETE CASCADE,
    FOREIGN KEY (blob_sha256) REFERENCES blobs(sha256) ON DELETE SET NULL,
    FOREIGN KEY (nested_document_id) REFERENCES extracted_documents(id) ON DELETE SET NULL
);

CREATE INDEX idx_archive_entries_document
ON archive_entries(document_id, ordinal);

CREATE INDEX idx_archive_entries_parent
ON archive_entries(parent_id, ordinal);

UPDATE storage_metadata SET value = '2' WHERE key = 'schema_version';
"#;

pub const MIGRATION_3_TO_4: &str = r#"

CREATE TABLE IF NOT EXISTS document_summaries (
    document_id TEXT PRIMARY KEY,
    summary TEXT NOT NULL DEFAULT '',
    model_name TEXT NOT NULL,
    summarizer_version TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'processing', 'completed', 'failed')),
    source_chunk_count INTEGER NOT NULL DEFAULT 0,
    error_message TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    FOREIGN KEY (document_id) REFERENCES extracted_documents(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_document_summaries_status
ON document_summaries(status, updated_at);

CREATE TABLE IF NOT EXISTS document_summary_parts (
    document_id TEXT NOT NULL,
    stage INTEGER NOT NULL CHECK (stage BETWEEN 0 AND 32),
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    content TEXT NOT NULL,
    source_chunk_count INTEGER NOT NULL DEFAULT 0,
    model_name TEXT NOT NULL,
    summarizer_version TEXT NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (document_id, stage, ordinal),
    FOREIGN KEY (document_id) REFERENCES extracted_documents(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_document_summary_parts_stage
ON document_summary_parts(document_id, stage, ordinal);

"#;

pub const MIGRATION_4_TO_5: &str = r#"
-- The operation journal must retain immutable blob hashes after blob metadata
-- is deleted. A foreign key with ON DELETE SET NULL destroys the recovery key.
CREATE TABLE storage_operations_v5 (
    id TEXT PRIMARY KEY,
    operation_type TEXT NOT NULL,
    blob_sha256 TEXT,
    source_path TEXT,
    target_path TEXT,
    state TEXT NOT NULL
        CHECK (state IN (
            'PENDING',
            'PROCESSING',
            'VERIFYING',
            'COMMITTING',
            'COMPLETED',
            'FAILED'
        )),
    metadata_json TEXT,
    error_message TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

INSERT INTO storage_operations_v5 (
    id, operation_type, blob_sha256, source_path, target_path,
    state, metadata_json, error_message, created_at, updated_at
)
SELECT
    id, operation_type, blob_sha256, source_path, target_path,
    state, metadata_json, error_message, created_at, updated_at
FROM storage_operations;

DROP TABLE storage_operations;
ALTER TABLE storage_operations_v5 RENAME TO storage_operations;

CREATE INDEX idx_storage_operations_state
ON storage_operations(state, updated_at);

UPDATE storage_metadata SET value = '5' WHERE key = 'schema_version';
"#;
