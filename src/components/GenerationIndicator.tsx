interface GenerationIndicatorProps {
  label?: string;
}

export default function GenerationIndicator({
  label = "Sonne думає",
}: GenerationIndicatorProps) {
  return (
    <div className="generation-indicator" role="status" aria-live="polite">
      <span className="generation-label">{label}</span>
      <span className="generation-dots" aria-hidden="true">
        <span />
        <span />
        <span />
      </span>
    </div>
  );
}
