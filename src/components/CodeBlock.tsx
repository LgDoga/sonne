import { useMemo, useState } from "react";

interface CodeBlockProps {
  lang: string;
  code: string;
  onPreview: (code: string, lang: string) => void;
  highlightCode: (code: string, lang: string) => string;
}

export default function CodeBlock({
  lang,
  code,
  onPreview,
  highlightCode,
}: CodeBlockProps) {
  const [copied, setCopied] = useState(false);

  const highlighted = useMemo(
    () => highlightCode(code, lang),
    [code, lang, highlightCode],
  );

  const canPreview = ["html", "markup", "svg", "css"].includes(lang);

  async function handleCopy() {
    try {
      await navigator.clipboard.writeText(code);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      // Clipboard access may be unavailable.
    }
  }

  return (
    <div className="code-block">
      <div className="code-header">
        <span className="code-lang">{lang}</span>

        <div className="code-btns">
          {canPreview && (
            <button
              className="mini-btn"
              onClick={() => onPreview(code, lang)}
            >
              Прев'ю
            </button>
          )}

          <button className="mini-btn" onClick={handleCopy}>
            {copied ? "Скопійовано ✓" : "Копіювати"}
          </button>
        </div>
      </div>

      <pre>
        <code dangerouslySetInnerHTML={{ __html: highlighted }} />
      </pre>
    </div>
  );
}
