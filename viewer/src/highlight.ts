// Syntax colours for the code view (code.tsx). Prism splits a block into tokens (keyword, string, function, ...); they
// are returned per line as plain pieces, so the code view builds them as React elements, never HTML strings, and keeps
// its own line numbers, cited lines and notes. A file in a language not listed here shows as plain text.
import './prism-setup';
import Prism from 'prismjs';
// Order matters: each language after the ones it builds on.
import 'prismjs/components/prism-markup-templating';
import 'prismjs/components/prism-typescript';
import 'prismjs/components/prism-jsx';
import 'prismjs/components/prism-tsx';
import 'prismjs/components/prism-python';
import 'prismjs/components/prism-go';
import 'prismjs/components/prism-rust';
import 'prismjs/components/prism-solidity';
import 'prismjs/components/prism-java';
import 'prismjs/components/prism-kotlin';
import 'prismjs/components/prism-swift';
import 'prismjs/components/prism-ruby';
import 'prismjs/components/prism-php';
import 'prismjs/components/prism-csharp';
import 'prismjs/components/prism-c';
import 'prismjs/components/prism-cpp';
import 'prismjs/components/prism-sql';
import 'prismjs/components/prism-json';
import 'prismjs/components/prism-yaml';
import 'prismjs/components/prism-bash';
import 'prismjs/components/prism-toml';
import 'prismjs/components/prism-scss';
import 'prismjs/components/prism-graphql';

const BY_EXTENSION: Record<string, string> = {
  ts: 'typescript', mts: 'typescript', cts: 'typescript', tsx: 'tsx', js: 'javascript', mjs: 'javascript', cjs: 'javascript',
  jsx: 'jsx', py: 'python', go: 'go', rs: 'rust', sol: 'solidity', java: 'java', kt: 'kotlin', kts: 'kotlin',
  swift: 'swift', rb: 'ruby', php: 'php', cs: 'csharp', c: 'c', h: 'c', cpp: 'cpp', cc: 'cpp', cxx: 'cpp', hpp: 'cpp',
  sql: 'sql', json: 'json', yml: 'yaml', yaml: 'yaml', sh: 'bash', bash: 'bash', zsh: 'bash', toml: 'toml',
  css: 'css', scss: 'scss', html: 'markup', htm: 'markup', xml: 'markup', svg: 'markup', graphql: 'graphql', gql: 'graphql',
};

/** The Prism language for a file, by its extension; null when it isn't one we colour. */
export function languageOf(file: string): string | null {
  const ext = /\.([^./]+)$/.exec(file)?.[1]?.toLowerCase();
  const lang = ext ? BY_EXTENSION[ext] : undefined;
  return lang && Prism.languages[lang] ? lang : null;
}

/** A run of text on one line, and the token kinds it is inside (`tok-keyword tok-string`, ...), or none. */
export type Piece = { text: string; kind: string | null };

/**
 * `lines` coloured as one piece of code (so a comment or string over several lines stays one), split back into the
 * same lines. A block can start mid-function or mid-comment; Prism then guesses, the way a diff viewer does.
 */
export function highlightLines(lines: readonly string[], lang: string | null): Piece[][] {
  const grammar = lang ? Prism.languages[lang] : undefined;
  if (!grammar) return lines.map((text) => (text ? [{ text, kind: null }] : []));
  const out: Piece[][] = [[]];
  const add = (text: string, kind: string) => {
    text.split('\n').forEach((part, i) => {
      if (i > 0) out.push([]);
      if (part) out[out.length - 1]!.push({ text: part, kind: kind || null });
    });
  };
  const walk = (t: string | Prism.Token, kind: string) => {
    if (typeof t === 'string') return add(t, kind);
    const aliases = t.alias ? (Array.isArray(t.alias) ? t.alias : [t.alias]) : [];
    const inner = [kind, ...[t.type, ...aliases].map((k) => `tok-${k}`)].filter(Boolean).join(' ');
    if (Array.isArray(t.content)) for (const c of t.content) walk(c, inner);
    else walk(t.content, inner);
  };
  for (const t of Prism.tokenize(lines.join('\n'), grammar)) walk(t, '');
  while (out.length < lines.length) out.push([]);
  return out.slice(0, lines.length);
}
