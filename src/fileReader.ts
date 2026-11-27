import { invoke } from "@tauri-apps/api/core";
import { OfficeParser } from "officeparser";
import { createWorker } from "tesseract.js";

import {
  createEmbeddings,
  createVisionDescription,
  OLLAMA_HOST,
  VISION_CONFIG,
  VISION_PROMPT_VERSION,
} from "./ollamaClient";
export type SupportedFileType =
  | "text"
  | "pdf"
  | "document"
  | "spreadsheet"
  | "presentation"
  | "epub"
  | "image";

export type ReadFileResult = {
  name: string;
  type: SupportedFileType;
  size: number;
  content: string;
};

const RESOURCE_ROOT = "/resources";

const PDF_WORKER = `${RESOURCE_ROOT}/pdfjs/pdf.worker.min.mjs`;
const PDF_CMAPS = `${RESOURCE_ROOT}/pdfjs/cmaps/`;
const PDF_DECODERS = `${RESOURCE_ROOT}/pdfjs/wasm/`;

const OCR_WORKER = `${RESOURCE_ROOT}/tesseract/worker.min.js`;
const OCR_CORE = `${RESOURCE_ROOT}/tesseract/core/`;
const OCR_LANG = `${RESOURCE_ROOT}/tesseract/lang/`;

const TEXT_EXTENSIONS = new Set([
  "txt", "md", "markdown", "json", "jsonc", "html", "htm",
  "css", "scss", "sass", "less", "js", "jsx", "mjs", "cjs",
  "ts", "tsx", "py", "java", "c", "cpp", "h", "hpp", "cs",
  "go", "rs", "php", "rb", "swift", "kt", "sql", "xml",
  "yaml", "yml", "toml", "ini", "sh", "bash", "zsh", "env",
  "log", "gitignore", "dockerfile", "vue", "svelte",
  "graphql", "gql", "svg",
]);

const IMAGE_EXTENSIONS = new Set([
  "png", "jpg", "jpeg", "webp", "gif", "bmp",
  "tif", "tiff", "avif", "heic", "heif",
]);

const FILE_TYPES: Readonly<Record<string, SupportedFileType>> = {
  pdf: "pdf",

  docx: "document",
  odt: "document",
  rtf: "document",

  xlsx: "spreadsheet",
  ods: "spreadsheet",
  csv: "spreadsheet",
  tsv: "spreadsheet",

  pptx: "presentation",
  odp: "presentation",

  epub: "epub",
};

function extensionOf(name: string): string {
  const index = name.lastIndexOf(".");
  return index >= 0
    ? name.slice(index + 1).toLowerCase()
    : name.toLowerCase();
}

export function getFileType(file: File): SupportedFileType {
  const extension = extensionOf(file.name);

  if (FILE_TYPES[extension]) return FILE_TYPES[extension];
  if (IMAGE_EXTENSIONS.has(extension)) return "image";
  if (TEXT_EXTENSIONS.has(extension)) return "text";

  throw new Error(
    `Непідтримуваний формат файлу: ${extension || "unknown"}`,
  );
}

async function readTextFile(file: File): Promise<string> {
  return file.text();
}

async function readOfficeFile(file: File): Promise<string> {
  const extension = extensionOf(file.name);
  const bytes = new Uint8Array(await file.arrayBuffer());

  const ast = await OfficeParser.parseOffice(bytes, {
    fileType: (extension === "tsv" ? "csv" : extension) as "pdf" | "docx" | "odt" | "rtf" | "xlsx" | "ods" | "csv" | "pptx" | "odp" | "epub",
    allowRemoteAssets: false,

    pdfWorkerSrc: PDF_WORKER,
    pdfCMapUrl: PDF_CMAPS,
    pdfDecoderUrl: PDF_DECODERS,

    ocr: true,
    extractAttachments: true,
    ocrConfig: {
      language: "eng+ukr",
      workerPath: OCR_WORKER,
      corePath: OCR_CORE,
      langPath: OCR_LANG,
      cacheMethod: "write",
      preserveLayout: true,
    },
  });

  const result = await ast.to("md", { includeImages: "ocr-text-only" });

  if (typeof result.value !== "string") {
    throw new Error(
      `Парсер повернув неочікуваний формат результату для "${file.name}".`,
    );
  }

  return result.value;
}

async function readImageFile(file: File): Promise<string> {
  const worker = await createWorker("eng+ukr", 1, {
    workerPath: OCR_WORKER,
    corePath: OCR_CORE,
    langPath: OCR_LANG,
    cacheMethod: "write",
  });

  try {
    const result = await worker.recognize(file);
    return result.data.text;
  } finally {
    await worker.terminate();
  }
}

export async function readFile(file: File): Promise<ReadFileResult> {
  const type = getFileType(file);
  let content: string;

  switch (type) {
    case "text":
      content = await readTextFile(file);
      break;

    case "image":
      content = await readImageFile(file);
      break;

    case "pdf":
    case "document":
    case "spreadsheet":
    case "presentation":
    case "epub":
      content = await readOfficeFile(file);
      break;
  }

  if (!content.trim()) {
    throw new Error(
      `Не вдалося отримати текст із файлу "${file.name}".`,
    );
  }

  return {
    name: file.name,
    type,
    size: file.size,
    content,
  };
}

