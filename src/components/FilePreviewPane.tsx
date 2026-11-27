import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  EXTRACTOR_ID,
  EXTRACTOR_VERSION,
  loadStoredAttachment,
  readStoredAttachment,
} from "../fileReader";
import { createEmbeddings, OLLAMA_HOST } from "../ollamaClient";
import type { AttachmentReference } from "./FileAttachments";

interface DocumentSearchResult {
  chunkId: string;
  fileName: string;
  detectedFormat: string | null;
  content: string;
}

interface FilePreviewPaneProps {
  attachment: AttachmentReference;
  onClose: () => void;
}

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

export default function FilePreviewPane({
  attachment,
  onClose,
}: FilePreviewPaneProps) {
  const [content, setContent] = useState("");
  const [imageUrl, setImageUrl] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [searchQuery, setSearchQuery] = useState("");
  const [searchResults, setSearchResults] = useState<DocumentSearchResult[]>([]);
  const [searching, setSearching] = useState(false);
  const [searchError, setSearchError] = useState("");

  async function searchDocuments() {
    const query = searchQuery.trim();
    if (!query || searching) return;

    setSearching(true);
    setSearchError("");
    setSearchResults([]);

    try {
      const vectors = await createEmbeddings(OLLAMA_HOST, [query]);
      const queryEmbedding = vectors[0];
      if (!queryEmbedding || queryEmbedding.length !== 768) {
        throw new Error("Не вдалося створити embedding пошукового запиту.");
      }
      const results = await invoke<DocumentSearchResult[]>(
        "search_attachment_documents",
        {
          chatId: attachment.chatId,
          messageId: attachment.messageId,
          referenceId: attachment.id,
          attachmentVersionId: attachment.attachmentVersionId,
          versionNumber: attachment.versionNumber,
          extractor: EXTRACTOR_ID,
          extractorVersion: EXTRACTOR_VERSION,
          query,
          queryEmbedding,
          limit: 30,
        },
      );
      setSearchResults(results);
      if (results.length === 0) setSearchError("Нічого не знайдено.");
    } catch (cause) {
      setSearchError(
        cause instanceof Error ? cause.message : "Не вдалося виконати пошук.",
      );
    } finally {
      setSearching(false);
    }
  }

  useEffect(() => {
    let cancelled = false;
    let objectUrl: string | null = null;

    setContent("");
    setImageUrl(null);
    setError("");
    setLoading(true);

    const input = {
      id: attachment.id,
      chatId: attachment.chatId,
      attachmentVersionId: attachment.attachmentVersionId,
      versionNumber: attachment.versionNumber,
      fileName: attachment.fileName,
      mimeType: attachment.mimeType,
      originalSize: attachment.originalSize,
    };

    async function loadPreview() {
      try {
        const isImage =
          attachment.mimeType.toLowerCase().startsWith("image/") ||
          /\.(png|jpe?g|webp|gif|bmp|tiff?|avif|heic|heif)$/i.test(
            attachment.fileName,
          );

        if (isImage) {
          const file = await loadStoredAttachment(input);
          objectUrl = URL.createObjectURL(file);
          if (!cancelled) setImageUrl(objectUrl);
        } else {
          const result = await readStoredAttachment(input);
          if (!cancelled) {
            setContent(
              result.content.trim()
                ? result.content
                : "Для цього файлу не вдалося отримати текстовий вміст.",
            );
          }
        }
      } catch (cause) {
        if (!cancelled) {
          setError(
            cause instanceof Error
              ? cause.message
              : "Не вдалося відкрити попередній перегляд файлу.",
          );
        }
      } finally {
        if (!cancelled) setLoading(false);
      }
    }

    void loadPreview();

    return () => {
      cancelled = true;
      if (objectUrl) URL.revokeObjectURL(objectUrl);
    };
  }, [
    attachment.id,
    attachment.chatId,
    attachment.fileName,
    attachment.mimeType,
    attachment.originalSize,
  ]);

  return (
    <aside className="file-preview-pane" aria-label="Попередній перегляд файлу">
      <div className="file-preview-header">
        <span className="file-preview-title" title={attachment.fileName}>
          {attachment.fileName}
        </span>
        <button
          type="button"
          className="file-preview-close"
          onClick={onClose}
          aria-label="Закрити попередній перегляд"
          title="Закрити"
        >
          ×
        </button>
      </div>
      <div className="file-preview-meta">
        {attachment.mimeType || "Невідомий формат"} · {formatSize(attachment.originalSize)}
      </div>
      <form
        className="file-preview-search"
        onSubmit={(event) => {
          event.preventDefault();
          void searchDocuments();
        }}
      >
        <input
          type="search"
          value={searchQuery}
          onChange={(event) => setSearchQuery(event.target.value)}
          placeholder="Пошук у цьому файлі…"
          aria-label="Пошук у цьому файлі"
          maxLength={512}
        />
        <button type="submit" disabled={searching || !searchQuery.trim()}>
          {searching ? "…" : "Знайти"}
        </button>
      </form>
      {(searchResults.length > 0 || searchError) && (
        <div className="file-preview-search-results" aria-live="polite">
          {searchError && <p className="file-preview-message">{searchError}</p>}
          {searchResults.map((result) => (
            <article className="file-preview-search-result" key={result.chunkId}>
              <strong title={result.fileName}>{result.fileName}</strong>
              <p>{result.content}</p>
            </article>
          ))}
        </div>
      )}
      <div className="file-preview-content">
        {loading ? (
          <p className="file-preview-message">Завантаження попереднього перегляду…</p>
        ) : error ? (
          <p className="file-preview-message" role="alert">{error}</p>
        ) : imageUrl ? (
          <img src={imageUrl} alt={attachment.fileName} />
        ) : (
          <pre>{content}</pre>
        )}
      </div>
    </aside>
  );
}
