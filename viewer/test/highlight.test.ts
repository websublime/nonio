import { describe, expect, it } from 'vitest';
import { highlightLines, languageOf, type Piece } from '../src/highlight';

const text = (line: Piece[]) => line.map((p) => p.text).join('');
const kindOf = (line: Piece[], word: string) => line.find((p) => p.text === word)?.kind ?? null;

describe('syntax colours', () => {
  it('knows the language by extension, and colours nothing it doesn\'t know', () => {
    expect(languageOf('contracts/WritesFacet.sol')).toBe('solidity');
    expect(languageOf('src/accounts/mod.rs')).toBe('rust');
    expect(languageOf('web/App.TSX')).toBe('tsx');
    expect(languageOf('notes.txt')).toBeNull();
    expect(languageOf('Makefile')).toBeNull();
    expect(highlightLines(['plain', '', 'text'], null)).toEqual([[{ text: 'plain', kind: null }], [], [{ text: 'text', kind: null }]]);
  });

  it('colours Solidity and keeps every line and character as it was', () => {
    const lines = [
      '    function newChainSecuredAccount(',
      '        string memory accountName',
      '    ) public {',
      '        address adminWalletAddress = msg.sender; // the signer',
      '        newAccount(apiKeyHash, false, accountName, "x");',
      '    }',
    ];
    const out = highlightLines(lines, languageOf('WritesFacet.sol'));
    expect(out).toHaveLength(lines.length);
    out.forEach((line, i) => expect(text(line)).toBe(lines[i]));
    expect(kindOf(out[0]!, 'function')).toContain('tok-keyword');
    expect(kindOf(out[4]!, 'false')).toContain('tok-boolean');
    expect(kindOf(out[4]!, '"x"')).toContain('tok-string');
    expect(kindOf(out[3]!, '// the signer')).toContain('tok-comment');
  });

  it('keeps a comment over several lines coloured on each of its lines', () => {
    const out = highlightLines(['/* one', 'two */', 'const a = 1;'], 'typescript');
    expect(kindOf(out[0]!, '/* one')).toContain('tok-comment');
    expect(kindOf(out[1]!, 'two */')).toContain('tok-comment');
    expect(kindOf(out[2]!, 'const')).toContain('tok-keyword');
  });
});