export interface StoredAttachmentInput {
  id: string;
  chatId: string;
  fileName: string;
  mimeType: string;
  originalSize: number;
  onProgress?: (message: string) => void;
}

export const EXTRACTOR_ID = "sonne-unified";
export const EXTRACTOR_VERSION = "officeparser-8.9.0_tesseract-7.0.0_assets-v4-on-demand-vision";
const STORED_READ_CHUNK_SIZE = 256 * 1024;
const INDEX_CHUNK_TARGET_TOKENS = 800;
const INDEX_CHUNK_OVERLAP_TOKENS = 120;
const INDEX_CHARS_PER_ESTIMATED_TOKEN = 3;
const INDEX_CHUNK_TARGET_SIZE =
  INDEX_CHUNK_TARGET_TOKENS * INDEX_CHARS_PER_ESTIMATED_TOKEN;
const INDEX_CHUNK_OVERLAP_SIZE =
  INDEX_CHUNK_OVERLAP_TOKENS * INDEX_CHARS_PER_ESTIMATED_TOKEN;
const INDEX_MAX_SECTIONS = 9_000;

type IndexSection = {
  key: string;
  parentKey: string | null;
  sectionType: string;
  title: string | null;
  ordinal: number;
  metadata?: Record<string, unknown>;
};

type IndexChunk = {
  sectionKey: string;
  content: string;
  tokenCount: number;
  startOffset: number;
  endOffset: number;
  chunkType: string;
  metadata?: Record<string, unknown>;
};

type ExtractedAsset = {
  sectionKey: string | null;
  assetType: "image" | "chart";
  mimeType: string | null;
  fileName: string | null;
  ordinal: number;
  data: number[];
  metadata: Record<string, unknown> | null;
};

const MAX_DOCUMENT_ASSETS = 1_000;
const MAX_ASSET_BYTES = 16 * 1024 * 1024;
const MAX_DOCUMENT_ASSET_BYTES = 64 * 1024 * 1024;
const MAX_ASSET_WARNING_MESSAGES = 100;

function isArchiveName(name: string): boolean {
  return /\.(zip|tar|gz|tgz)$/i.test(name);
}

export async function loadStoredAttachment(
  attachment: StoredAttachmentInput,
): Promise<File> {
  if (
    !Number.isSafeInteger(attachment.originalSize) ||
    attachment.originalSize < 0 ||
    attachment.originalSize > 512 * 1024 * 1024
  ) {
    throw new Error(
      `Некоректний або завеликий розмір вкладення "${attachment.fileName}".`,
    );
  }

  const chunks: ArrayBuffer[] = [];
  let offset = 0;

  while (offset < attachment.originalSize) {
    const length = Math.min(
      STORED_READ_CHUNK_SIZE,
      attachment.originalSize - offset,
    );

    const bytes = await invoke<number[]>("read_attachment_chunk", {
      chatId: attachment.chatId,
      referenceId: attachment.id,
      offset,
      length,
    });

    if (!bytes.length || bytes.length > length) {
      throw new Error(
        `Не вдалося прочитати вкладення "${attachment.fileName}" повністю.`,
      );
    }

    const chunk = Uint8Array.from(bytes);
    chunks.push(
      chunk.buffer.slice(
        chunk.byteOffset,
        chunk.byteOffset + chunk.byteLength,
      ) as ArrayBuffer,
    );

    offset += bytes.length;
  }

  if (offset !== attachment.originalSize) {
    throw new Error(
      `Розмір прочитаного файлу "${attachment.fileName}" не збігається з оригіналом.`,
    );
  }

  return new File(chunks, attachment.fileName, {
    type: attachment.mimeType || "application/octet-stream",
  });
}

function normalizeSectionType(value: unknown): string {
  const type = typeof value === "string" ? value.toLowerCase() : "";

  if (/page/.test(type)) return "page";
  if (/sheet|worksheet/.test(type)) return "sheet";
  if (/slide/.test(type)) return "slide";
  if (/chapter|epub/.test(type)) return "chapter";
  if (/table|row|cell|grid/.test(type)) return "table";
  if (/header/.test(type)) return "header";
  if (/footer/.test(type)) return "footer";
  if (/note|comment/.test(type)) return "notes";
  if (/paragraph|text|heading|title|list|item/.test(type)) {
    return "paragraph";
  }

  return "paragraph";
}

