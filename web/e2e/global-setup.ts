// Boot the mock coefficient, then the squintly Rust binary, before any tests
// run. We use the production-shape pipeline: vite build + cargo build --release
// before spawning, so tests exercise the embedded frontend, not the dev server.
//
// State lives in /tmp/squintly-e2e/squintly.db; we wipe it on each run to keep
// tests deterministic.

import { spawn, type ChildProcessWithoutNullStreams } from 'node:child_process';
import { mkdirSync, rmSync, existsSync } from 'node:fs';
import { setTimeout as sleep } from 'node:timers/promises';

import { COEFFICIENT_PORT, SQUINTLY_PORT } from '../playwright.config';

const STATE_DIR = '/tmp/squintly-e2e';
const DB_PATH = `${STATE_DIR}/squintly.db`;

let mock: ChildProcessWithoutNullStreams | null = null;
let server: ChildProcessWithoutNullStreams | null = null;

async function waitForOk(url: string, attempts = 60): Promise<void> {
  for (let i = 0; i < attempts; i++) {
    try {
      const r = await fetch(url);
      if (r.ok) return;
    } catch {
      // not yet
    }
    await sleep(500);
  }
  throw new Error(`gave up waiting for ${url}`);
}

export default async function globalSetup() {
  // Clean state.
  rmSync(STATE_DIR, { recursive: true, force: true });
  mkdirSync(STATE_DIR, { recursive: true });

  // Build the binary if it doesn't exist (first run only — subsequent runs
  // reuse the cached release binary, which is incremental).
  const binPath = '../target/release/squintly';
  // We don't auto-build here because cargo build is slow and the user is
  // expected to run `just e2e-prep` first. If the binary is missing, fail loud.
  if (!existsSync(binPath)) {
    throw new Error(
      `release binary not found at ${binPath}. Run \`just e2e-prep\` first ` +
      '(builds the frontend then cargo build --release).',
    );
  }

  // 1. Mock coefficient.
  mock = spawn('node', ['--import', 'tsx', 'e2e/mock-coefficient.ts'], {
    env: { ...process.env, COEFFICIENT_PORT: String(COEFFICIENT_PORT) },
    stdio: ['ignore', 'inherit', 'inherit'],
  });
  await waitForOk(`http://127.0.0.1:${COEFFICIENT_PORT}/health`);

  // 2. squintly binary.
  server = spawn(
    binPath,
    [
      '--coefficient-http', `http://127.0.0.1:${COEFFICIENT_PORT}`,
      '--bind', `127.0.0.1:${SQUINTLY_PORT}`,
      '--db', DB_PATH,
    ],
    {
      env: { ...process.env, RUST_LOG: 'warn,squintly=info' },
      stdio: ['ignore', 'inherit', 'inherit'],
    },
  );
  await waitForOk(`http://127.0.0.1:${SQUINTLY_PORT}/api/stats`);

  // Hand the child handles to teardown via globalThis so the matching teardown
  // file can find them.
  (globalThis as unknown as { __squintly_e2e: { mock: typeof mock; server: typeof server } })
    .__squintly_e2e = { mock, server };
}
