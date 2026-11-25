import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import ArchiveBrowser, { type ArchiveContextItem } from "./ArchiveBrowser";
import { processStoredAttachment } from "../fileReader";

export interface AttachmentReference {
  id: string;
  chatId: string;
  messageId: string | null;
  attachmentVersionId: string;
  attachmentId: string;
  versionNumber: number;
  fileName: string;
  extension: string;
  mimeType: string;
  blobSha256: string;
  originalSize: number;
  position: number;
  purpose: string;
  createdAt: string;
}

interface FileAttachmentsProps {
  chatId: string;
  attachments: AttachmentReference[];
  onAttachmentsChange: (attachments: AttachmentReference[]) => void;
  onArchiveContent: (item: ArchiveContextItem) => void;
  onPreviewAttachment: (attachment: AttachmentReference) => void;
}

function formatFileSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) {
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  }
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

export default function FileAttachments({
  chatId,
  attachments,
  onAttachmentsChange,
  onArchiveContent,
  onPreviewAttachment,
}: FileAttachmentsProps) {
  const [isDragging, setIsDragging] = useState(false);
  const [error, setError] = useState("");
  const [isImporting, setIsImporting] = useState(false);
  const importInProgress = useRef(false);
  const [archiveReferenceId, setArchiveReferenceId] = useState<string | null>(null);

  const processingReferences = useRef(new Set<string>());

  const [processingStatus, setProcessingStatus] = useState<Record<string, "processing" | "failed" | "completed">>({});

  useEffect(() => {
    let disposed = false;

    const pending = attachments.filter((attachment) => {
      if (/\.(zip|tar|gz|tgz)$/i.test(attachment.fileName)) return false;
      if (processingReferences.current.has(attachment.id)) return false;

      processingReferences.current.add(attachment.id);
      return true;
    });

    for (const attachment of pending) {
      setProcessingStatus((current) => ({
        ...current,
        [attachment.id]: "processing",
      }));

      void processStoredAttachment({
        id: attachment.id,
        chatId: attachment.chatId || chatId,
        fileName: attachment.fileName,
        mimeType: attachment.mimeType,
        originalSize: attachment.originalSize,
      })
        .then(() => {
          if (disposed) return;
          setProcessingStatus((current) => ({
            ...current,
            [attachment.id]: "completed",
          }));
        })
        .catch((cause) => {
          processingReferences.current.delete(attachment.id);
          if (disposed) return;

          setProcessingStatus((current) => ({
            ...current,
            [attachment.id]: "failed",
          }));

          const message =
            cause instanceof Error
              ? cause.message
              : "Не вдалося обробити вкладення.";
          setError(`${attachment.fileName}: ${message}`);
        });
    }

    

    return () => {
      disposed = true;
    };
  }, [attachments, chatId]);

  useEffect(() => {
    let disposed = false;
    let unlistenDrop: (() => void) | undefined;
    let unlistenImported: (() => void) | undefined;
    let unlistenError: (() => void) | undefined;

    async function setupListeners() {
      const importedListener = await listen<AttachmentReference[]>(
        "attachments-imported",
        (event) => {
          if (disposed) return;
          onAttachmentsChange(event.payload);
          setError("");
          setIsImporting(false);
          importInProgress.current = false;
        },
      );

      const errorListener = await listen<string>(
        "attachments-import-error",
        (event) => {
          if (disposed) return;
          setError(event.payload);
          setIsImporting(false);
          importInProgress.current = false;
        },
      );

      const dropListener = await getCurrentWebview().onDragDropEvent(
        (event) => {
          if (disposed) return;

          switch (event.payload.type) {
            case "enter":
            case "over":
              setIsDragging(true);
              break;

            case "leave":
              setIsDragging(false);
              break;

            case "drop": {
              setIsDragging(false);
              const paths = event.payload.paths;

              if (paths.length > 0) {
                void importPaths(paths);
              }
              break;
            }
          }
        },
      );

      if (disposed) {
        importedListener();
        errorListener();
        dropListener();
        return;
      }

      unlistenImported = importedListener;
      unlistenError = errorListener;
      unlistenDrop = dropListener;
    }

    void setupListeners().catch((cause) => {
      if (!disposed) {
        setError(
          cause instanceof Error
            ? cause.message
            : "Не вдалося підключити обробники вкладень.",
        );
      }
    });

    return () => {
      disposed = true;
      unlistenImported?.();
      unlistenError?.();
      unlistenDrop?.();
    };
  }, [chatId, onAttachmentsChange]);
  async function retryAttachment(attachment: AttachmentReference) {
    if (processingReferences.current.has(attachment.id)) return;

    processingReferences.current.add(attachment.id);
    setError("");
    setProcessingStatus((current) => ({
      ...current,
      [attachment.id]: "processing",
    }));

    try {
      await processStoredAttachment({
        id: attachment.id,
        chatId: attachment.chatId || chatId,
        fileName: attachment.fileName,
        mimeType: attachment.mimeType,
        originalSize: attachment.originalSize,
      });

      setProcessingStatus((current) => ({
        ...current,
        [attachment.id]: "completed",
      }));
    } catch (cause) {
      processingReferences.current.delete(attachment.id);
      setProcessingStatus((current) => ({
        ...current,
        [attachment.id]: "failed",
      }));

      const message =
        cause instanceof Error
          ? cause.message
          : "Не вдалося повторно обробити вкладення.";
      setError(`${attachment.fileName}: ${message}`);
    }
  }

  async function importPaths(paths: string[]) {
    if (importInProgress.current || paths.length === 0) return;

    importInProgress.current = true;
    setIsImporting(true);
    setError("");

    try {
      await invoke("import_files", { chatId, paths });
    } catch (cause) {
      importInProgress.current = false;
      setIsImporting(false);
      setError(
        cause instanceof Error ? cause.message : "Не вдалося імпортувати файли.",
      );
    }
  }

  async function openFileDialog() {
    if (importInProgress.current) return;

    setError("");

    try {
      await invoke("import_attachments", { chatId });
    } catch (cause) {
      setError(
        cause instanceof Error
          ? cause.message
          : "Не вдалося відкрити діалог вибору файлів.",
      );
    }
  }

  async function removeAttachment(referenceId: string) {
    setError("");

    try {
      await invoke("remove_attachment_reference", {
        referenceId,
      });

      const updated = await invoke<AttachmentReference[]>(
        "list_composer_attachments",
        { chatId },
      );

      onAttachmentsChange(updated);
    } catch (cause) {
      setError(
        cause instanceof Error
          ? cause.message
          : "Не вдалося видалити вкладення.",
      );
    }
  }

  return (
    <div className={`file-attachments ${isDragging ? "is-dragging" : ""}`}>
      <button
        type="button"
        className="file-attach-btn"
        onClick={() => void openFileDialog()}
        disabled={isImporting}
        title="Додати файли"
      >
        <span aria-hidden="true">＋</span> Додати файли
      </button>

      {attachments.length > 0 && (
        <div className="file-attachments-list">
          {attachments.map((attachment) => (
            <div className="file-attachment" key={attachment.id}>
              <div className="file-attachment-preview">
                <span className="file-attachment-name">
                  {attachment.fileName}
                </span>
                <span className="file-attachment-size">
                  {formatFileSize(attachment.originalSize)}
                </span>
                {processingStatus[attachment.id] === "processing" && (
                  <span className="file-attachments-status">Обробка...</span>
                )}
                {processingStatus[attachment.id] === "failed" && (
                  <>
                    <span className="file-attachments-error">Помилка обробки</span>
                    <button
                      type="button"
                      className="file-attachment-open"
                      onClick={() => void retryAttachment(attachment)}
                    >
                      Повторити
                    </button>
                  </>
                )}

                {!/\.(zip|tar|gz|tgz)$/i.test(attachment.fileName) && (
                  <button
                    type="button"
                    className="file-attachment-open"
                    onClick={() => onPreviewAttachment(attachment)}
                    title="Попередній перегляд файлу"
                    aria-label={`Переглянути ${attachment.fileName}`}
                  >
                    Переглянути
                  </button>
                )}
                {/\.(zip|tar|gz|tgz)$/i.test(attachment.fileName) && (
                  <button
                    type="button"
                    className="file-attachment-open"
                    onClick={() => setArchiveReferenceId(attachment.id)}
                    title="Переглянути вміст архіву"
                    aria-label={`Переглянути архів ${attachment.fileName}`}
                  >
                    Переглянути
                  </button>
                )}
              </div>

              <button
                type="button"
                className="file-attachment-remove"
                onClick={() => void removeAttachment(attachment.id)}
                aria-label={`Видалити ${attachment.fileName}`}
                title="Видалити файл"
              >
                ×
              </button>
            </div>
          ))}
        </div>
      )}

      {archiveReferenceId && (
        <ArchiveBrowser
          key={`${chatId}:${archiveReferenceId}`}
          chatId={chatId}
          referenceId={archiveReferenceId}
          onClose={() => setArchiveReferenceId(null)}
    onAddToContext={onArchiveContent}
        />
      )}

      {isImporting && (
        <div className="file-attachments-status">Імпорт файлів...</div>
      )}

      {error && (
        <div className="file-attachments-error" role="alert">
          {error}
        </div>
      )}

      {isDragging && (
        <div className="file-drop-overlay">
          Відпусти файли, щоб додати їх
        </div>
      )}
    </div>
  );
}
