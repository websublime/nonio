// Builds the tool the tests read maps with, once before they run.
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

export default function setup() {
  execFileSync('cargo', ['build', '--quiet', '-p', 'nonio-server', '--example', 'viewer-maps'], {
    cwd: fileURLToPath(new URL('../..', import.meta.url)),
    stdio: 'inherit',
  });
}
