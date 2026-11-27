import { describe, expect, it } from 'vitest';
import { parseToured, withToured } from '../src/toured';

describe('the maps whose walkthrough was taken', () => {
  it('reads nothing from a missing or broken value', () => {
    expect(parseToured(null)).toEqual([]);
    expect(parseToured('not json')).toEqual([]);
    expect(parseToured('{"a":1}')).toEqual([]);
    expect(parseToured('["a", 2, "b"]')).toEqual(['a', 'b']);
  });

  it('adds a map once, as the newest', () => {
    expect(withToured(['a', 'b'], 'c')).toEqual(['a', 'b', 'c']);
    expect(withToured(['a', 'b', 'c'], 'a')).toEqual(['b', 'c', 'a']);
  });

  it('drops the oldest past the cap', () => {
    expect(withToured(['a', 'b', 'c'], 'd', 3)).toEqual(['b', 'c', 'd']);
  });
});
