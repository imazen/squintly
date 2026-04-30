import type { ChildProcessWithoutNullStreams } from 'node:child_process';

export default async function globalTeardown() {
  const handles = (globalThis as unknown as {
    __squintly_e2e?: {
      mock: ChildProcessWithoutNullStreams | null;
      server: ChildProcessWithoutNullStreams | null;
    };
  }).__squintly_e2e;
  if (!handles) return;
  for (const proc of [handles.server, handles.mock]) {
    if (proc && !proc.killed) {
      proc.kill('SIGTERM');
    }
  }
  // Best-effort wait so the OS releases the ports before the next run.
  await new Promise((r) => setTimeout(r, 300));
}
