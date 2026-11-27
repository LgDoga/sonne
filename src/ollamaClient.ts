import {
  Ollama,
  type AbortableAsyncIterator,
  type ChatRequest,
  type ChatResponse,
  type Fetch,
} from "ollama/browser";
import { getOllamaModelsToUnload } from "./ollamaLifecycle";

export type OllamaChatStream = AbortableAsyncIterator<ChatResponse>;

export type ActiveOllamaChatStream = {
  stream: OllamaChatStream;
  abort: () => void;
  dispose: () => void;
};

// All local Ollama calls share one FIFO lease. This prevents Vision, embeddings,
// and a large response model from competing for unified memory at the same time.
let ollamaOperationTail: Promise<void> = Promise.resolve();
let lastOllamaModel: string | null = null;

async function acquireOllamaLease(signal?: AbortSignal): Promise<() => void> {
  const previous = ollamaOperationTail;
  let releaseCurrent!: () => void;
  const current = new Promise<void>((resolve) => {
    releaseCurrent = resolve;
  });
  ollamaOperationTail = previous.then(() => current);

  if (signal?.aborted) {
    void previous.then(() => releaseCurrent());
    throw new DOMException("Ollama operation cancelled", "AbortError");
  }

  if (signal) {
    let abortListener: (() => void) | undefined;
    const aborted = new Promise<"aborted">((resolve) => {
      abortListener = () => resolve("aborted");
      signal.addEventListener("abort", abortListener, { once: true });
    });
    const outcome = await Promise.race([
      previous.then(() => "ready" as const),
      aborted,
    ]);
    if (abortListener) signal.removeEventListener("abort", abortListener);
    if (outcome === "aborted" || signal.aborted) {
      void previous.then(() => releaseCurrent());
      throw new DOMException("Ollama operation cancelled", "AbortError");
    }
  } else {
    await previous;
  }

  let released = false;
  return () => {
    if (released) return;
    released = true;
    releaseCurrent();
  };
}

function createAbortableFetch(
  signal: AbortSignal,
  registerCleanup: (cleanup: () => void) => void,
): Fetch {
  return async (input, init: RequestInit = {}) => {
    const controller = new AbortController();
    const sdkSignal = init.signal;
    let cleaned = false;
    const cleanup = () => {
      if (cleaned) return;

      cleaned = true;
      signal.removeEventListener("abort", abort);
      sdkSignal?.removeEventListener("abort", abort);
    };
    const abort = () => {
      cleanup();
      controller.abort();
    };

    signal.addEventListener("abort", abort, { once: true });
    sdkSignal?.addEventListener("abort", abort, { once: true });
    registerCleanup(cleanup);

    if (signal.aborted || sdkSignal?.aborted) {
      abort();
    }

    try {
      return await fetch(input, { ...init, signal: controller.signal });
    } catch (error) {
      cleanup();
      throw error;
    }
  };
}

async function requestOllamaModelUnload(host: string, model: string): Promise<void> {
  const response = await fetch(`${host}/api/generate`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ model, prompt: "", stream: false, keep_alive: 0 }),
  });
  if (!response.ok) {
    throw new Error(`Не вдалося звільнити модель ${model}: HTTP ${response.status}`);
  }
  await response.json().catch(() => undefined);
}

async function unloadPreviousOllamaModel(host: string, nextModel: string): Promise<void> {
  if (!lastOllamaModel || lastOllamaModel === nextModel) return;
  const previousModel = lastOllamaModel;
  try {
    await requestOllamaModelUnload(host, previousModel);
    lastOllamaModel = null;
  } catch (error) {
    console.warn(`[Sonne Ollama] Не вдалося звільнити попередню модель ${previousModel}:`, error);
  }
}

