import { useEffect, useMemo, useRef, useState } from "react";
import "prismjs";
const Prism = (globalThis as unknown as {
  Prism?: {
    languages?: Record<string, unknown>;
    highlight?: (code: string, grammar: unknown, lang: string) => string;
  };
}).Prism;
import "./App.css";

type ModeOption = "auto" | "chat" | "code" | "image";
type ResolvedMode = "chat" | "code" | "image";

type ChatMessage = {
  role: "user" | "assistant";
  content: string;
  thinking?: string;
  modeUsed?: ResolvedMode;
  modelUsed?: string;
  timestamp: string;
  durationSec?: string;
};

type MessagePart =
  | { type: "text"; value: string }
  | { type: "code"; lang: string; code: string };

const OLLAMA_URL = "http://127.0.0.1:11434";

const CHAT_SYSTEM_PROMPT =
  "Ти — швидкий і живий локальний асистент Sonne. Відповідай українською. " +
  "На привітання, балачку та прості запитання відповідай коротко (1-3 речення) і по суті, " +
  "БЕЗ коду, верстки та зайвих вступів, якщо користувач прямо не просить код або дизайн.";

const CODE_SYSTEM_PROMPT =
  "Ти — провідний UI/UX дизайнер і креативний frontend-інженер рівня Awwwards у додатку Sonne. Пиши виключно українською у живому, драйвовому та впевненому стилі (без сухої офіційності й без англійських чернеток чи внутрішніх монологів). " +
  "Коли просять зверстати секцію, сторінку або компонент, ЗАБОРОНЕНО робити примітивні шаблони з 30 рядків, стандартними кнопками або текстом-рибою типу 'Привітання!' чи 'Тут буде опис'. " +
  "Якщо робиш Hero або лендинг — вигадуй цілісний концепт-бренд (наприклад, студія, космічна місія, AI-платформа, кібер-архітектура) і будуй насичену багатошарову сцену: " +
  "навігація з логотипом та скляною кнопкою, акцентний бейдж, масивний градієнтний заголовок через clamp(), виразний підзаголовок і дві кнопки дій, " +
  "складний центральний візуальний арт-об'єкт на чистому CSS (сфери, кільця, орбіти, світлові промені, сітки або 3D-шари) плюс 2 плаваючі glassmorphism-картки з живими метриками та нижній статус-бар. " +
  "Використовуй CSS-змінні (:root), багаторівневі radial-gradient, backdrop-filter: blur(), плавні @keyframes (світіння, левітація, орбітальний рух, перелив градієнта), hover-мікроанімації, повний адаптив (@media) та @media (prefers-reduced-motion: reduce). " +
  "Формат відповіді:\n" +
  "1) Живий вступ (2-3 речення): яку саме концепцію ти вигадав і що це один самодостатній HTML-файл без бібліотек.\n" +
  "2) Один повний, детально прописаний блок ```html ... ```.\n" +
  "3) Розділ 'Що тут прикольного' списком фіч і анімацій, коротка порада щодо запуску на весь екран і смілива ідея для наступної версії.";

