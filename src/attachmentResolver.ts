export type AttachmentCandidate = {
  id: string;
  chatId: string;
  messageId: string | null;
  attachmentVersionId: string;
  versionNumber: number;
  fileName: string;
  extension: string;
  mimeType: string;
  originalSize: number;
  purpose: string;
  position: number;
};

export type HistoricalMessageLike = {
  dbId?: string;
  role: string;
  attachments?: readonly AttachmentCandidate[];
};

export type HistoricalAttachmentReference = {
  attachment: AttachmentCandidate;
  messageId: string;
  messageOrdinal: number;
};

export type HistoricalAttachmentResolution = {
  status: "none" | "resolved" | "ambiguous" | "not-found";
  references: HistoricalAttachmentReference[];
  candidates: HistoricalAttachmentReference[];
  reason: string | null;
  omittedCount: number;
};

const MAX_REFERENCED_ATTACHMENTS = 8;

function normalize(value: string): string {
  return value
    .normalize("NFKC")
    .toLocaleLowerCase("uk-UA")
    .replace(/[’'`]/gu, "")
    .replace(/[^\p{L}\p{N}]+/gu, " ")
    .trim();
}

function fileExtension(file: AttachmentCandidate): string {
  const extension = file.extension.trim().replace(/^\./u, "").toLocaleLowerCase("uk-UA");
  if (extension) return extension;
  const match = file.fileName.match(/\.([^.]+)$/u);
  return match?.[1]?.toLocaleLowerCase("uk-UA") ?? "";
}

function isArchive(file: AttachmentCandidate): boolean {
  return /^(zip|7z|rar|tar|gz|tgz|bz2|tbz2?|xz|txz|zst|zstd|lz4|lzma|lz)$/u.test(fileExtension(file));
}

function matchesFileType(prompt: string, file: AttachmentCandidate): boolean {
  const extension = fileExtension(file);
  const checks: Array<{ pattern: RegExp; extensions: string[] }> = [
    { pattern: /(?:\bpdf\b|пдф)/iu, extensions: ["pdf"] },
    { pattern: /(?:\bdocx?\b|ворд|word)/iu, extensions: ["doc", "docx", "rtf", "odt"] },
    { pattern: /(?:\bxlsx?\b|ексель|excel|таблиц)/iu, extensions: ["xls", "xlsx", "ods", "csv"] },
    { pattern: /(?:\bpptx?\b|презентац|powerpoint)/iu, extensions: ["ppt", "pptx", "odp"] },
    { pattern: /(?:\btxt\b|текстовий файл|текстовий документ)/iu, extensions: ["txt", "md", "markdown", "log"] },
    { pattern: /(?:зображенн|картинк|фото|скріншот|screenshot|\bimage\b|\bphoto\b)/iu, extensions: ["png", "jpg", "jpeg", "webp", "gif", "bmp", "tif", "tiff"] },
  ];
  const requested = checks.filter((check) => check.pattern.test(prompt));
  if (requested.length === 0) return true;
  return requested.some((check) =>
    check.extensions.includes(extension) ||
    (check.extensions.includes("png") && file.mimeType.toLocaleLowerCase("en-US").startsWith("image/")),
  );
}

function filenameMentioned(prompt: string, fileName: string): boolean {
  const normalizedPrompt = normalize(prompt);
  const normalizedName = normalize(fileName);
  if (!normalizedName) return false;
  if (normalizedPrompt.includes(normalizedName)) return true;
  const stem = normalize(fileName.replace(/\.[^.]+$/u, ""));
  return stem.length >= 7 && normalizedPrompt.includes(stem);
}

function userReferenceIntent(prompt: string): boolean {
  const previousCue = /(?:попередн\p{L}*|раніш\p{L}*|минул\p{L}*\s+(?:повідомлен\p{L}*|раз\p{L}*)|вище|останн\p{L}*|перш\p{L}*|друг\p{L}*|previous\p{L}*|earlier|prior|above|last\s+(?:file|document|attachment)|first\s+(?:file|document|attachment))/iu.test(prompt);
  const fileNoun = /(?:файл\p{L}*|документ\p{L}*|вкладенн\p{L}*|\bpdf\b|пдф|\bdocx?\b|\bxlsx?\b|\bpptx?\b|зображенн\p{L}*|картинк\p{L}*|скріншот\p{L}*|\bfile\b|\bdocument\b|\battachment\b|\bimage\b|\bscreenshot\b)/iu.test(prompt);
  const sentEarlier = /(?:надсилав\p{L}*|надсилал\p{L}*|прикріпив\p{L}*|додав\p{L}*|завантажив\p{L}*|sent\s+(?:earlier|before)|attached\s+(?:earlier|before))/iu.test(prompt);
  const referredPronoun = /(?:тим|того|той|нею|ним|його|її|that\s+one|the\s+one)/iu.test(prompt);
  return previousCue && (fileNoun || sentEarlier || referredPronoun);
}

function requestsMultipleFileTypes(prompt: string): boolean {
  const typePatterns = [
    /(?:\bpdf\b|пдф)/iu,
    /(?:\bdocx?\b|ворд|word)/iu,
    /(?:\bxlsx?\b|ексель|excel|таблиц)/iu,
    /(?:\bpptx?\b|презентац|powerpoint)/iu,
    /(?:\btxt\b|текстовий файл|текстовий документ)/iu,
    /(?:зображенн|картинк|фото|скріншот|screenshot|\bimage\b|\bphoto\b)/iu,
  ];
  return typePatterns.filter((pattern) => pattern.test(prompt)).length > 1;
}

function isFirstRequest(prompt: string): boolean {
  return /(?:перш\p{L}*\s*(?:файл\p{L}*|документ\p{L}*|вкладенн\p{L}*|pdf|повідомленн\p{L}*)|first\s+(?:file|document|attachment|pdf|message))/iu.test(prompt);
}

function isSecondRequest(prompt: string): boolean {
  return /(?:друг\p{L}*\s*(?:файл\p{L}*|документ\p{L}*|вкладенн\p{L}*|pdf|повідомленн\p{L}*)|second\s+(?:file|document|attachment|pdf|message))/iu.test(prompt);
}

function isAllRequest(prompt: string): boolean {
  return /(?:усі|всі|кожн\p{L}*|all|every|обидва|обидві|both)\s*(?:(?:попередн\p{L}*|earlier|previous)\s*)?(?:файл\p{L}*|документ\p{L}*|вкладенн\p{L}*|attachments?|pdf(?:-файл\p{L}*)?|docx?|xlsx?|pptx?)/iu.test(prompt);
}

function emptyResolution(): HistoricalAttachmentResolution {
  return { status: "none", references: [], candidates: [], reason: null, omittedCount: 0 };
}

/**
 * Resolve only explicit references to files attached to earlier user messages.
 * Composer references, assistant messages, malformed ownership pairs and
 * attachments from other chats are never candidates.
 */
export function resolveConversationAttachmentReferences(input: {
  prompt: string;
  chatId: string;
  history: readonly HistoricalMessageLike[];
  maxResults?: number;
}): HistoricalAttachmentResolution {
  const prompt = input.prompt.trim();
  if (!prompt || !input.chatId) return emptyResolution();

  const candidates: HistoricalAttachmentReference[] = [];
  let messageOrdinal = 0;

  for (const message of input.history) {
    if (message.role !== "user") continue;
    messageOrdinal += 1;
    if (!message.dbId) continue;

    const attachments = [...(message.attachments ?? [])]
      .filter((attachment) =>
        attachment.chatId === input.chatId &&
        attachment.messageId === message.dbId &&
        attachment.purpose === "message" &&
        Boolean(attachment.id) &&
        Boolean(attachment.attachmentVersionId) &&
        Number.isInteger(attachment.versionNumber) &&
        attachment.versionNumber > 0 &&
        !isArchive(attachment),
      )
      .sort((left, right) => left.position - right.position);

    for (const attachment of attachments) {
      candidates.push({ attachment, messageId: message.dbId, messageOrdinal });
    }
  }

  if (candidates.length === 0) {
    return userReferenceIntent(prompt)
      ? { status: "not-found", references: [], candidates: [], reason: "У цьому чаті немає доступних попередніх вкладень, що відповідають запиту.", omittedCount: 0 }
      : emptyResolution();
  }

  const named = candidates.filter((candidate) => filenameMentioned(prompt, candidate.attachment.fileName));
  const hasReferenceIntent = userReferenceIntent(prompt);
  if (named.length === 0 && !hasReferenceIntent) return emptyResolution();

  let pool = named.length > 0
    ? named
    : candidates.filter((candidate) => matchesFileType(prompt, candidate.attachment));

  if (pool.length === 0) {
    return {
      status: "not-found",
      references: [],
      candidates: [],
      reason: "У цьому чаті не знайдено попереднього вкладення потрібного типу або з указаною назвою.",
      omittedCount: 0,
    };
  }

  const latestMessageOrdinal = pool[pool.length - 1].messageOrdinal;
  const latestMessageCandidates = pool.filter((candidate) => candidate.messageOrdinal === latestMessageOrdinal);
  let selected: HistoricalAttachmentReference[];

  if (isAllRequest(prompt)) {
    selected = pool;
  } else if (requestsMultipleFileTypes(prompt) && pool.length > 1) {
    return {
      status: "ambiguous",
      references: [],
      candidates: pool,
      reason: "Запит згадує кілька типів файлів, але не уточнює, чи всі вони мають бути взяті з попередніх повідомлень. Уточни, які саме історичні вкладення потрібно використати.",
      omittedCount: 0,
    };
  } else if (isFirstRequest(prompt)) {
    selected = [pool[0]];
  } else if (isSecondRequest(prompt)) {
    selected = pool.length >= 2 ? [pool[1]] : [];
  } else if (named.length > 0) {
    // When a filename was reused in the chat, the most recent prior upload is
    // the least surprising interpretation of "the file named X".
    selected = [named[named.length - 1]];
  } else if (latestMessageCandidates.length === 1) {
    selected = latestMessageCandidates;
  } else {
    return {
      status: "ambiguous",
      references: [],
      candidates: latestMessageCandidates,
      reason: "У попередньому повідомленні є кілька вкладень, і запит не визначає, яке саме потрібно використати.",
      omittedCount: 0,
    };
  }

  if (selected.length === 0) {
    return {
      status: "not-found",
      references: [],
      candidates: [],
      reason: "У чаті немає вкладення з таким порядковим номером.",
      omittedCount: 0,
    };
  }

  const maxResults = Math.max(1, Math.min(input.maxResults ?? MAX_REFERENCED_ATTACHMENTS, MAX_REFERENCED_ATTACHMENTS));
  const omittedCount = Math.max(0, selected.length - maxResults);
  selected = selected.slice(0, maxResults);

  return {
    status: "resolved",
    references: selected,
    candidates: [],
    reason: null,
    omittedCount,
  };
}

/** Detect complete comparison/review requests that require full-document coverage. */
export function isFullAttachmentComparisonRequest(prompt: string): boolean {
  const explicitComparison = /(?:порівня\p{L}*|зістав\p{L}*|compare)/iu.test(prompt);
  const reviewIntent = /(?:перевір\p{L}*|оцін\p{L}*|відповідн\p{L}*|review|verify|assess|check\s+against)/iu.test(prompt);
  const completeScope = /(?:кожн\p{L}*|усі|всі|повн\p{L}*|детальн\p{L}*|цілков\p{L}*|вимог\p{L}*|пункт\p{L}*|all|every|complete|entire|full|requirements?|criteria)/iu.test(prompt);
  return explicitComparison || (reviewIntent && completeScope);
}