export async function createChatStream(
  host: string,
  request: ChatRequest & { stream: true },
  signal: AbortSignal,
  unloadModelsBefore: string[] = [],
): Promise<ActiveOllamaChatStream> {
  const releaseLease = await acquireOllamaLease(signal);
  const cleanups: Array<() => void> = [];
  let disposed = false;
  const dispose = () => {
    if (disposed) return;
    disposed = true;
    while (cleanups.length > 0) {
      cleanups.pop()?.();
    }
    releaseLease();
  };
  const client = new Ollama({
    host,
    fetch: createAbortableFetch(signal, (cleanup) => cleanups.push(cleanup)),
  });

  try {
    const requestedModel = request.model;
    const modelsToUnload = getOllamaModelsToUnload(
      lastOllamaModel,
      requestedModel,
      unloadModelsBefore,
    );
    for (const model of modelsToUnload) {
      try {
        await requestOllamaModelUnload(host, model);
        if (lastOllamaModel === model) lastOllamaModel = null;
      } catch (error) {
        console.warn(`[Sonne Ollama] Не вдалося звільнити ${model} перед запитом:`, error);
      }
    }
    lastOllamaModel = requestedModel;
    const stream = await client.chat(request);

    return {
      stream,
      abort: () => {
        stream.abort();
        dispose();
      },
      dispose,
    };
  } catch (error) {
    dispose();
    throw error;
  }
}

export const VISION_PROMPT_VERSION = 2;

export const VISION_CONFIG = {
  backgroundMaxImages: 4,
  backgroundMaxBytes: 20 * 1024 * 1024,
  maxAssetBytes: 16 * 1024 * 1024,
  descriptionMaxTokens: 192,
} as const;

export async function createVisionDescription(
  host: string,
  image: Uint8Array,
  signal?: AbortSignal,
): Promise<string> {
  const releaseLease = await acquireOllamaLease(signal);
  const cleanups: Array<() => void> = [];
  let disposed = false;
  const dispose = () => {
    if (disposed) return;
    disposed = true;
    while (cleanups.length > 0) cleanups.pop()?.();
    releaseLease();
  };
  const client = new Ollama({
    host,
    ...(signal
      ? { fetch: createAbortableFetch(signal, (cleanup) => cleanups.push(cleanup)) }
      : {}),
  });

  try {
    await unloadPreviousOllamaModel(host, "sonne-gemma:latest");
    lastOllamaModel = "sonne-gemma:latest";
    const response = await client.chat({
      model: "sonne-gemma:latest",
      stream: false,
      keep_alive: "5m",
      messages: [{
        role: "user",
        content:
          "Опиши лише те, що достовірно видно на зображенні. Не вигадуй даних, " +
          "не роби припущень про причини, наслідки чи призначення зображення і " +
          "не прив'язуй його до розділів документа. Для графіків і таблиць " +
          "передай назви осей, одиниці, легенду та числові значення лише якщо " +
          "їх можна чітко прочитати; нерозбірливе познач як невідоме. Для схем " +
          "опиши тільки видимі підписи та зв'язки. Відповідай українською стисло " +
          "у форматі: Тип зображення; Видимий текст; Структура й дані; " +
          "Невизначеність. Не домислюй відсутні деталі.",
        images: [image],
      }],
      options: { temperature: 0, num_predict: VISION_CONFIG.descriptionMaxTokens },
    });
    return response.message.content.trim();
  } finally {
    dispose();
  }
}

export async function unloadOllamaModel(
  host: string,
  model: string,
): Promise<void> {
  const releaseLease = await acquireOllamaLease();
  try {
    await requestOllamaModelUnload(host, model);
    if (lastOllamaModel === model) lastOllamaModel = null;
  } finally {
    releaseLease();
  }
}

export const OLLAMA_HOST = "http://127.0.0.1:11434";

export async function createEmbeddings(
  host: string,
  input: string[],
  signal?: AbortSignal,
): Promise<number[][]> {
  if (input.length === 0) return [];

  const releaseLease = await acquireOllamaLease(signal);
  const cleanups: Array<() => void> = [];
  let disposed = false;
  const dispose = () => {
    if (disposed) return;
    disposed = true;
    while (cleanups.length > 0) cleanups.pop()?.();
    releaseLease();
  };
  const client = new Ollama({
    host,
    ...(signal
      ? { fetch: createAbortableFetch(signal, (cleanup) => cleanups.push(cleanup)) }
      : {}),
  });

  try {
    await unloadPreviousOllamaModel(host, "nomic-embed-text:latest");
    lastOllamaModel = "nomic-embed-text:latest";
    const response = await client.embed({
      model: "nomic-embed-text:latest",
      input,
      truncate: false,
    });
    return response.embeddings;
  } finally {
    dispose();
  }
}