function formatNow(): string {
  return new Date().toLocaleTimeString("uk-UA", {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}

function detectIntent(prompt: string, modeSetting: ModeOption): ResolvedMode {
  if (modeSetting !== "auto") return modeSetting;

  const q = prompt.toLowerCase().trim();

  const imageKeywords = [
    "намалюй",
    "згенеруй картинку",
    "згенеруй зображення",
    "зроби картинку",
    "зроби зображення",
    "банер",
    "іконк",
    "арт",
    "постер",
    "обкладинк",
    "draw ",
    "generate image",
    "create image",
    "image of",
    "banner",
  ];
  if (imageKeywords.some((kw) => q.includes(kw))) {
    return "image";
  }

  const codeKeywords = [
    "код",
    "верстк",
    "html",
    "css",
    "javascript",
    "typescript",
    "react",
    "python",
    "java",
    "c++",
    "компонент",
    "сторінк",
    "лендинг",
    "сайт",
    "кнопк",
    "картк",
    "карточк",
    "анімац",
    "функц",
    "скрипт",
    "баг",
    "помилк",
    "sql",
    "api",
    "code",
    "layout",
    "ui",
    "ux",
    "інтерфейс",
    "форма",
    "таблиц",
  ];
  if (codeKeywords.some((kw) => q.includes(kw))) {
    return "code";
  }

  return "chat";
}

function pickModelForMode(
  resolvedMode: ResolvedMode,
  availableModels: string[],
  selectedModel: string
): string {
  if (selectedModel !== "auto" && availableModels.includes(selectedModel)) {
    return selectedModel;
  }

  if (resolvedMode === "chat") {
    const lightModel = availableModels.find((m) => m.toLowerCase().includes("gemma3"));
    if (lightModel) return lightModel;
  }

  if (resolvedMode === "code") {
    const proModel = availableModels.find((m) => m.toLowerCase().includes("pro-ai"));
    if (proModel) return proModel;
    const coderModel = availableModels.find((m) => m.toLowerCase().includes("coder"));
    if (coderModel) return coderModel;
  }

  const fallbackPro = availableModels.find((m) => m.toLowerCase().includes("pro-ai"));
  return fallbackPro || availableModels[0] || "pro-ai:latest";
}

function stripSelfMonologue(text: string): string {
  const lines = text.split("\n");
  const filtered = lines.filter((line) => {
    const t = line.trim();
    if (!t) return true;
    const isMonologueLine =
      /^(okay,|first,|wait,|but sometimes|the user (asked|said|might)|let me |i'll |i will |check online:|make sure there are)/i.test(
        t
      );
    return !isMonologueLine;
  });
  return filtered.join("\n").replace(/\n{3,}/g, "\n\n").trim();
}

function splitThinking(raw: string): { thinking: string; answer: string } {
  const thinkOpen = raw.indexOf("<think>");
  if (thinkOpen === -1) {
    return { thinking: "", answer: stripSelfMonologue(raw) };
  }
  const thinkClose = raw.indexOf("</think>");
  if (thinkClose === -1) {
    return {
      thinking: raw.slice(thinkOpen + 7).trim(),
      answer: "",
    };
  }
  return {
    thinking: raw.slice(thinkOpen + 7, thinkClose).trim(),
    answer: stripSelfMonologue(raw.slice(thinkClose + 8).trim()),
  };
}

function extractRawHtmlIfUnfenced(text: string): MessagePart[] | null {
  const trimmed = text.trim();
  const doctypeIdx = trimmed.toLowerCase().indexOf("<!doctype html");
  const htmlIdx = trimmed.toLowerCase().indexOf("<html");
  const startIdx =
    doctypeIdx !== -1 && htmlIdx !== -1
      ? Math.min(doctypeIdx, htmlIdx)
      : Math.max(doctypeIdx, htmlIdx);

  if (startIdx === -1) return null;

  const closeHtmlIdx = trimmed.toLowerCase().lastIndexOf("</html>");
  if (closeHtmlIdx === -1) return null;

  const endIdx = closeHtmlIdx + "</html>".length;
  const before = trimmed.slice(0, startIdx).trim();
  const htmlCode = trimmed.slice(startIdx, endIdx).trim();
  const after = trimmed.slice(endIdx).trim();

  const parts: MessagePart[] = [];
  if (before) parts.push({ type: "text", value: before });
  parts.push({ type: "code", lang: "html", code: htmlCode });
  if (after) parts.push({ type: "text", value: after });
  return parts;
}

function parseContent(raw: string): MessagePart[] {
  const parts: MessagePart[] = [];
  const regex = /```([a-zA-Z0-9_-]*)\n([\s\S]*?)```/g;
  let lastIndex = 0;
  let match: RegExpExecArray | null;

  while ((match = regex.exec(raw)) !== null) {
    if (match.index > lastIndex) {
      parts.push({ type: "text", value: raw.slice(lastIndex, match.index) });
    }
    parts.push({
      type: "code",
      lang: (match[1] || "code").toLowerCase(),
      code: match[2].replace(/\n$/, ""),
    });
    lastIndex = regex.lastIndex;
  }

  if (lastIndex < raw.length) {
    parts.push({ type: "text", value: raw.slice(lastIndex) });
  }

  if (parts.length > 0) {
    const hasCode = parts.some((p) => p.type === "code");
    if (!hasCode) {
      const unfencedHtml = extractRawHtmlIfUnfenced(raw);
      if (unfencedHtml) return unfencedHtml;
    }
    return parts;
  }

  return [{ type: "text", value: raw }];
}

function highlightCode(code: string, lang: string): string {
  const normalized =
    lang === "html" || lang === "xml" || lang === "svg"
      ? "markup"
      : lang === "js"
        ? "javascript"
        : lang === "ts"
          ? "typescript"
          : lang === "py"
            ? "python"
            : lang;

  const grammar = Prism?.languages?.[normalized];
  if (grammar && typeof Prism?.highlight === "function") {
    return Prism.highlight(code, grammar, normalized);
  }
  return code
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}

function CodeBlock({
  lang,
  code,
  onPreview,
}: {
  lang: string;
  code: string;
  onPreview: (code: string, lang: string) => void;
}) {
  const [copied, setCopied] = useState(false);
  const highlighted = useMemo(() => highlightCode(code, lang), [code, lang]);
  const canPreview = ["html", "markup", "svg", "css"].includes(lang);

  async function handleCopy() {
    try {
      await navigator.clipboard.writeText(code);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      // ignore
    }
  }

  return (
    <div className="code-block">
      <div className="code-header">
        <span className="code-lang">{lang}</span>
        <div className="code-btns">
          {canPreview && (
            <button className="mini-btn" onClick={() => onPreview(code, lang)}>
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

function ThinkingBlock({
  thinking,
  isActive,
}: {
  thinking: string;
  isActive: boolean;
}) {
  const [open, setOpen] = useState(false);

  return (
    <div className="thinking-box">
      <button className="thinking-toggle" onClick={() => setOpen((v) => !v)}>
        <span>{isActive ? "Хід думок моделі (думає...)" : "Хід думок моделі"}</span>
        <span>{open ? "Згорнути ▲" : "Розгорнути ▼"}</span>
      </button>
      {open && <div className="thinking-content">{thinking}</div>}
    </div>
  );
}

export function App() {
  const [models, setModels] = useState<string[]>([]);
  const [selectedModel, setSelectedModel] = useState<string>("auto");
  const [modeSetting, setModeSetting] = useState<ModeOption>("auto");
  const [allowThinking, setAllowThinking] = useState<boolean>(false);
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [input, setInput] = useState<string>("");
  const [isGenerating, setIsGenerating] = useState<boolean>(false);
  const [elapsedSec, setElapsedSec] = useState<number>(0);
  const [previewHtml, setPreviewHtml] = useState<string | null>(null);
  const bottomRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    void Promise.all([
      import("prismjs/components/prism-markup"),
      import("prismjs/components/prism-css"),
      import("prismjs/components/prism-javascript"),
      import("prismjs/components/prism-typescript"),
      import("prismjs/components/prism-java"),
      import("prismjs/components/prism-c"),
      import("prismjs/components/prism-python"),
    ]).catch(() => {});
  }, []);

  useEffect(() => {
    async function loadModels() {
      try {
        const res = await fetch(`${OLLAMA_URL}/api/tags`);
        if (!res.ok) return;
        const data = await res.json();
        const names: string[] = (data.models || [])
          .map((m: { name: string }) => m.name)
          .filter((name: string) => !name.includes("embed"));
        if (names.length > 0) {
          setModels(names);
        }
      } catch {
        // Ollama offline
      }
    }
    loadModels();
  }, []);

  useEffect(() => {
    if (!isGenerating) return;
    const start = Date.now();
    setElapsedSec(0);
    const timer = setInterval(() => {
      setElapsedSec(Number(((Date.now() - start) / 1000).toFixed(1)));
    }, 100);
    return () => clearInterval(timer);
  }, [isGenerating]);

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages, isGenerating]);

  function handleOpenPreview(code: string, lang: string) {
    if (lang === "css") {
      setPreviewHtml(
        `<!DOCTYPE html><html><head><style>${code}</style></head><body><div style="padding:24px;font-family:sans-serif">CSS підключено</div></body></html>`
      );
      return;
    }
    setPreviewHtml(code);
  }

  async function handleSend() {
    const prompt = input.trim();
    if (!prompt || isGenerating) return;

    const userTime = formatNow();
    const startTime = Date.now();
    const resolvedMode = detectIntent(prompt, modeSetting);
    const modelToUse = pickModelForMode(resolvedMode, models, selectedModel);

    setMessages((prev) => [
      ...prev,
      { role: "user", content: prompt, timestamp: userTime },
      {
        role: "assistant",
        content: "",
        thinking: "",
        modeUsed: resolvedMode,
        modelUsed: modelToUse,
        timestamp: userTime,
      },
    ]);

    setInput("");
    setIsGenerating(true);

    if (resolvedMode === "image") {
      const totalSec = ((Date.now() - startTime) / 1000).toFixed(1);
      setMessages((prev) => {
        const updated = [...prev];
        updated[updated.length - 1] = {
          role: "assistant",
          modeUsed: "image",
          modelUsed: "Image Generator",
          timestamp: formatNow(),
          durationSec: totalSec,
          content:
            `Запит розпізнано як генерацію зображення: \`${prompt}\`.\n\n` +
            `Генератор зображень підключимо наступним кроком.`,
        };
        return updated;
      });
      setIsGenerating(false);
      return;
    }

    const shouldThink = resolvedMode === "code" ? allowThinking : false;

    const historyMessages = messages.map((m) => ({
      role: m.role,
      content: m.content,
    }));

    const systemPrompt =
      resolvedMode === "chat" ? CHAT_SYSTEM_PROMPT : CODE_SYSTEM_PROMPT;

    const apiMessages = [
      { role: "system" as const, content: systemPrompt },
      ...historyMessages,
      { role: "user" as const, content: prompt },
    ];

    try {
      const response = await fetch(`${OLLAMA_URL}/api/chat`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          model: modelToUse,
          stream: true,
          think: shouldThink,
          messages: apiMessages,
        }),
      });

      if (!response.ok || !response.body) {
        throw new Error(`HTTP ${response.status}`);
      }

      const reader = response.body.getReader();
      const decoder = new TextDecoder();
      let buffer = "";
      let rawFull = "";

      while (true) {
        const { done, value } = await reader.read();
        if (done) break;

        buffer += decoder.decode(value, { stream: true });
        const lines = buffer.split("\n");
        buffer = lines.pop() || "";

        for (const line of lines) {
          if (!line.trim()) continue;
          const parsed = JSON.parse(line);

          const thinkingChunk = parsed.message?.thinking || "";
          const contentChunk = parsed.message?.content || "";

          if (thinkingChunk) {
            rawFull = rawFull.includes("<think>")
              ? rawFull + thinkingChunk
              : `<think>${thinkingChunk}`;
          }
          if (contentChunk) {
            if (rawFull.includes("<think>") && !rawFull.includes("</think>")) {
              rawFull += `</think>${contentChunk}`;
            } else {
              rawFull += contentChunk;
            }
          }

          if (thinkingChunk || contentChunk) {
            const { thinking, answer } = splitThinking(rawFull);

            setMessages((prev) => {
              const updated = [...prev];
              updated[updated.length - 1] = {
                role: "assistant",
                content: answer,
                thinking,
                modeUsed: resolvedMode,
                modelUsed: modelToUse,
                timestamp: formatNow(),
              };
              return updated;
            });
          }
        }
      }

      const totalSec = ((Date.now() - startTime) / 1000).toFixed(1);
      setMessages((prev) => {
        const updated = [...prev];
        updated[updated.length - 1] = {
          ...updated[updated.length - 1],
          timestamp: formatNow(),
          durationSec: totalSec,
        };
        return updated;
      });
    } catch (err) {
      const errMsg = err instanceof Error ? err.message : "Помилка з'єднання з Ollama";
      setMessages((prev) => {
        const updated = [...prev];
        updated[updated.length - 1] = {
          role: "assistant",
          modeUsed: resolvedMode,
          modelUsed: modelToUse,
          content: `Не вдалося отримати відповідь від Ollama (${OLLAMA_URL}): ${errMsg}. Перевір, чи запущена Ollama.`,
          timestamp: formatNow(),
        };
        return updated;
      });
    } finally {
      setIsGenerating(false);
    }
  }

  return (
    <div className={`sonne-shell ${isGenerating ? "is-generating" : ""}`}>
      <div className="ambient-layer" />

      <header className="sonne-header">
        <div className="brand-wrap">
          <span className="brand-dot" />
          <span className="brand-title">Sonne</span>
        </div>

        <div className="header-actions">
          <button
            className="ghost-btn"
            onClick={() => setAllowThinking((v) => !v)}
            disabled={isGenerating}
            title="Вмикає глибокі роздуми моделі для складних задач"
          >
            {allowThinking ? "Думати: УВІМК" : "Думати: ВИМК"}
          </button>

          <select
            className="model-select"
            value={modeSetting}
            onChange={(e) => setModeSetting(e.target.value as ModeOption)}
            disabled={isGenerating}
          >
            <option value="auto">Режим: Авто</option>
            <option value="chat">Режим: Чат</option>
            <option value="code">Режим: Код</option>
            <option value="image">Режим: Зображення</option>
          </select>

          {messages.length > 0 && (
            <button
              className="ghost-btn"
              onClick={() => {
                setMessages([]);
                setPreviewHtml(null);
              }}
              disabled={isGenerating}
            >
              Новий чат
            </button>
          )}

          <select
            className="model-select"
            value={selectedModel}
            onChange={(e) => setSelectedModel(e.target.value)}
            disabled={isGenerating}
          >
            <option value="auto">Модель: Авто</option>
            {models.map((m) => (
              <option key={m} value={m}>
                {m}
              </option>
            ))}
          </select>
        </div>
      </header>

      <div className="sonne-workspace">
        <div className="chat-pane">
          <main className="sonne-messages">
            {messages.length === 0 ? (
              <div className="empty-state">
                <h2>Привіт, це Sonne</h2>
                <p>
                  Авторежим сам кидає прості запити на легку модель, а верстку та код — на сильну.
                </p>
              </div>
            ) : (
              messages.map((m, idx) => {
                const isLastAssistant = isGenerating && idx === messages.length - 1;
                const assistantLabel =
                  isLastAssistant && !m.content ? "Sonne думає" : "Sonne";

                return (
                  <div key={idx} className={`msg ${m.role}`}>
                    <div className="msg-meta">
                      <span className="msg-role-label">
                        {m.role === "user" ? "Ти" : assistantLabel}
                      </span>
                      <span className="msg-time">
                        {m.timestamp}
                        {m.durationSec ? ` · ${m.durationSec} с` : ""}
                      </span>
                    </div>

                    {m.role === "user" ? (
                      <div>{m.content}</div>
                    ) : (
                      <>
                        {m.thinking && !m.content && (
                          <ThinkingBlock thinking={m.thinking} isActive={isLastAssistant} />
                        )}

                        {m.content &&
                          parseContent(m.content).map((part, pIdx) =>
                            part.type === "code" ? (
                              <CodeBlock
                                key={pIdx}
                                lang={part.lang}
                                code={part.code}
                                onPreview={handleOpenPreview}
                              />
                            ) : (
                              <div key={pIdx} className="prose-text">
                                {part.value}
                              </div>
                            )
                          )}
                      </>
                    )}
                  </div>
                );
              })
            )}
            <div ref={bottomRef} />
          </main>

          <div className="sonne-input-wrap">
            <div className="input-box">
              <textarea
                placeholder="Напиши запит... (Enter — надіслати, Shift+Enter — новий рядок)"
                value={input}
                onChange={(e) => setInput(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" && !e.shiftKey) {
                    e.preventDefault();
                    handleSend();
                  }
                }}
                disabled={isGenerating}
              />
              <button
                className="send-btn"
                onClick={handleSend}
                disabled={isGenerating || !input.trim()}
              >
                {isGenerating ? `${elapsedSec.toFixed(0)} с...` : "Надіслати"}
              </button>
            </div>
          </div>
        </div>

        {previewHtml !== null && (
          <aside className="preview-pane">
            <div className="preview-header">
              <span>Live Preview</span>
              <button className="mini-btn" onClick={() => setPreviewHtml(null)}>
                Закрити ✕
              </button>
            </div>
            <iframe
              className="preview-frame"
              srcDoc={previewHtml}
              sandbox="allow-scripts"
              title="Live Preview"
            />
          </aside>
        )}
      </div>
    </div>
  );
}

export default App;
