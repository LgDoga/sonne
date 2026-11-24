interface LivePreviewProps {
  html: string;
  onClose: () => void;
}

export default function LivePreview({ html, onClose }: LivePreviewProps) {
  return (
    <aside className="preview-pane">
      <div className="preview-header">
        <span>Live Preview</span>
        <button className="mini-btn" onClick={onClose}>
          Закрити ✕
        </button>
      </div>

      <iframe
        className="preview-frame"
        srcDoc={html}
        sandbox="allow-scripts"
        title="Live Preview"
      />
    </aside>
  );
}
