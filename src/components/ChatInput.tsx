interface ChatInputProps {
  input: string;
  onInputChange: (value: string) => void;
  isGenerating: boolean;
  elapsedSec: number;
  onSend: () => void | Promise<void>;
  onStop: () => void;
}

export default function ChatInput({
  input,
  onInputChange,
  isGenerating,
  elapsedSec,
  onSend,
  onStop,
}: ChatInputProps) {
  return (
    <div className="sonne-input-wrap">
      <div className="input-box">
        <textarea
          placeholder="Напиши запит... (Enter — надіслати, Shift+Enter — новий рядок)"
          value={input}
          onChange={(event) => onInputChange(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter" && !event.shiftKey) {
              event.preventDefault();
              void onSend();
            }
          }}
          disabled={isGenerating}
        />
        <button
          className="send-btn"
          onClick={() => {
            if (isGenerating) {
              onStop();
            } else {
              void onSend();
            }
          }}
          disabled={!isGenerating && !input.trim()}
        >
          {isGenerating
            ? `Зупинити · ${elapsedSec.toFixed(0)} с`
            : "Надіслати"}
        </button>
      </div>
    </div>
  );
}
