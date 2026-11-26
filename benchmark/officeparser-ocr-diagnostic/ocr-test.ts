import { createWorker } from "tesseract.js";

const output = document.querySelector("#result")!;
const log = (message: string) => {
  output.textContent += "\n" + message;
  console.log(message);
};

async function run() {
  output.textContent = "Creating local OCR worker...";
  let worker;

  try {
    worker = await createWorker("eng+ukr", 1, {
      workerPath: "/resources/tesseract/worker.min.js",
      corePath: "/resources/tesseract/core/",
      langPath: "/resources/tesseract/lang/",
      cacheMethod: "none",
      logger: (event) => {
        if (event.status) log(`${event.status}: ${Math.round((event.progress || 0) * 100)}%`);
      },
    });

    log("Worker created. Testing recognition...");

    const canvas = document.createElement("canvas");
    canvas.width = 1200;
    canvas.height = 180;

    const ctx = canvas.getContext("2d")!;
    ctx.fillStyle = "white";
    ctx.fillRect(0, 0, canvas.width, canvas.height);
    ctx.fillStyle = "black";
    ctx.font = "36px Arial";
    ctx.fillText("Sonne OCR test — Український текст", 20, 90);

    const result = await worker.recognize(canvas);
    log("RECOGNIZED TEXT:");
    log(result.data.text || "[EMPTY]");
    log(`CONFIDENCE: ${result.data.confidence}`);
  } catch (error) {
    log(`ERROR: ${error instanceof Error ? error.stack : String(error)}`);
  } finally {
    if (worker) await worker.terminate();
  }
}

run();
