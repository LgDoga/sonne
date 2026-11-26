import { OfficeParser } from "officeparser";

const output = document.querySelector("#result")!;
const fileInput = document.querySelector<HTMLInputElement>("#file")!;

const log = (value: unknown) => {
  output.textContent += "\n" + (
    typeof value === "string" ? value : JSON.stringify(value, null, 2)
  );
};

fileInput.addEventListener("change", async () => {
  const file = fileInput.files?.[0];
  if (!file) return;

  output.textContent = `Parsing ${file.name} (${file.size} bytes)...`;

  const originalWarn = console.warn;
  const originalError = console.error;

  console.warn = (...args) => {
    log(["WARNING", ...args.map(String)]);
    originalWarn(...args);
  };

  console.error = (...args) => {
    log(["ERROR", ...args.map(String)]);
    originalError(...args);
  };

  try {
    const ast = await OfficeParser.parseOffice(await file.arrayBuffer(), {
      fileType: "pdf",
      allowRemoteAssets: false,
      pdfWorkerSrc: "/resources/pdfjs/pdf.worker.min.mjs",
      pdfCMapUrl: "/resources/pdfjs/cmaps/",
      pdfDecoderUrl: "/resources/pdfjs/wasm/",
      ocr: true,
      extractAttachments: true,
      ocrConfig: {
        language: "eng+ukr",
        workerPath: "/resources/tesseract/worker.min.js",
        corePath: "/resources/tesseract/core/",
        langPath: "/resources/tesseract/lang/",
        cacheMethod: "write",
        preserveLayout: true,
      },
    });

    log("WARNINGS:");
    log(ast.warnings);

    log("ATTACHMENTS:");
    log((ast.attachments ?? []).map((item: any) => ({
      name: item.name,
      mimeType: item.mimeType,
      keys: Object.keys(item),
      ocrText: item.ocrText,
      ocrLength: item.ocrText?.length ?? 0,
    })));

    log("CONTENT TYPE:");
    log(typeof ast.content);

    const markdown = await ast.to("md", { includeImages: "image+ocr-text" });

    log("MARKDOWN RESULT:");
    log({
      type: typeof markdown,
      keys: Object.keys(markdown ?? {}),
      value: (markdown as any)?.value?.slice?.(0, 12000),
      messages: (markdown as any)?.messages,
    });
  } catch (error) {
    log("PARSER FAILURE:");
    log(error instanceof Error ? error.stack : String(error));
  } finally {
    console.warn = originalWarn;
    console.error = originalError;
  }
});
