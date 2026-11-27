import { describe, expect, it } from 'vitest';
import { followTarget, latestBelow } from '../src/follow';

describe('following the newest turn', () => {
  it('scrolls so the newest turn starts at the top of the view, not to the end of a long reply', () => {
    // View starts at y=100 on screen and is scrolled 500px; the newest turn starts 900px down the screen.
    expect(followTarget(500, 100, 900)).toBe(500 + 800 - 12);
  });

  it('never asks for a negative scroll', () => {
    expect(followTarget(0, 100, 50)).toBe(0);
  });

  it('says the newest turn is out of view only when it starts below what is on screen', () => {
    expect(latestBelow(800, 900)).toBe(true);
    expect(latestBelow(800, 300)).toBe(false);
    // Reading the newest reply itself, its start scrolled off the top: not "away".
    expect(latestBelow(800, -400)).toBe(false);
  });
});
