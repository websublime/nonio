// Must run before Prism loads: tells it not to highlight the page by itself, since nonio asks for tokens only
// (highlight.ts). Imported first by highlight.ts.
const g = globalThis as unknown as { Prism?: { manual?: boolean; disableWorkerMessageHandler?: boolean } };
g.Prism = { ...(g.Prism ?? {}), manual: true, disableWorkerMessageHandler: true };
export {};
