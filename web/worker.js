import init, { parse_files } from "./pkg/gerbrr.js";

self.onmessage = async ({ data }) => {
  try {
    await init();
    const board = parse_files(data.names, data.contents);
    self.postMessage({ board }, [board.buffer]);
  } catch (error) {
    self.postMessage({ error: error?.message ?? String(error) });
  }
};
