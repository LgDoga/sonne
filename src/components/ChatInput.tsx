import { invoke } from "@tauri-apps/api/core";
import FileAttachments, {
  type AttachmentReference,
} from "./FileAttachments";
import type { ArchiveContextItem } from "./ArchiveBrowser";

interface ChatInputProps {
  input: string;
  onInputChange: (value: string) => void;
  isGenerating: boolean;
  elapsedSec: number;
  onSend: () => void | Promise<void>;
  onStop: () => void;
  chatId: string;
  attachments: AttachmentReference[];
  onAttachmentsChange: (attachments: AttachmentReference[]) => void;
  hasArchiveContext: boolean;
  onArchiveContent: (item: ArchiveContextItem) => void;
  onPreviewAttachment: (attachment: AttachmentReference) => void;
}

export default function ChatInput({
  input,
  onInputChange,
  isGenerating,
  elapsedSec,
  onSend,
  onStop,
  chatId,
  attachments,
  onAttachmentsChange,
  hasArchiveContext,
  onArchiveContent,
  onPreviewAttachment,
}: ChatInputProps) {
  return (
    <div className="sonne-input-wrap">
      <FileAttachments
        chatId={chatId}
        attachments={attachments}
        onAttachmentsChange={onAttachmentsChange}
        onArchiveContent={onArchiveContent}
        onPreviewAttachment={onPreviewAttachment}
      />

      <div className="input-box">
        <textarea
          onPaste={async (event) => {
              const imageItem = Array.from(event.clipboardData.items).find(
                (item) => item.kind === "file" && item.type.startsWith("image/"),
              );
              if (!imageItem) return;

              const file = imageItem.getAsFile();
              if (!file) return;

              event.preventDefault();

              try {
                const data = Array.from(new Uint8Array(await file.arrayBuffer()));
                const updated = await invoke<AttachmentReference[]>(
                  "import_clipboard_image",
                  { chatId, mimeType: file.type, data },
                );
                onAttachmentsChange(updated);
              } catch (error) {
                console.error("Clipboard image import failed:", error);
              }
            }}
            placeholder="Напиши запит... (Enter — надіслати, Shift+Enter — новий рядок)"
          value={input}
          onChange={(event) => onInputChange(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter" && !event.shiftKey && !isGenerating) {
              event.preventDefault();
              void onSend();
            }
          }}
        />

        <button
          className="send-btn"
          onClick={() => {
            if (isGenerating) onStop();
            else void onSend();
          }}
          disabled={
            !isGenerating && !input.trim() && attachments.length === 0 && !hasArchiveContext
          }
        >
          {isGenerating ? `Зупинити · ${elapsedSec.toFixed(0)} с` : "Надіслати"}
        </button>
      </div>
    </div>
  );
}
