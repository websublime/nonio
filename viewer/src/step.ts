// A question asked while a walkthrough stop is on screen is about that step. The step goes to Claude as the start of
// the question, so Claude knows it without anything new on the server; the conversation shows it the way it shows a
// selection, under the question, rather than as part of the question's text.
const PREFIX = /^About the walkthrough step “([^”]*)”: /;

export const askAboutStep = (step: string, question: string): string => `About the walkthrough step “${step}”: ${question}`;

export function splitStep(question: string): { question: string; step: string | null } {
  const m = PREFIX.exec(question);
  return m ? { question: question.slice(m[0].length), step: m[1]! } : { question, step: null };
}
