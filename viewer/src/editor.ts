// Links that open a file at a line in the person's own editor, so nonio never has to be one.

/** Editors that open a file at a line from a link. All take `<scheme>://file/<absolute path>:<line>`. */
export const EDITORS = [
  { id: 'vscode', name: 'VS Code' },
  { id: 'cursor', name: 'Cursor' },
  { id: 'windsurf', name: 'Windsurf' },
  { id: 'zed', name: 'Zed' },
] as const;
export type EditorId = (typeof EDITORS)[number]['id'];

/** A link that opens `file` at `line` in an editor. `root` is the repo's folder on this machine. */
export function editorUrl(editor: EditorId, root: string, file: string, line: number): string {
  const abs = `${root.replace(/\\/g, '/').replace(/\/+$/, '')}/${file}`;
  return `${editor}://file${abs.startsWith('/') ? '' : '/'}${abs.split('/').map(encodeURIComponent).join('/').replace(/%3A/g, ':')}:${line}`;
}