function collectAstSections(ast: unknown): IndexSection[] {
  const sections: IndexSection[] = [{
    key: "root",
    parentKey: null,
    sectionType: "root",
    title: "Документ",
    ordinal: 0,
  }];

  const root = ast as Record<string, unknown>;
  const content = Array.isArray(root?.content) ? root.content : [];
  let ordinal = 1;
  let generatedKey = 0;

  const visit = (
    value: unknown,
    parentKey: string,
    depth: number,
  ): void => {
    if (sections.length >= INDEX_MAX_SECTIONS || depth > 32) return;

    if (Array.isArray(value)) {
      for (const item of value) {
        visit(item, parentKey, depth + 1);
        if (sections.length >= INDEX_MAX_SECTIONS) break;
      }
      return;
    }

    if (!value || typeof value !== "object") return;

    const node = value as Record<string, unknown>;
    const rawType = typeof node.type === "string" ? node.type : "paragraph";
    const sectionType = normalizeSectionType(rawType);
    const key = `ast-${generatedKey++}`;

    const rawTitle =
      typeof node.title === "string"
        ? node.title
        : typeof node.text === "string"
          ? node.text
          : "";

    const title = rawTitle.trim().slice(0, 300) || null;

    const metadata: Record<string, unknown> = {
      sourceNodeType: rawType.slice(0, 100),
    };

    if (
      node.metadata &&
      typeof node.metadata === "object" &&
      !Array.isArray(node.metadata)
    ) {
      metadata.sourceMetadata = node.metadata;
    }

    sections.push({
      key,
      parentKey,
      sectionType,
      title,
      ordinal: ordinal++,
      metadata,
    });

    if (node.children !== undefined) {
      visit(node.children, key, depth + 1);
    }
  };

  visit(content, "root", 0);
  return sections;
}

function splitIntoChunks(
  content: string,
  sectionKey = "root",
): IndexChunk[] {
  const normalized = content.replace(/\r\n/g, "\n").trim();
  if (!normalized) return [];

  const chunks: IndexChunk[] = [];
  let start = 0;

  const findBoundary = (from: number, hardEnd: number): number => {
    const minimumEnd = from + Math.floor((hardEnd - from) * 0.65);

    const paragraphBreak = normalized.lastIndexOf("\n\n", hardEnd);
    if (paragraphBreak >= minimumEnd) return paragraphBreak + 2;

    const lineBreak = normalized.lastIndexOf("\n", hardEnd);
    if (lineBreak >= minimumEnd) return lineBreak + 1;

    let sentenceBoundary = -1;
    for (const punctuation of [". ", "! ", "? ", "; "]) {
      sentenceBoundary = Math.max(
        sentenceBoundary,
        normalized.lastIndexOf(punctuation, hardEnd),
      );
    }
    if (sentenceBoundary >= minimumEnd) return sentenceBoundary + 1;

    let whitespace = -1;
    for (let i = hardEnd; i >= minimumEnd; i--) {
      if (/\s/.test(normalized[i])) {
        whitespace = i;
        break;
      }
    }
    return whitespace >= minimumEnd ? whitespace : hardEnd;
  };

  while (start < normalized.length) {
    const hardEnd = Math.min(
      start + INDEX_CHUNK_TARGET_SIZE,
      normalized.length,
    );

    const end =
      hardEnd === normalized.length
        ? normalized.length
        : findBoundary(start, hardEnd);

    let contentStart = start;
    let contentEnd = Math.max(end, start + 1);

    while (contentStart < contentEnd && /\s/.test(normalized[contentStart])) {
      contentStart++;
    }
    while (contentEnd > contentStart && /\s/.test(normalized[contentEnd - 1])) {
      contentEnd--;
    }

    const text = normalized.slice(contentStart, contentEnd);

    if (text) {
      chunks.push({
        sectionKey,
        content: text,
        tokenCount: Math.max(
          1,
          Math.ceil(text.length / INDEX_CHARS_PER_ESTIMATED_TOKEN),
        ),
        startOffset: contentStart,
        endOffset: contentEnd,
        chunkType: "text",
      });
    }

    if (end >= normalized.length) break;

    let nextStart = Math.max(
      start + 1,
      contentEnd - INDEX_CHUNK_OVERLAP_SIZE,
    );

    while (nextStart > start && !/\s/.test(normalized[nextStart - 1])) {
      nextStart--;
    }
    while (nextStart < normalized.length && /\s/.test(normalized[nextStart])) {
      nextStart++;
    }

    if (nextStart <= start || nextStart >= end) {
      nextStart = end;
    }

    start = nextStart;
  }

  return chunks;
}

function getAstMetadata(ast: unknown): Record<string, unknown> {
  const root = ast as Record<string, unknown>;
  const metadata = root?.metadata;

  if (
    metadata &&
    typeof metadata === "object" &&
    !Array.isArray(metadata)
  ) {
    return metadata as Record<string, unknown>;
  }

  return {};
}

