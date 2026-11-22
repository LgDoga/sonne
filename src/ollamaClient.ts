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
