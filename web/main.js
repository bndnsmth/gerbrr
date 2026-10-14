const canvas = document.querySelector("#gerbrr");
const startup = document.querySelector("#startup");
const status = document.querySelector("#startup-status");
const overlay = document.querySelector("#drop-overlay");
const notice = document.querySelector("#notice");
const fileInput = document.querySelector("#file-input");
const folderInput = document.querySelector("#folder-input");
const screenPrompt = document.querySelector("#small-screen");
const continueButton = document.querySelector("#continue-small-screen");
const smallScreen = window.matchMedia("(max-width: 899px), (max-height: 599px) and (pointer: coarse)");
let screenAccepted = false;
let starting;
let viewer;
let importing = false;
let noticeTimer;
let dragDepth = 0;

function notify(message, error = false) {
  clearTimeout(noticeTimer);
  notice.textContent = message;
  notice.dataset.error = String(error);
  notice.hidden = false;
  noticeTimer = setTimeout(() => { notice.hidden = true; }, error ? 12000 : 6000);
}

function fatal(error) {
  startup.hidden = false;
  const message = error?.message ?? String(error);
  const graphics = /create (?:wgpu )?surface|adapter|request.*device/i.test(message);
  status.textContent = graphics
    ? `Could not initialize browser graphics. Gerbrr needs WebGPU or WebGL2. Enable browser hardware acceleration and try current Chrome, Edge, or Firefox. WebGPU also requires localhost or HTTPS.\nDetails: ${message}`
    : `Could not start: ${message}`;
  status.style.color = "#f2b170";
  document.body.dataset.ready = "error";
  console.error(error);
}

function workerParse(names, contents) {
  return new Promise((resolve, reject) => {
    const worker = new Worker(new URL("./worker.js", import.meta.url), { type: "module" });
    const timer = setTimeout(() => { worker.terminate(); reject(new Error("Import exceeded the two-minute PoC limit")); }, 120000);
    const finish = () => { clearTimeout(timer); worker.terminate(); };
    worker.onmessage = ({ data }) => {
      finish();
      if (data.error) reject(new Error(data.error));
      else resolve(data.board);
    };
    worker.onerror = (event) => { finish(); reject(new Error(event.message || "Parser worker failed")); };
    worker.postMessage({ names, contents }, contents.map((bytes) => bytes.buffer));
  });
}

async function importFiles(files) {
  if (screenPrompt.open) return;
  const selection = Array.from(files ?? []).filter((file) => !file.webkitRelativePath
    || /\.(zip|gbr|gbx|art|pho|ger|gerber|gtl|gbl|gto|gbo|gts|gbs|gtp|gbp|gpb|gpt|gko|gm\d+|gml|drl|drd|xln|exc|rou|txt|g\d+)$/i.test(file.name));
  if (!selection.length) return;
  if (!viewer || importing || !viewer.begin_import()) { notify("A board is already loading. Please wait."); return; }
  importing = true;
  document.body.dataset.importing = "true";
  const start = performance.now();
  try {
    if (selection.length > 256) throw new Error("Select at most 256 files");
    if (selection.some((file) => file.size > 128 * 1024 * 1024)) throw new Error("A selected file exceeds 128 MiB");
    if (selection.reduce((total, file) => total + file.size, 0) > 512 * 1024 * 1024) throw new Error("Selected files exceed 512 MiB");
    const names = selection.map((file) => file.webkitRelativePath || file.name);
    const contents = await Promise.all(selection.map(async (file) => new Uint8Array(await file.arrayBuffer())));
    notify("Parsing in a local Web Worker. Your board stays in this browser.");
    const board = await workerParse(names, contents);
    viewer.finish_import(board);
    // Egui consumes the queued document on the next animation frame.
    await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    const summary = viewer.summary();
    window.gerbrrLastImport = { ...summary, elapsedMs: performance.now() - start };
    notify(`${summary.layers} layers loaded · ${summary.primitives.toLocaleString()} primitives · ${summary.warnings} warnings`);
    document.title = `${summary.name} · Gerbrr`;
    document.body.dataset.imported = summary.name;
    document.body.dataset.importError = "";
  } catch (error) {
    const message = error?.message ?? String(error);
    viewer.fail_import(message);
    document.body.dataset.importError = message;
    notify(`Import failed: ${message}`, true);
    console.error(error);
  } finally {
    importing = false;
    document.body.dataset.importing = "false";
    canvas.focus();
  }
}