function collectAstAssetSectionKeys(ast: unknown): Map<string, string> {
  const result = new Map<string, string>();
  const root = ast as Record<string, unknown>;
  const content = Array.isArray(root?.content) ? root.content : [];

  let generatedKey = 0;
  let visited = 1;

  const visit = (value: unknown, depth: number): void => {
    if (visited >= INDEX_MAX_SECTIONS || depth > 32) return;

    if (Array.isArray(value)) {
      for (const item of value) {
        visit(item, depth + 1);
        if (visited >= INDEX_MAX_SECTIONS) break;
      }
      return;
    }

    if (!value || typeof value !== "object") return;

    const node = value as Record<string, unknown>;
    const key = `ast-${generatedKey++}`;
    visited++;

    const metadata =
      node.metadata &&
      typeof node.metadata === "object" &&
      !Array.isArray(node.metadata)
        ? node.metadata as Record<string, unknown>
        : {};

    const attachmentName =
      typeof metadata.attachmentName === "string"
        ? metadata.attachmentName
        : typeof node.attachmentName === "string"
          ? node.attachmentName
          : null;

    if (attachmentName && !result.has(attachmentName)) {
      result.set(attachmentName, key);
    }

    if (node.children !== undefined) {
      visit(node.children, depth + 1);
    }
  };

  visit(content, 0);
  return result;
}

type OfficeAssetExtraction = {
  assets: ExtractedAsset[];
  warnings: string[];
};

function extractOfficeAssets(ast: unknown): OfficeAssetExtraction {
  const root = ast as Record<string, unknown>;
  const attachments = Array.isArray(root?.attachments)
    ? root.attachments
    : [];

  const sectionKeys = collectAstAssetSectionKeys(ast);
  const assets: ExtractedAsset[] = [];
  const warnings: string[] = [];
  let totalBytes = 0;
  let omittedWarnings = 0;

  const warn = (message: string): void => {
    if (warnings.length < MAX_ASSET_WARNING_MESSAGES - 1) {
      warnings.push(message);
    } else {
      omittedWarnings += 1;
    }
  };

  for (let index = 0; index < attachments.length; index++) {
    const raw = attachments[index];

    if (!raw || typeof raw !== "object") {
      warn(`Вкладення OfficeParser №${index + 1} пропущено: некоректний запис.`);
      continue;
    }

    const attachment = raw as Record<string, unknown>;
    const assetType = attachment.type;

    if (assetType !== "image" && assetType !== "chart") {
      continue;
    }

    if (assets.length >= MAX_DOCUMENT_ASSETS) {
      warn(
        `Вкладення OfficeParser №${index + 1} пропущено: досягнуто ліміт ${MAX_DOCUMENT_ASSETS} assets.`,
      );
      continue;
    }

    if (typeof attachment.data !== "string" || !attachment.data.trim()) {
      warn(
        `Вкладення OfficeParser №${index + 1} пропущено: парсер не повернув дані.`,
      );
      continue;
    }

    let base64 = attachment.data.trim();
    let dataUriMime: string | null = null;

    if (base64.startsWith("data:")) {
      const comma = base64.indexOf(",");
      if (comma < 0 || !base64.slice(0, comma).includes(";base64")) {
        warn(
          `Вкладення OfficeParser №${index + 1} пропущено: некоректний Base64 data URI.`,
        );
        continue;
      }

      const header = base64.slice(5, comma);
      dataUriMime = header.split(";")[0] || null;
      base64 = base64.slice(comma + 1);
    }

    base64 = base64.replace(/\s/g, "");

    if (
      !base64 ||
      base64.length % 4 !== 0 ||
      !/^[A-Za-z0-9+/]*={0,2}$/.test(base64)
    ) {
      warn(
        `Вкладення OfficeParser №${index + 1} пропущено: некоректне Base64-кодування.`,
      );
      continue;
    }

    const estimatedSize =
      (base64.length * 3) / 4 -
      (base64.endsWith("==") ? 2 : base64.endsWith("=") ? 1 : 0);

    if (
      !Number.isSafeInteger(estimatedSize) ||
      estimatedSize <= 0 ||
      estimatedSize > MAX_ASSET_BYTES
    ) {
      warn(
        `Вкладення OfficeParser №${index + 1} пропущено: порожнє або перевищує ліміт ${MAX_ASSET_BYTES / 1024 / 1024} MiB.`,
      );
      continue;
    }

    let binary: string;
    try {
      binary = atob(base64);
    } catch {
      warn(
        `Вкладення OfficeParser №${index + 1} пропущено: не вдалося декодувати Base64.`,
      );
      continue;
    }

    const bytes = new Uint8Array(binary.length);
    for (let byteIndex = 0; byteIndex < binary.length; byteIndex++) {
      bytes[byteIndex] = binary.charCodeAt(byteIndex);
    }

    const name =
      typeof attachment.name === "string" ? attachment.name : null;
    const rawMimeType =
      typeof attachment.mimeType === "string"
        ? attachment.mimeType
        : dataUriMime;
    const mimeType = rawMimeType || null;

    if (name && name.length > 512) {
      warn(`Вкладення OfficeParser №${index + 1} пропущено: назва задовга.`);
      continue;
    }

    if (mimeType !== null && (mimeType.length === 0 || mimeType.length > 128)) {
      warn(`Вкладення OfficeParser №${index + 1} пропущено: некоректний MIME-тип.`);
      continue;
    }

    const metadata: Record<string, unknown> = {};
    if (name) metadata.attachmentName = name;

    if (typeof attachment.ocrText === "string" && attachment.ocrText) {
      metadata.ocrText = attachment.ocrText;
    }

    if (
      typeof attachment.chartData === "object" &&
      attachment.chartData !== null
    ) {
      metadata.chartData = attachment.chartData;
    }

    const astSectionKey = name ? sectionKeys.get(name) ?? null : null;
    if (astSectionKey) metadata.astSectionKey = astSectionKey;

    let metadataJson: string;
    try {
      metadataJson = JSON.stringify(metadata);
    } catch {
      warn(
        `Вкладення OfficeParser №${index + 1} пропущено: не вдалося серіалізувати метадані.`,
      );
      continue;
    }

    if (
      new TextEncoder().encode(metadataJson).byteLength >
      256 * 1024
    ) {
      warn(
        `Вкладення OfficeParser №${index + 1} пропущено: метадані перевищують 256 KiB.`,
      );
      continue;
    }

    const nextTotal = totalBytes + bytes.byteLength;
    if (!Number.isSafeInteger(nextTotal) || nextTotal > MAX_DOCUMENT_ASSET_BYTES) {
      warn(
        `Вкладення OfficeParser №${index + 1} пропущено: було б перевищено сумарний ліміт ${MAX_DOCUMENT_ASSET_BYTES / 1024 / 1024} MiB.`,
      );
      continue;
    }

    assets.push({
      sectionKey: astSectionKey,
      assetType,
      mimeType,
      fileName: name,
      ordinal: index,
      data: Array.from(bytes),
      metadata: Object.keys(metadata).length ? metadata : null,
    });
    totalBytes = nextTotal;
  }

  if (omittedWarnings > 0) {
    warnings.push(
      `Ще ${omittedWarnings} попереджень про вкладення не показано через ліміт діагностики.`,
    );
  }

  return { assets, warnings };
}

