import CodeBlock from "./CodeBlock";
import GenerationIndicator from "./GenerationIndicator";
import { MarkdownText, parseContent } from "./MarkdownText";
import type { ResolvedMode } from "../modelRouting";

export type ChatMessage = {
  id?: number;
  role: "user" | "assistant";
  content: string;
  modeUsed?: ResolvedMode;
  modelUsed?: string;
  timestamp: string;
  durationSec?: string;
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
}

export default function ChatMessageItem({
  message,
  index,
  totalMessages,
  isGenerating,
  onPreview,
  highlightCode,
}: ChatMessageProps) {
  const isLastAssistant =
    isGenerating && index === totalMessages - 1;

  const assistantLabel =
    isLastAssistant && !message.content ? "Sonne думає" : "Sonne";

  return (
    <div className={`msg ${message.role}`}>
      <div className="msg-meta">
        <span className="msg-role-label">
          {message.role === "user" ? "Ти" : assistantLabel}
        </span>
        <span className="msg-time">
          {message.timestamp}
          {message.durationSec ? ` · ${message.durationSec} с` : ""}
        </span>
      </div>

      {message.role === "user" ? (
        <div>{message.content}</div>
      ) : (
        <>
          {message.content &&
            parseContent(message.content).map((part, partIndex) =>
              part.type === "code" ? (
                <CodeBlock
                  key={partIndex}
                  lang={part.lang}
                  code={part.code}
                  onPreview={onPreview}
                  highlightCode={highlightCode}
                />
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
