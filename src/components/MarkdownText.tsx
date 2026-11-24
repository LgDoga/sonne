import type { ReactNode } from "react";

type MessagePart =
  | { type: "text"; value: string }
  | { type: "code"; lang: string; code: string };


function extractRawHtmlIfUnfenced(text: string): MessagePart[] | null {
  const trimmed = text.trim();

  const lowerText = trimmed.toLowerCase();
  const doctypeIndex = lowerText.indexOf("<!doctype html");
  const htmlIndex = lowerText.indexOf("<html");

  const startIndex =
    doctypeIndex !== -1 && htmlIndex !== -1
      ? Math.min(doctypeIndex, htmlIndex)
      : Math.max(doctypeIndex, htmlIndex);

  if (startIndex === -1) return null;

  const closeHtmlIndex = lowerText.lastIndexOf("</html>");

  if (closeHtmlIndex === -1) return null;

  const endIndex = closeHtmlIndex + "</html>".length;

  const before = trimmed.slice(0, startIndex).trim();
  const htmlCode = trimmed.slice(startIndex, endIndex).trim();
  const after = trimmed.slice(endIndex).trim();

  const parts: MessagePart[] = [];

  if (before) {
    parts.push({ type: "text", value: before });
  }

  parts.push({
    type: "code",
    lang: "html",
    code: htmlCode,
  });

  if (after) {
    parts.push({ type: "text", value: after });
  }

  return parts;
}