async function extractStoredFile(
  file: File,
): Promise<{
  type: SupportedFileType;
  content: string;
  sections: IndexSection[];
  assets: ExtractedAsset[];
  metadata: Record<string, unknown>;
}> {
  const type = getFileType(file);

  if (
    type === "pdf" ||
    type === "document" ||
    type === "spreadsheet" ||
    type === "presentation" ||
    type === "epub"
  ) {
    const extension = extensionOf(file.name);
    const bytes = new Uint8Array(await file.arrayBuffer());

    const ast = await OfficeParser.parseOffice(bytes, {
      fileType: (extension === "tsv" ? "csv" : extension) as "pdf" | "docx" | "odt" | "rtf" | "xlsx" | "ods" | "csv" | "pptx" | "odp" | "epub",
      allowRemoteAssets: false,
      pdfWorkerSrc: PDF_WORKER,
      pdfCMapUrl: PDF_CMAPS,
      pdfDecoderUrl: PDF_DECODERS,
      ocr: true,
      extractAttachments: true,
      ocrConfig: {
        language: "eng+ukr",
        workerPath: OCR_WORKER,
        corePath: OCR_CORE,
        langPath: OCR_LANG,
        cacheMethod: "write",
        preserveLayout: true,
      },
    });

    const result = await ast.to("md", { includeImages: "ocr-text-only" });
    if (typeof result.value !== "string" || !result.value.trim()) {
      throw new Error(
        `Не вдалося отримати текст із файлу "${file.name}".`,
      );
    }

    const sections = collectAstSections(ast);
    const assetExtraction = extractOfficeAssets(ast);
    const metadata = getAstMetadata(ast);
    if (assetExtraction.warnings.length) {
      metadata.assetExtractionWarnings = assetExtraction.warnings;
    }

    return {
      type,
      content: result.value,
      sections,
      assets: assetExtraction.assets,
      metadata,
    };
  }

  if (type === "image") {
    // Keep OCR text even when the original image is too large to embed as an asset.
    const ocrText = await readImageFile(file);
    const assets: ExtractedAsset[] = [];
    const metadata: Record<string, unknown> = {
      fileName: file.name,
      mimeType: file.type,
      extraction: "tesseract-ocr",
    };

    if (file.size > MAX_ASSET_BYTES) {
      metadata.assetExtractionWarnings = [
        `Зображення "${file.name}" не збережено як asset: перевищено ліміт ${MAX_ASSET_BYTES / 1024 / 1024} MiB. OCR-текст збережено.`,
      ];
    } else if (file.size > 0) {
      const bytes = new Uint8Array(await file.arrayBuffer());
      assets.push({
        sectionKey: "root",
        assetType: "image",
        mimeType: file.type || null,
        fileName: file.name,
        ordinal: 0,
        data: Array.from(bytes),
        metadata: ocrText.trim() ? { ocrText: ocrText.trim() } : null,
      });
    } else {
      metadata.assetExtractionWarnings = [
        `Зображення "${file.name}" не збережено як asset: файл порожній.`,
      ];
    }

    return {
      type,
      content: ocrText.trim() || `Зображення: ${file.name}`,
      sections: [{
        key: "root",
        parentKey: null,
        sectionType: "root",
        title: file.name,
        ordinal: 0,
      }],
      assets,
      metadata,
    };
  }

  const result = await readFile(file);
  if (!result.content.trim()) {
    throw new Error(`Файл "${file.name}" не містить доступного тексту.`);
  }

  return {
    type: result.type,
    content: result.content,
    sections: [{
      key: "root",
      parentKey: null,
      sectionType: "root",
      title: file.name,
      ordinal: 0,
    }],
    assets: [],
    metadata: {
      fileName: file.name,
      mimeType: file.type,
      extraction: "plain-text",
    },
  };
}

