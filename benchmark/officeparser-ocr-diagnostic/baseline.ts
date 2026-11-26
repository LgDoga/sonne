import { readFile } from "../../sonne/src/fileReader";

const input = document.querySelector<HTMLInputElement>("#files")!;
const button = document.querySelector<HTMLButtonElement>("#run")!;
const status = document.querySelector<HTMLPreElement>("#status")!;

const expected = new Map([
  ["01-digital-text.pdf", 86635],
  ["02-scanned.pdf", 1333691],
  ["03-mixed-content.pdf", 865401],
  ["04-structured.docx", 41141],
  ["05-images.docx", 140480],
  ["06-multisheet.xlsx", 19576],
  ["07-visual-slides.pptx", 135159],
  ["08-semantic.html", 5342],
  ["09-text-and-markdown.md", 2771],
  ["10-data.csv", 17720],
  ["11-book.epub", 27954],
  ["12-nested-archive.zip", 75453],
]);

input.addEventListener("change", () => {
  const files = Array.from(input.files ?? []);
  const lines = files.map((file, i) => {
    const path = file.webkitRelativePath || file.name;
    const expectedSize = expected.get(file.name);
    const valid = expectedSize === file.size;
    return `${i + 1}. ${path}\n   ${file.size} байт — ${
      expectedSize === undefined
        ? "НЕОЧІКУВАНИЙ ФАЙЛ"
        : valid
          ? "РОЗМІР OK"
          : `НЕВІРНИЙ РОЗМІР (очікується ${expectedSize})`
    }`;
  });

  const validCount = files.filter(
    file => expected.get(file.name) === file.size
  ).length;

  status.textContent =
    `Вибрано: ${files.length} файлів\n` +
    `Збігів з еталоном: ${validCount}/12\n\n` +
    lines.join("\n\n");
});

button.addEventListener("click", async () => {
  const files = Array.from(input.files ?? []);

  const valid =
    files.length === 12 &&
    new Set(files.map(file => file.name)).size === 12 &&
    files.every(file => expected.get(file.name) === file.size) &&
    files.every(file => expected.get(file.name) === file.size);

  if (!valid) {
    status.textContent =
      "Benchmark не запущено: вибери саме папку source з усіма 12 еталонними файлами.";
    return;
  }

  button.disabled = true;
  const results = [];

  try {
    for (let i = 0; i < files.length; i++) {
      const file = files[i];
      const started = performance.now();
      status.textContent =
        `Обробка ${i + 1}/12: ${file.webkitRelativePath}`;

      try {
        const parsed = await readFile(file);
        results.push({
          name: file.name,
          relativePath: file.webkitRelativePath,
          type: parsed.type,
          size: file.size,
          lastModified: file.lastModified,
          durationMs: Math.round(performance.now() - started),
          characters: parsed.content.length,
          content: parsed.content,
          error: null,
        });
      } catch (error) {
        results.push({
          name: file.name,
          relativePath: file.webkitRelativePath,
          size: file.size,
          lastModified: file.lastModified,
          durationMs: Math.round(performance.now() - started),
          characters: 0,
          content: "",
          error: error instanceof Error ? error.message : String(error),
        });
      }
    }

    const report = {
      createdAt: new Date().toISOString(),
      extractor: "Sonne readFile / OfficeParser 8.9.0",
      totalFiles: files.length,
      successful: results.filter(x => !x.error).length,
      failed: results.filter(x => x.error).length,
      results,
    };

    const blob = new Blob([JSON.stringify(report, null, 2)], {
      type: "application/json",
    });
    const url = URL.createObjectURL(blob);
    const link = document.createElement("a");
    link.href = url;
    link.download = "sonne-baseline-results.json";
    link.click();
    URL.revokeObjectURL(url);

    status.textContent =
      `Завершено: ${report.successful}/12\n` +
      `Помилок: ${report.failed}\n\n` +
      results.map(x =>
        `${x.name}: ${x.error ?? `${x.characters} символів, ${x.durationMs} мс`}`
      ).join("\n");
  } finally {
    button.disabled = false;
  }
});