export function parseContent(raw: string): MessagePart[] {
  const parts: MessagePart[] = [];
  const regex = /```([a-zA-Z0-9_-]*)\n([\s\S]*?)```/g;

  let lastIndex = 0;
  let match: RegExpExecArray | null;

  while ((match = regex.exec(raw)) !== null) {
    if (match.index > lastIndex) {
      parts.push({
        type: "text",
        value: raw.slice(lastIndex, match.index),
      });
    }

    parts.push({
      type: "code",
      lang: detectCodeLanguage(match[1], match[2]),
      code: match[2].replace(/\n$/, ""),
    });

    lastIndex = regex.lastIndex;
  }

  const remaining = raw.slice(lastIndex);
  const incompleteFence = /```([a-zA-Z0-9_-]*)\n([\s\S]*)$/.exec(remaining);

  if (incompleteFence) {
    const fenceIndex = lastIndex + incompleteFence.index;

    if (fenceIndex > lastIndex) {
      parts.push({
        type: "text",
        value: raw.slice(lastIndex, fenceIndex),
      });
    }

    parts.push({
      type: "code",
      lang: detectCodeLanguage(incompleteFence[1], incompleteFence[2]),
      code: incompleteFence[2],
    });

    lastIndex = raw.length;
  }

  if (lastIndex < raw.length) {
    parts.push({
      type: "text",
      value: raw.slice(lastIndex),
    });
  }

  if (parts.length > 0) {
    const hasCode = parts.some((part) => part.type === "code");

    if (!hasCode) {
      const unfencedHtml = extractRawHtmlIfUnfenced(raw);

      if (unfencedHtml) return unfencedHtml;
    }

    return parts;
  }

  return [{ type: "text", value: raw }];
}

function detectCodeLanguage(declaredLanguage: string, code: string): string {
  const language = declaredLanguage.trim().toLowerCase();

  const aliases: Record<string, string> = {
    csharp: "csharp",
    cs: "csharp",
    html: "html",
    js: "javascript",
    jsx: "javascript",
    py: "python",
    shell: "bash",
    sh: "bash",
    ts: "typescript",
    tsx: "typescript",
    xml: "xml",
    yml: "yaml",
  };

  if (language) {
    return aliases[language] || language;
  }

  const trimmedCode = code.trim();

  if (
    (trimmedCode.startsWith("{") || trimmedCode.startsWith("[")) &&
    (() => {
      try {
        JSON.parse(trimmedCode);
        return true;
      } catch {
        return false;
      }
    })()
  ) {
    return "json";
  }

  return "code";
}

function normalizeEscapedMarkdownDelimiters(value: string): string {
  return value
    .replace(/\\\*\\\*\\\*([^*\n]+?)\\\*\\\*\\\*/g, "***$1***")
    .replace(/\\\*\\\*([^*\n]+?)\\\*\\*/g, "**$1**")
    .replace(/\\__([^_\n]+?)\\__/g, "__$1__")
    .replace(/\\\*([^*\n]+?)\\\*/g, "*$1*")
    .replace(/\\_([^_\n]+?)\\_/g, "_$1_");
}

function renderFormattedText(value: string, keyPrefix: string): ReactNode[] {
  const normalized = normalizeEscapedMarkdownDelimiters(value);
  const tokens = normalized.split(
    /(\*\*\*[^*\n]+?\*\*\*|___[^_\n]+?___|\*\*[^*\n]+?\*\*|__[^_\n]+?__|\*[^*\n]+?\*|(?<!\w)_[^_\n]+?_(?!\w))/g,
  );

  return tokens.map((token, index) => {
    const isAsteriskBoldItalic =
      token.startsWith("***") && token.endsWith("***");
    const isUnderscoreBoldItalic =
      token.startsWith("___") && token.endsWith("___");
    const isAsteriskBold = token.startsWith("**") && token.endsWith("**");
    const isUnderscoreBold = token.startsWith("__") && token.endsWith("__");
    const isAsteriskItalic = token.startsWith("*") && token.endsWith("*");
    const isUnderscoreItalic = token.startsWith("_") && token.endsWith("_");

    if (isAsteriskBoldItalic || isUnderscoreBoldItalic) {
      return (
        <strong key={`${keyPrefix}-bold-italic-${index}`}>
          <em>{token.slice(3, -3)}</em>
        </strong>
      );
    }

    if (isAsteriskBold || isUnderscoreBold) {
      return (
        <strong key={`${keyPrefix}-bold-${index}`}>{token.slice(2, -2)}</strong>
      );
    }

    if (isAsteriskItalic || isUnderscoreItalic) {
      return <em key={`${keyPrefix}-italic-${index}`}>{token.slice(1, -1)}</em>;
    }

    return token;
  });
}

function renderInlineMarkdown(value: string, keyPrefix: string): ReactNode[] {
  const nodes: ReactNode[] = [];
  const codePattern = /`([^`\n]*)`|\\`([^`\n]*)\\`/g;
  let lastIndex = 0;
  let match: RegExpExecArray | null;
  let tokenIndex = 0;

  while ((match = codePattern.exec(value)) !== null) {
    if (match.index > lastIndex) {
      nodes.push(
        ...renderFormattedText(
          value.slice(lastIndex, match.index),
          `${keyPrefix}-text-${tokenIndex}`,
        ),
      );
    }

    nodes.push(
      <code
        className="markdown-inline-code"
        key={`${keyPrefix}-code-${tokenIndex}`}
      >
        {match[1] ?? match[2] ?? ""}
      </code>,
    );

    lastIndex = codePattern.lastIndex;
    tokenIndex += 1;
  }

  if (lastIndex < value.length) {
    nodes.push(
      ...renderFormattedText(
        value.slice(lastIndex),
        `${keyPrefix}-text-${tokenIndex}`,
      ),
    );
  }

  return nodes;
}

function renderTextLines(lines: string[], keyPrefix: string): ReactNode[] {
  return lines.flatMap((line, index) => [
    ...renderInlineMarkdown(line, `${keyPrefix}-${index}`),
    ...(index < lines.length - 1
      ? [<br key={`${keyPrefix}-break-${index}`} />]
      : []),
  ]);
}

function isTableDivider(line: string): boolean {
  return /^\s*\|?\s*:?-{3,}:?\s*(?:\|\s*:?-{3,}:?\s*)+\|?\s*$/.test(line);
}

function splitTableRow(line: string): string[] {
  return line
    .trim()
    .replace(/^\|/, "")
    .replace(/\|$/, "")
    .split("|")
    .map((cell) => cell.trim());
}

function isTableStart(lines: string[], index: number): boolean {
  return lines[index].includes("|") && isTableDivider(lines[index + 1] || "");
}

function normalizeEscapedMarkdownBlockSyntax(line: string): string {
  return line
    .replace(/^(\s*)\\(>\s?)/, "$1$2")
    .replace(/^(\s*)\\((?:---|\*\*\*|___)\s*)$/, "$1$2");
}

function isHorizontalRule(line: string): boolean {
  return /^\s*(?:-{3,}|\*{3,}|_{3,})\s*$/.test(line);
}

export function MarkdownText({ content }: { content: string }) {
  const lines = content
    .replace(/\r\n?/g, "\n")
    .split("\n")
    .map(normalizeEscapedMarkdownBlockSyntax);
  const blocks: ReactNode[] = [];
  let lineIndex = 0;

  while (lineIndex < lines.length) {
    const line = lines[lineIndex];

    if (!line.trim()) {
      lineIndex += 1;
      continue;
    }

    const quoteMatch = /^\s*>\s?(.*)$/.exec(line);

    if (quoteMatch) {
      const quoteLines: string[] = [];

      while (lineIndex < lines.length) {
        const currentQuote = /^\s*>\s?(.*)$/.exec(lines[lineIndex]);

        if (!currentQuote) break;

        quoteLines.push(currentQuote[1]);
        lineIndex += 1;
      }

      blocks.push(
        <blockquote className="markdown-quote" key={`quote-${lineIndex}`}>
          {renderTextLines(quoteLines, `quote-${lineIndex}`)}
        </blockquote>,
      );
      continue;
    }

    if (isHorizontalRule(line)) {
      blocks.push(<hr className="markdown-rule" key={`rule-${lineIndex}`} />);
      lineIndex += 1;
      continue;
    }

    const headingMatch = /^(#{1,6})\s+(.+?)\s*#*\s*$/.exec(line);

    if (headingMatch) {
      const level = headingMatch[1].length;
      blocks.push(
        <div
          aria-level={level}
          className={`markdown-heading markdown-heading-${level}`}
          key={`heading-${lineIndex}`}
          role="heading"
        >
          {renderInlineMarkdown(headingMatch[2], `heading-${lineIndex}`)}
        </div>,
      );
      lineIndex += 1;
      continue;
    }

    if (isTableStart(lines, lineIndex)) {
      const headerCells = splitTableRow(line);
      const rows: string[][] = [];
      lineIndex += 2;

      while (
        lineIndex < lines.length &&
        lines[lineIndex].trim().includes("|")
      ) {
        rows.push(splitTableRow(lines[lineIndex]));
        lineIndex += 1;
      }

      blocks.push(
        <div className="markdown-table-wrap" key={`table-${lineIndex}`}>
          <table className="markdown-table">
            <thead>
              <tr>
                {headerCells.map((cell, cellIndex) => (
                  <th key={`header-${cellIndex}`}>
                    {renderInlineMarkdown(cell, `table-header-${cellIndex}`)}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {rows.map((row, rowIndex) => (
                <tr key={`row-${rowIndex}`}>
                  {headerCells.map((_, cellIndex) => (
                    <td key={`cell-${rowIndex}-${cellIndex}`}>
                      {renderInlineMarkdown(
                        row[cellIndex] || "",
                        `table-${rowIndex}-${cellIndex}`,
                      )}
                    </td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </div>,
      );
      continue;
    }

    const unorderedMatch = /^\s*[-*+]\s+(.+)$/.exec(line);
    const orderedMatch = /^\s*(\d+)\.\s+(.+)$/.exec(line);

    if (unorderedMatch || orderedMatch) {
      const isOrdered = Boolean(orderedMatch);
      const items: string[] = [];
      const start = orderedMatch ? Number(orderedMatch[1]) : undefined;

      while (lineIndex < lines.length) {
        const itemMatch = isOrdered
          ? /^\s*\d+\.\s+(.+)$/.exec(lines[lineIndex])
          : /^\s*[-*+]\s+(.+)$/.exec(lines[lineIndex]);

        if (!itemMatch) break;

        items.push(itemMatch[1]);
        lineIndex += 1;
      }

      const listItems = items.map((item, itemIndex) => (
        <li key={`item-${itemIndex}`}>
          {renderInlineMarkdown(item, `list-${lineIndex}-${itemIndex}`)}
        </li>
      ));

      blocks.push(
        isOrdered ? (
          <ol
            className="markdown-list"
            key={`ordered-${lineIndex}`}
            start={start}
          >
            {listItems}
          </ol>
        ) : (
          <ul className="markdown-list" key={`unordered-${lineIndex}`}>
            {listItems}
          </ul>
        ),
      );
      continue;
    }

    const paragraphLines: string[] = [];

    while (lineIndex < lines.length) {
      const currentLine = lines[lineIndex];

      if (
        !currentLine.trim() ||
        /^(#{1,6})\s+/.test(currentLine) ||
        /^\s*(?:[-*+]|\d+\.)\s+/.test(currentLine) ||
        isTableStart(lines, lineIndex)
      ) {
        break;
      }

      paragraphLines.push(currentLine);
      lineIndex += 1;
    }

    blocks.push(
      <p className="markdown-paragraph" key={`paragraph-${lineIndex}`}>
        {renderTextLines(paragraphLines, `paragraph-${lineIndex}`)}
      </p>,
    );
  }

  return <div className="prose-text markdown-text">{blocks}</div>;
}

// ============================================================
// CODE HIGHLIGHTING
// ============================================================