export async function readStoredAttachment(
  attachment: StoredAttachmentInput,
): Promise<ReadFileResult> {
  const file = await loadStoredAttachment(attachment);
  return readFile(file);
}


function flattenChartData(value: unknown): string {
  const lines: string[] = [];
  let visited = 0;

  const visit = (current: unknown, path: string, depth: number): void => {
    if (visited >= 10000 || depth > 10 || lines.join("\n").length >= 180000) {
      return;
    }

    if (current === null || current === undefined) {
      return;
    }

    if (Array.isArray(current)) {
      current.forEach((item, index) => {
        if (visited < 10000) {
          visit(item, path ? `${path}[${index}]` : `[${index}]`, depth + 1);
        }
      });
      return;
    }

    if (typeof current === "object") {
      for (const [key, item] of Object.entries(current as Record<string, unknown>)) {
        if (visited >= 10000) break;
        visit(item, path ? `${path}.${key}` : key, depth + 1);
      }
      return;
    }

    if (
      typeof current === "string" ||
      typeof current === "number" ||
      typeof current === "boolean"
    ) {
      const text = String(current).slice(0, 180000);
      lines.push(path ? `${path}: ${text}` : text);
      visited += 1;
    }
  };

  visit(value, "", 0);
  return lines.join("\n").slice(0, 180000);
}

const MAX_BACKGROUND_VISION_IMAGES = VISION_CONFIG.backgroundMaxImages;
const MAX_BACKGROUND_VISION_BYTES = VISION_CONFIG.backgroundMaxBytes;
const MAX_ASSET_METADATA_BYTES = 256 * 1024;

async function addBackgroundVisionDescriptions(
  assets: ExtractedAsset[],
  onProgress?: (message: string) => void,
): Promise<ExtractedAsset[]> {
  const result = [...assets];
  let processedImages = 0;
  let processedBytes = 0;
  const candidates = result.filter((asset) => {
    if (asset.assetType !== "image") return false;
    const mimeType = asset.mimeType?.toLowerCase() ?? "";
    const fileName = asset.fileName?.toLowerCase() ?? "";
    return ["image/png", "image/jpeg", "image/webp"].includes(mimeType) ||
      /\.(png|jpe?g|webp)$/.test(fileName);
  });
  const selectedCandidates = candidates.slice(0, MAX_BACKGROUND_VISION_IMAGES);
  const totalCandidates = selectedCandidates.length;

  for (const asset of selectedCandidates) {
    const index = result.indexOf(asset);
    const imageBytes = Uint8Array.from(asset.data);
    if (imageBytes.byteLength === 0) {
      onProgress?.(`Фоновий Vision (Gemma 3): пропущено порожнє зображення ${asset.ordinal + 1}`);
      continue;
    }
    if (processedBytes + imageBytes.byteLength > MAX_BACKGROUND_VISION_BYTES) {
      onProgress?.("Фоновий Vision (Gemma 3): досягнуто ліміт 20 MiB; решту залишено для аналізу за запитом");
      continue;
    }

    processedImages += 1;
    processedBytes += imageBytes.byteLength;
    const visionStartedAt = performance.now();
    onProgress?.(`Фоновий Vision (Gemma 3): аналіз ${processedImages}/${totalCandidates} · зображення ${asset.ordinal + 1}`);

    try {
      const description = await createVisionDescription(OLLAMA_HOST, imageBytes);
      if (!description) {
        onProgress?.(`Фоновий Vision (Gemma 3): порожній опис ${processedImages}/${totalCandidates}`);
        continue;
      }

      const metadata = {
        ...(asset.metadata ?? {}),
        visionDescription: description,
        visionModel: "sonne-gemma:latest",
        visionPromptVersion: VISION_PROMPT_VERSION,
        visionAnalyzedAt: new Date().toISOString(),
      };
      const metadataSize = new TextEncoder().encode(JSON.stringify(metadata)).byteLength;

      if (metadataSize > MAX_ASSET_METADATA_BYTES) {
        console.warn(`[Sonne Vision] Метадані зображення №${asset.ordinal + 1} перевищують ліміт.`);
        onProgress?.(`Фоновий Vision (Gemma 3): опис ${processedImages}/${totalCandidates} не збережено — завеликі метадані`);
        continue;
      }

      result[index] = { ...asset, metadata };
      console.info("[Sonne Vision] Background image", {
        image: asset.fileName || asset.ordinal + 1,
        bytes: imageBytes.byteLength,
        durationMs: Math.round(performance.now() - visionStartedAt),
        result: "cached-for-index",
      });
      onProgress?.(`Фоновий Vision (Gemma 3): завершено ${processedImages}/${totalCandidates}`);
    } catch (error) {
      console.warn(
        `[Sonne Vision] Не вдалося описати зображення №${asset.ordinal + 1}:`,
        error,
      );
      console.info("[Sonne Vision] Background image", {
        image: asset.fileName || asset.ordinal + 1,
        bytes: imageBytes.byteLength,
        durationMs: Math.round(performance.now() - visionStartedAt),
        result: "failed",
      });
      onProgress?.(`Фоновий Vision (Gemma 3): помилка зображення ${asset.ordinal + 1} (${processedImages}/${totalCandidates})`);
    }
  }

  if (candidates.length > selectedCandidates.length) {
    onProgress?.(`Фоновий Vision (Gemma 3): ${processedImages}/${totalCandidates} оброблено; ${candidates.length - selectedCandidates.length} залишено для аналізу за запитом`);
  }

  return result;
}

