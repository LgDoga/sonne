import ModelActivity from "./ModelActivity";
import {
  type ModeOption,
  getSelectableModels,
} from "../modelRouting";

interface ModelActivityData {
  model: string;
  profile: string;
  backend: string;
  thinking: boolean;
}

interface ChatHeaderProps {
  models: string[];
  selectedModel: string;
  onSelectedModelChange: (value: string) => void;
  modeSetting: ModeOption;
  onModeSettingChange: (value: ModeOption) => void;
  allowThinking: boolean;
  onToggleThinking: () => void;
  messagesLength: number;
  onNewChat: () => void;
  isGenerating: boolean;
  modelActivity: ModelActivityData | null;
  elapsedSec: number;
}

export default function ChatHeader({
  models,
  selectedModel,
  onSelectedModelChange,
  modeSetting,
  onModeSettingChange,
  allowThinking,
  onToggleThinking,
  messagesLength,
  onNewChat,
  isGenerating,
  modelActivity,
  elapsedSec,
}: ChatHeaderProps) {
  return (
    <header className="sonne-header">
      <div className="brand-wrap">
        <span className="brand-dot" />
        <span className="brand-title">Sonne</span>
      </div>

      {modelActivity && (
        <ModelActivity
          model={modelActivity.model}
          profile={modelActivity.profile}
          backend={modelActivity.backend}
          thinking={modelActivity.thinking}
          isGenerating={isGenerating}
          elapsedSec={elapsedSec}
        />
      )}

      <div className="header-actions">
        <button
          className="ghost-btn"
          onClick={onToggleThinking}
          disabled={isGenerating}
          title="Вмикає глибокі роздуми моделі для складних задач"
        >
          {allowThinking ? "Думати: УВІМК" : "Думати: ВИМК"}
        </button>

        <select
          className="model-select"
          value={modeSetting}
          onChange={(event) =>
            onModeSettingChange(event.target.value as ModeOption)
          }
          disabled={isGenerating}
        >
          <option value="auto">Режим: Авто</option>
          <option value="chat">Режим: Чат</option>
          <option value="code">Режим: Код</option>
          <option value="image">Режим: Зображення</option>
        </select>

        {messagesLength > 0 && (
          <button
            className="ghost-btn"
            onClick={onNewChat}
            disabled={isGenerating}
          >
            Новий чат
          </button>
        )}

        <select
          className="model-select"
          value={selectedModel}
          onChange={(event) => onSelectedModelChange(event.target.value)}
          disabled={isGenerating}
        >
          <option value="auto">Модель: Авто</option>
          {getSelectableModels(models).map((model) => (
            <option key={model} value={model}>
              {model}
            </option>
          ))}
        </select>
      </div>
    </header>
  );
}
