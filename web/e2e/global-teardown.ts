import type { ChildProcess } from 'node:child_process';

export default async function globalTeardown() {
  const handles = (globalThis as unknown as {
    __squintly_e2e?: { children: ChildProcess[] };
  }).__squintly_e2e;
  if (!handles) return;
  for (const proc of handles.children) {
    if (proc && !proc.killed) proc.kill('SIGTERM');
  }
  // Best-effort wait so the OS releases every worker's ports before the next
  // run. A leftover listener is not harmless here: the next run's setup would
  // fail with EADDRINUSE and every test in that worker would look broken.
  await new Promise((r) => setTimeout(r, 500));
}