function createAssetIndexChunks(assets: ExtractedAsset[]): IndexChunk[] {
  const result: IndexChunk[] = [];

  for (const asset of assets) {
    const metadata = asset.metadata ?? {};
    const lines: string[] = [];

    lines.push(asset.assetType === "chart" ? "Embedded chart" : "Embedded image");

    if (asset.fileName) {
      lines.push(`File name: ${asset.fileName}`);
    }

    const altText = metadata.altText;
    if (typeof altText === "string" && altText.trim()) {
      lines.push(`Description: ${altText.trim()}`);
    }

    const visionDescription = metadata.visionDescription;
    if (
      asset.assetType === "image" &&
      typeof visionDescription === "string" &&
      visionDescription.trim()
    ) {
      lines.push(`Vision description: ${visionDescription.trim()}`);
    }
    const ocrText = metadata.ocrText;
    if (typeof ocrText === "string" && ocrText.trim()) {
      lines.push(`OCR text:\n${ocrText.trim()}`);
    }

    if (asset.assetType === "chart" && metadata.chartData !== undefined) {
      const chartText = flattenChartData(metadata.chartData);
      if (chartText) {
        lines.push(`Chart data:\n${chartText}`);
      }
    }

    const searchableText = lines.join("\n").trim();
    if (!searchableText) continue;

    const chunks = splitIntoChunks(searchableText, asset.sectionKey ?? "root");

    chunks.forEach((chunk) => {
      result.push({
        ...chunk,
        chunkType: asset.assetType === "chart" ? "chart_data" : "image_ocr",
        metadata: {
          source: "embedded_asset",
          assetType: asset.assetType,
          assetOrdinal: asset.ordinal,
          fileName: asset.fileName,
        },
      });
    });
  }

  return result;
}