window.gerbrrBridge = {
  openFiles(folder) {
    const input = folder ? folderInput : fileInput;
    input.value = "";
    input.click();
  },
  exportCanvas() {
    notify("Preparing workspace PNG…");
    requestAnimationFrame(() => requestAnimationFrame(() => canvas.toBlob((blob) => {
      if (!blob) { notify("Browser could not encode the workspace image", true); return; }
      const url = URL.createObjectURL(blob);
      const link = document.createElement("a");
      link.href = url; link.download = "gerbrr-board.png"; document.body.append(link); link.click(); link.remove();
      setTimeout(() => URL.revokeObjectURL(url), 10000);
      notify("Workspace PNG downloaded.");
    }, "image/png")));
  },
};
fileInput.addEventListener("change", () => importFiles(fileInput.files));
folderInput.addEventListener("change", () => importFiles(folderInput.files));

// Capture before eframe's drop handler: browser filenames are not filesystem paths.
window.addEventListener("dragenter", (event) => {
  if (!event.dataTransfer?.types.includes("Files")) return;
  event.preventDefault(); dragDepth++; overlay.hidden = false;
}, true);
window.addEventListener("dragover", (event) => {
  if (!event.dataTransfer?.types.includes("Files")) return;
  event.preventDefault(); event.dataTransfer.dropEffect = "copy";
}, true);
window.addEventListener("dragleave", () => { if (--dragDepth <= 0) { dragDepth = 0; overlay.hidden = true; } }, true);
window.addEventListener("drop", (event) => {
  if (!event.dataTransfer?.files.length) return;
  event.preventDefault(); event.stopImmediatePropagation(); dragDepth = 0; overlay.hidden = true;
  if (screenPrompt.open) return;
  importFiles(event.dataTransfer.files);
}, true);

function startViewer() {
  if (starting) return starting;
  starting = (async () => {
    try {
      const wasm = await import("./pkg/gerbrr.js");
      await wasm.default();
      viewer = new wasm.BrowserApp();
      const forceWebgl = new URLSearchParams(location.search).get("renderer") === "webgl";
      await viewer.start(canvas, forceWebgl);
      startup.hidden = true;
      document.body.dataset.ready = "true";
      window.gerbrr = viewer;
      if (!screenPrompt.open) canvas.focus();
      setInterval(() => { const panic = viewer.panic_message(); if (panic) fatal(panic); }, 1000);
      console.info("Gerbrr Web PoC ready. File parsing is local; no board uploads.", viewer.summary());
    } catch (error) { fatal(error); }
  })();
  return starting;
}

function updateScreenPrompt() {
  const blocked = smallScreen.matches && !screenAccepted;
  document.body.dataset.screen = blocked ? "small" : "viewer";
  if (blocked) {
    if (!screenPrompt.open) screenPrompt.showModal();
  } else {
    if (screenPrompt.open) screenPrompt.close();
    void startViewer();
    if (document.body.dataset.ready === "true") canvas.focus();
  }
}

// This is an explicit opt-in, not a modal that Escape accidentally bypasses.
screenPrompt.addEventListener("cancel", event => event.preventDefault());
continueButton.addEventListener("click", () => {
  screenAccepted = true;
  updateScreenPrompt();
});
smallScreen.addEventListener("change", updateScreenPrompt);
// Do not download the WASM or allocate a GPU surface on a small screen until
// the visitor chooses Continue anyway (or resizes to a desktop-sized viewport).
updateScreenPrompt();
