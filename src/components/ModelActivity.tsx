interface ModelActivityProps {
  model: string;
  profile: string;
  backend: string;
  thinking: boolean;
  isGenerating: boolean;
  elapsedSec: number;
}

export default function ModelActivity({
  model,
  profile,
  backend,
  thinking,
  isGenerating,
  elapsedSec,
}: ModelActivityProps) {
  const profileLabel =
    profile === "pro-ai"
      ? "Pro AI"
      : profile === "gemma"
        ? "Gemma"
        : profile;

  const backendLabel = thinking
    ? "Thinking"
    : backend === "instruct"
      ? "Instruct"
      : backend;

  return (
    <div className={`model-activity ${isGenerating ? "is-active" : ""}`}>
      <span className="activity-indicator" />
      <div className="activity-info">
        <span className="activity-model">{profileLabel}</span>
        <span className="activity-details">
          {backendLabel} · {model}
        </span>
      </div>
      <span className="activity-status">
        {isGenerating
          ? `Генерація · ${elapsedSec.toFixed(1)} с`
          : "Остання активна модель"}
      </span>
    </div>
  );
}
