import CodeBlock from "./CodeBlock";
import GenerationIndicator from "./GenerationIndicator";
import { MarkdownText, parseContent } from "./MarkdownText";
import type { ResolvedMode } from "../modelRouting";
import type { AttachmentReference } from "./FileAttachments";

export type ChatMessage = {
  id?: number;
  dbId?: string;
  role: "user" | "assistant";
  content: string;
  promptContent?: string;
  attachments?: AttachmentReference[];
  modeUsed?: ResolvedMode;
  modelUsed?: string;
  timestamp: string;
  durationSec?: string;
  visionProgress?: string;
  includeInHistory?: boolean;
  turnId?: number;
};

interface ChatMessageProps {
  message: ChatMessage;
  index: number;
  totalMessages: number;
  isGenerating: boolean;
  onPreview: (code: string, lang: string) => void;
  highlightCode: (code: string, lang: string) => string;
  onCopy: (content: string) => void;
  onEdit: (index: number) => void;
}

export default function ChatMessageItem({
  message,
  index,
  totalMessages,
  isGenerating,
  onPreview,
  highlightCode,
  onCopy,
  onEdit,
}: ChatMessageProps) {
  const isLastAssistant = isGenerating && index === totalMessages - 1;
  const assistantLabel = isLastAssistant && !message.content ? "Sonne думає" : "Sonne";

  // Copy only the text shown in the user message bubble. `promptContent`
  // is model-only context and may include extracted document text and AI summaries.
  const copyContent = message.content;

  return (
    <div className={`msg ${message.role}`}>
      <div className="msg-meta">
        <span className="msg-role-label">{message.role === "user" ? "Ти" : assistantLabel}</span>
        <span className="msg-time">
          {message.timestamp}
          {message.durationSec ? ` · ${message.durationSec} с` : ""}
        </span>
      </div>

      {message.role === "user" ? (
        <>
          <div>{message.content}</div>
          <div className="msg-actions">
            <button
              type="button"
              className="msg-action-btn"
              onClick={() => onCopy(copyContent)}
              title="Копіювати текст запиту"
              aria-label="Копіювати текст запиту"
            >
              <svg aria-hidden="true" viewBox="0 0 24 24" width="17" height="17"
                fill="none" stroke="currentColor" strokeWidth="1.7"
                strokeLinecap="round" strokeLinejoin="round">
                <rect x="8" y="8" width="13" height="13" rx="2" />
                <path d="M16 8V5a2 2 0 0 0-2-2H5a2 2 0 0 0-2 2v9a2 2 0 0 0 2 2h3" />
              </svg>
            </button>
            <button
              type="button"
              className="msg-action-btn"
              onClick={() => onEdit(index)}
              title="Редагувати повідомлення"
              aria-label="Редагувати повідомлення"
            >
              <svg aria-hidden="true" viewBox="0 0 24 24" width="17" height="17"
                fill="none" stroke="currentColor" strokeWidth="1.7"
                strokeLinecap="round" strokeLinejoin="round">
                <path d="M12 20h9" />
                <path d="M16.5 3.5a2.12 2.12 0 0 1 3 3L8 18l-4 1 1-4Z" />
              </svg>
            </button>
          </div>
        </>
      ) : (
        <>
          {message.visionProgress && (
            <div className="vision-progress" role="status" aria-live="polite">
              {message.visionProgress}
            </div>
          )}
          {message.content &&
            parseContent(message.content).map((part, partIndex) =>
              part.type === "code" ? (
                <CodeBlock key={partIndex} lang={part.lang} code={part.code}
                  onPreview={onPreview} highlightCode={highlightCode} />
              ) : (
                <MarkdownText content={part.value} key={partIndex} />
              ),
            )}
          {isLastAssistant && <GenerationIndicator />}
        </>
      )}
    </div>
  );
}
