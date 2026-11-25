import {
  Ollama,
  type AbortableAsyncIterator,
  type ChatRequest,
  type ChatResponse,
  type Fetch,
} from "ollama/browser";

export type OllamaChatStream = AbortableAsyncIterator<ChatResponse>;

export type ActiveOllamaChatStream = {
  stream: OllamaChatStream;
  abort: () => void;
  dispose: () => void;
};

function createAbortableFetch(
  signal: AbortSignal,
  registerCleanup: (cleanup: () => void) => void,
): Fetch {
  return async (input, init = {}) => {
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

export async function createChatStream(
  host: string,
  request: ChatRequest & { stream: true },
  signal: AbortSignal,
): Promise<ActiveOllamaChatStream> {
  const cleanups: Array<() => void> = [];
  const dispose = () => {
    while (cleanups.length > 0) {
      cleanups.pop()?.();
    }
  };
  const client = new Ollama({
    host,
    fetch: createAbortableFetch(signal, (cleanup) => cleanups.push(cleanup)),
  });

  try {
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

export async function createVisionDescription(
  host: string,
  image: Uint8Array,
): Promise<string> {
  const client = new Ollama({ host });
  const response = await client.chat({
    model: "sonne-gemma:latest",
    stream: false,
    messages: [{
      role: "user",
      content:
        "Опиши це зображення для пошуку в документах. Визнач основні об'єкти, " +
        "схеми, графіки, взаємозв'язки та важливі деталі. Якщо є текст, " +
        "передай лише той, який можна впевнено прочитати. Не вигадуй відсутніх " +
        "даних і не роби припущень. Пиши українською, стисло та інформативно.",
      images: [image],
    }],
    options: { temperature: 0.2, num_predict: 256 },
  });
  return response.message.content.trim();
}

export const OLLAMA_HOST = "http://127.0.0.1:11434";

export async function createEmbeddings(
  host: string,
  input: string[],
): Promise<number[][]> {
  if (input.length === 0) return [];

  const client = new Ollama({ host });
  const response = await client.embed({
    model: "nomic-embed-text:latest",
    input,
    truncate: false,
  });

  return response.embeddings;
}

