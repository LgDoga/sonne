import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { readFile } from "../fileReader";

interface ArchiveEntry {
  id: string;
  documentId: string;
  parentId: string | null;
  entryPath: string;
  entryName: string;
  entryType: string;
  nestedDepth: number;
  compressionMethod: string | null;
  compressedSize: number | null;
  uncompressedSize: number | null;
  blobSha256: string | null;
  nestedDocumentId: string | null;
  extractionStatus: string;
  errorMessage: string | null;
  ordinal: number;
  createdAt: number;
}

export interface ArchiveContextItem {
  referenceId: string;
  entryId: string;
  name: string;
  content: string;
}

interface ArchiveBrowserProps {
  chatId: string;
  referenceId: string;
  onClose: () => void;
  onAddToContext: (item: ArchiveContextItem) => void;
}

type ArchiveListItem =
  | { kind: "directory"; name: string; path: string }
  | { kind: "entry"; entry: ArchiveEntry };

const CHUNK_SIZE = 256 * 1024;
const MAX_ARCHIVE_PREVIEW_BYTES = 8 * 1024 * 1024;



function formatBytes(bytes: number | null): string {
  if (bytes === null) return "Розмір невідомий";
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function getEntryIcon(entry: ArchiveEntry): string {
  if (entry.entryType === "directory") return "▸";
  if (entry.entryType === "archive" || entry.nestedDocumentId) return "▣";
  return "▤";
}

export default function ArchiveBrowser({
  chatId,
  referenceId,
  onClose,
  onAddToContext,
}: ArchiveBrowserProps) {
  const [entries, setEntries] = useState<ArchiveEntry[]>([]);
  const [nestedStack, setNestedStack] = useState<
    Array<{ id: string; name: string }>
  >([]);
  const [directoryStack, setDirectoryStack] = useState<string[]>([]);
  const [loading, setLoading] = useState(false);
  const [readingId, setReadingId] = useState<string | null>(null);
  const [selectedEntry, setSelectedEntry] = useState<ArchiveEntry | null>(null);
  const [preview, setPreview] = useState<string | null>(null);
  const [previewNotice, setPreviewNotice] = useState("");
  const [error, setError] = useState<string | null>(null);

  const currentNestedEntryId = nestedStack[nestedStack.length - 1]?.id ?? null;

  const breadcrumbs = useMemo(
    () => [
      { id: null as string | null, name: "Архів" },
      ...nestedStack,
      ...directoryStack.map((name, index) => ({
        id: `directory:${index}:${name}`,
        name,
      })),
    ],
    [nestedStack, directoryStack],
  );

  const visibleEntries = useMemo<ArchiveListItem[]>(() => {
    const directories = new Map<string, ArchiveListItem>();
    const files: ArchiveListItem[] = [];
    const prefix = directoryStack.length
      ? `${directoryStack.join("/")}/`
      : "";

    for (const entry of entries) {
      const normalizedPath = entry.entryPath.replace(/^\/+|\/+$/g, "");
      if (!normalizedPath.startsWith(prefix)) continue;

      const relativePath = normalizedPath.slice(prefix.length);
      if (!relativePath) continue;

      const segments = relativePath.split("/").filter(Boolean);
      if (segments.length === 0) continue;

      if (segments.length > 1 || entry.entryType === "directory") {
        const name = segments[0];
        const directoryPath = [...directoryStack, name].join("/");

        directories.set(directoryPath, {
          kind: "directory",
          name,
          path: directoryPath,
        });
        continue;
      }

      files.push({ kind: "entry", entry });
    }

    return [...directories.values(), ...files].sort((a, b) => {
      if (a.kind !== b.kind) return a.kind === "directory" ? -1 : 1;

      const nameA = a.kind === "directory" ? a.name : a.entry.entryName;
      const nameB = b.kind === "directory" ? b.name : b.entry.entryName;

      return nameA.localeCompare(nameB, undefined, {
        numeric: true,
        sensitivity: "base",
      });
    });
  }, [entries, directoryStack]);

  useEffect(() => {
    let active = true;

    async function loadEntries() {
      setLoading(true);
      setError(null);
      setSelectedEntry(null);
      setPreview(null);
      setPreviewNotice("");

      try {
        const result = await invoke<ArchiveEntry[]>("list_archive_entries", {
          chatId,
          referenceId,
          nestedEntryId: currentNestedEntryId,
        });

        if (active) setEntries(result);
      } catch (cause) {
        if (active) {
          setEntries([]);
          setError(
            cause instanceof Error
              ? cause.message
              : String(cause),
          );
        }
      } finally {
        if (active) setLoading(false);
      }
    }

    void loadEntries();

    return () => {
      active = false;
    };
  }, [chatId, referenceId, currentNestedEntryId]);

  async function openTextEntry(entry: ArchiveEntry) {
    setSelectedEntry(entry);
    setPreview(null);
    setPreviewNotice("");
    setError(null);
    setReadingId(entry.id);

    try {
      if (entry.uncompressedSize === null) {
        setPreviewNotice("Розмір файла невідомий — попередній перегляд недоступний.");
        return;
      }

      if (entry.uncompressedSize > MAX_ARCHIVE_PREVIEW_BYTES) {
        setPreviewNotice(
          `Файл завеликий для попереднього перегляду (${formatBytes(entry.uncompressedSize)}). Ліміт — ${formatBytes(MAX_ARCHIVE_PREVIEW_BYTES)}.`,
        );
        return;
      }

      const chunks: Uint8Array[] = [];
      let offset = 0;

      while (offset < entry.uncompressedSize) {
        const requestedLength = Math.min(
          CHUNK_SIZE,
          entry.uncompressedSize - offset,
        );

        const bytes = await invoke<number[]>("read_archive_entry_chunk", {
          chatId,
          referenceId,
          entryId: entry.id,
          offset,
          length: requestedLength,
        });

        if (
          !Array.isArray(bytes) ||
          bytes.length === 0 ||
          bytes.length > requestedLength
        ) {
          throw new Error(
            `Не вдалося повністю прочитати файл "${entry.entryName}".`,
          );
        }

        chunks.push(Uint8Array.from(bytes));
        offset += bytes.length;

        if (bytes.length < requestedLength && offset < entry.uncompressedSize) {
          throw new Error(
            `Архів повернув неповний вміст файла "${entry.entryName}".`,
          );
        }
      }

      if (offset !== entry.uncompressedSize) {
        throw new Error(
          `Розмір прочитаного файла "${entry.entryName}" не збігається з очікуваним.`,
        );
      }

      const combined = new Uint8Array(offset);
      let position = 0;

      for (const chunk of chunks) {
        combined.set(chunk, position);
        position += chunk.length;
      }

      const file = new File([combined], entry.entryName, {
        type: "application/octet-stream",
      });

      const result = await readFile(file);
      setPreview(result.content);

      if (result.type === "image") {
        setPreviewNotice("Текст отримано розпізнаванням OCR.");
      }
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setReadingId(null);
    }
  }

  function openDirectory(item: Extract<ArchiveListItem, { kind: "directory" }>) {
    setDirectoryStack(item.path.split("/").filter(Boolean));
    setSelectedEntry(null);
    setPreview(null);
    setPreviewNotice("");
    setError(null);
  }

  function openNestedArchive(entry: ArchiveEntry) {
    if (!entry.nestedDocumentId) return;

    setDirectoryStack([]);
    setNestedStack((current) => [
      ...current,
      { id: entry.id, name: entry.entryName },
    ]);
  }

  function navigateToBreadcrumb(index: number) {
    if (index <= nestedStack.length) {
      setNestedStack((current) => current.slice(0, index));
      setDirectoryStack([]);
    } else {
      setDirectoryStack((current) =>
        current.slice(0, index - nestedStack.length),
      );
    }

    setSelectedEntry(null);
    setPreview(null);
    setPreviewNotice("");
    setError(null);
  }

  return (
    <section className="archive-browser" aria-label="Перегляд архіву">
      <header className="archive-browser__header">
        <div className="archive-browser__heading">
          <strong>Вміст архіву</strong>
          <span>{visibleEntries.length} елементів</span>
        </div>
        <button
          type="button"
          className="archive-browser__close"
          onClick={onClose}
          aria-label="Закрити перегляд архіву"
        >
          ×
        </button>
      </header>

      <nav className="archive-browser__breadcrumbs" aria-label="Шлях в архіві">
        {breadcrumbs.map((item, index) => (
          <span className="archive-browser__breadcrumb" key={item.id ?? "root"}>
            {index > 0 && <span aria-hidden="true">/</span>}
            <button
              type="button"
              onClick={() => navigateToBreadcrumb(index)}
              disabled={index === breadcrumbs.length - 1}
            >
              {item.name}
            </button>
          </span>
        ))}
      </nav>

      {error && <p className="archive-browser__error">{error}</p>}

      <div className="archive-browser__content">
        <div className="archive-browser__list">
          {loading ? (
            <p className="archive-browser__status">Завантаження вмісту…</p>
          ) : visibleEntries.length === 0 ? (
            <p className="archive-browser__status">
              У цій папці немає доступних елементів.
            </p>
          ) : (
            visibleEntries.map((item) => {
              if (item.kind === "directory") {
                return (
                  <button
                    type="button"
                    key={`directory:${item.path}`}
                    className="archive-browser__entry"
                    onClick={() => openDirectory(item)}
                    title={item.path}
                  >
                    <span className="archive-browser__entry-icon">▸</span>
                    <span className="archive-browser__entry-name">
                      {item.name}
                    </span>
                    <span className="archive-browser__entry-size">
                      Папка
                    </span>
                    <span className="archive-browser__entry-arrow">›</span>
                  </button>
                );
              }

              const entry = item.entry;
              const isNestedArchive =
                entry.entryType === "archive" && !!entry.nestedDocumentId;

              return (
                <button
                  type="button"
                  key={entry.id}
                  className={`archive-browser__entry${
                    selectedEntry?.id === entry.id
                      ? " archive-browser__entry--selected"
                      : ""
                  }`}
                  onClick={() => {
                    if (isNestedArchive) openNestedArchive(entry);
                    else void openTextEntry(entry);
                  }}
                  disabled={
                    !isNestedArchive &&
                    entry.extractionStatus !== "completed"
                  }
                  title={entry.errorMessage ?? entry.entryPath}
                >
                  <span className="archive-browser__entry-icon">
                    {getEntryIcon(entry)}
                  </span>
                  <span className="archive-browser__entry-name">
                    {entry.entryName}
                  </span>
                  <span className="archive-browser__entry-size">
                    {formatBytes(entry.uncompressedSize)}
                  </span>
                  {isNestedArchive && (
                    <span className="archive-browser__entry-arrow">›</span>
                  )}
                  {entry.extractionStatus !== "completed" && (
                    <span className="archive-browser__entry-status">
                      Недоступний
                    </span>
                  )}
                </button>
              );
            })
          )}
        </div>

        <div className="archive-browser__preview">
          {!selectedEntry ? (
            <p className="archive-browser__status">
              Вибери файл, щоб переглянути його вміст.
            </p>
          ) : (
            <>
              <div className="archive-browser__preview-header">
                <strong>{selectedEntry.entryName}</strong>
                <span>{formatBytes(selectedEntry.uncompressedSize)}</span>
              </div>
              {readingId === selectedEntry.id ? (
                <p className="archive-browser__status">Читання файла…</p>
              ) : preview !== null ? (
                <div className="archive-browser__text-result">
                  <pre className="archive-browser__text-preview">{preview}</pre>
                  <button
                    type="button"
                    className="archive-browser__add-context"
                    onClick={() => onAddToContext({
                      referenceId,
                      entryId: selectedEntry.id,
                      name: selectedEntry.entryPath,
                      content: preview,
                    })}
                  >
                    Додати в контекст
                  </button>
                </div>
              ) : (
                <p className="archive-browser__status">
                  {previewNotice || "Попередній перегляд недоступний."}
                </p>
              )}
              {previewNotice && preview !== null && (
                <p className="archive-browser__notice">{previewNotice}</p>
              )}
            </>
          )}
        </div>
      </div>
    </section>
  );
}