export async function processStoredAttachment(
  attachment: StoredAttachmentInput,
): Promise<{ status: "completed" | "skipped"; documentId?: string }> {
  const onProgress = attachment.onProgress;
  onProgress?.("Витягування тексту, структури та зображень…");

  if (isArchiveName(attachment.fileName)) {
    return { status: "skipped" };
  }

  const statusArgs = {
    chatId: attachment.chatId,
    referenceId: attachment.id,
    extractor: EXTRACTOR_ID,
    extractorVersion: EXTRACTOR_VERSION,
  };

  let documentId: string | null = null;
  let processingToken: string | null = null;
  let stage: "extraction" | "embedding" = "extraction";
  let heartbeatTimer: ReturnType<typeof setInterval> | null = null;
  let heartbeatInFlight = false;
  let claimLost = false;

  const renewClaim = async (): Promise<void> => {
    if (!processingToken) return;

    if (!documentId || claimLost) {
      throw new Error("Обробка документа більше не має активного права запису.");
    }

    const renewed = await invoke<boolean>(
      "renew_attachment_processing_claim",
      { documentId, processingToken },
    );

    if (!renewed) {
      claimLost = true;
      throw new Error(
        "Часове право обробки документа втрачено. Повторіть обробку.",
      );
    }
  };

  const startHeartbeat = (): void => {
    if (!documentId || !processingToken) return;

    heartbeatTimer = setInterval(() => {
      if (heartbeatInFlight || claimLost || !documentId || !processingToken) {
        return;
      }

      heartbeatInFlight = true;

      void invoke<boolean>("renew_attachment_processing_claim", {
        documentId,
        processingToken,
      })
        .then((renewed) => {
          if (!renewed) claimLost = true;
        })
        .catch((error: unknown) => {
          console.warn("Не вдалося оновити lease обробки документа:", error);
        })
        .finally(() => {
          heartbeatInFlight = false;
        });
    }, 30_000);
  };

  try {
    const existing = await invoke<{
      documentId: string | null;
      status: string | null;
    }>("get_attachment_processing_status", statusArgs);

    let documentCompleted =
      existing.status === "completed" && Boolean(existing.documentId);
    documentId = documentCompleted ? existing.documentId : null;

    if (!documentId) {
      const started = await invoke<{
        status: {
          documentId: string | null;
          status: string | null;
        };
        claimToken: string | null;
      }>("begin_attachment_processing", statusArgs);

      documentId = started.status.documentId;
      processingToken = started.claimToken;
      documentCompleted = started.status.status === "completed";

      if (!documentCompleted && !processingToken) {
        if (started.status.status === "processing") {
          throw new Error(
            "Цей документ уже обробляється іншим процесом. Повторіть спробу пізніше.",
          );
        }

        throw new Error("Не вдалося отримати право на обробку документа.");
      }
    }

    if (!documentId) {
      throw new Error("Не вдалося отримати ID документа.");
    }

    startHeartbeat();

    if (!documentCompleted) {
      if (!processingToken) {
        throw new Error("Не вдалося підтвердити право на обробку документа.");
      }

      await renewClaim();

      const file = await loadStoredAttachment(attachment);
      const extracted = await extractStoredFile(file);
      const visionAssets = await addBackgroundVisionDescriptions(
        extracted.assets,
        onProgress,
      );

      await renewClaim();

      onProgress?.("Підготовка пошукових фрагментів…");

      const chunks = [
        ...splitIntoChunks(extracted.content),
        ...createAssetIndexChunks(visionAssets),
      ];

      if (!chunks.length) {
        throw new Error("Не вдалося отримати текстові фрагменти документа.");
      }

      if (chunks.length > 50000) {
        throw new Error("Документ перевищує ліміт у 50000 фрагментів.");
      }

      await renewClaim();

      const saved = await invoke<{
        documentId: string;
        assetCount: number;
        assetWarningCount: number;
        assetWarnings: string[];
        assetWarningsOmitted: number;
      }>(
        "save_extracted_document",
        {
          input: {
            ...statusArgs,
            processingToken,
            detectedFormat: extensionOf(attachment.fileName),
            language: "eng+ukr",
            metadata: {
              ...extracted.metadata,
              fileName: attachment.fileName,
              mimeType: attachment.mimeType,
              parserType: "officeparser",
            },
            sourceSize: attachment.originalSize,
            sections: extracted.sections,
            chunks,
            assets: visionAssets,
          },
        },
      );

      if (saved.assetWarningCount > 0) {
        onProgress?.(
          `Збережено assets: ${saved.assetCount}; попереджень: ${saved.assetWarningCount}.`,
        );
        for (const warning of saved.assetWarnings.slice(0, 5)) {
          onProgress?.(`Asset: ${warning}`);
        }
        if (saved.assetWarningCount > saved.assetWarnings.length) {
          onProgress?.(
            `Ще ${saved.assetWarningCount - saved.assetWarnings.length} попереджень не показано.`,
          );
        }
      }

      documentId = saved.documentId;
    }

    if (!documentId) {
      throw new Error("Не вдалося отримати ID документа.");
    }

    const claimArgs = processingToken ? { processingToken } : {};

    stage = "embedding";
    onProgress?.("Створення локального пошукового індексу…");

    while (true) {
      await renewClaim();

      const batch = await invoke<
        Array<{ chunkId: string; content: string }>
      >("get_document_embedding_batch", {
        documentId,
        limit: 32,
        ...claimArgs,
      });

      if (batch.length === 0) break;

      const vectors = await createEmbeddings(
        OLLAMA_HOST,
        batch.map((chunk) => chunk.content),
      );

      if (
        vectors.length !== batch.length ||
        vectors.some(
          (vector) =>
            vector.length !== 768 ||
            vector.some((value) => !Number.isFinite(value)),
        )
      ) {
        throw new Error("Некоректна відповідь embedding-моделі.");
      }

      await renewClaim();

      await invoke("save_document_embedding_batch", {
        documentId,
        ...claimArgs,
        embeddings: batch.map((chunk, index) => ({
          chunkId: chunk.chunkId,
          vector: vectors[index],
        })),
      });
    }

    await renewClaim();

    await invoke("finalize_document_embedding_index", {
      documentId,
      ...claimArgs,
    });

    onProgress?.("Обробку завершено.");
    return { status: "completed", documentId };
  } catch (error) {
    const message =
      error instanceof Error ? error.message : String(error);

    if (processingToken) {
      try {
        await invoke("mark_attachment_processing_failed", {
          ...statusArgs,
          processingToken,
          errorCode:
            stage === "embedding" ? "embedding_failed" : "extraction_failed",
          errorMessage: message,
        });
      } catch {
        // Preserve the original processing error.
      }
    }

    throw error;
  } finally {
    if (heartbeatTimer !== null) {
      clearInterval(heartbeatTimer);
    }
  }
}
