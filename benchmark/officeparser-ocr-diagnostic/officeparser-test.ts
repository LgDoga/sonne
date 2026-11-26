import { OfficeParser } from "officeparser";

const output = document.querySelector("#result")!;
const fileInput = document.querySelector<HTMLInputElement>("#file")!;

fileInput.addEventListener("change", async () => {
  const file = fileInput.files?.[0];
  if (!file) return;

  output.textContent = `Parsing ${file.name}...`;

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

    const content = ast.content as any;

    const inspect = (node: any): any => ({
      type: node?.type,
      text: typeof node?.text === "string" ? node.text.slice(0, 500) : undefined,
      textLength: node?.text?.length ?? 0,
      metadata: node?.metadata,
      children: Array.isArray(node?.children) ? node.children.map(inspect) : undefined,
    });

    const markdownResult = await ast.to("md", {
      includeImages: "ocr-text-only",
    });
    const markdownValue = String((markdownResult as any).value ?? "");
    const markdownWithoutImages = markdownValue.replace(
      new RegExp("data:image/[^;]+;base64,[A-Za-z0-9+/=]+", "g"),
      "[IMAGE_DATA]",
    );

    const report = {
      contentType: typeof content,
      contentKeys: content && typeof content === "object" ? Object.keys(content) : [],
      contentSample: Array.isArray(content)
        ? content.slice(0, 30).map(inspect)
        : inspect(content),
      markdown: {
        resultType: typeof markdownResult,
        resultKeys: Object.keys(markdownResult as any),
        valueLength: markdownValue.length,
        containsOcrText: markdownValue.includes("Склад Ne3"),
        valueSample: markdownWithoutImages.slice(0, 10000),
        messages: (markdownResult as any).messages,
      },
      attachments: (ast.attachments ?? []).map((item: any) => ({
        name: item.name,
        ocrLength: item.ocrText?.length ?? 0,
        ocrSample: item.ocrText?.slice(0, 150),
      })),
    };

    output.textContent = JSON.stringify(report, null, 2);
  } catch (error) {
    output.textContent = error instanceof Error
      ? error.stack ?? error.message
      : String(error);
  }
});
