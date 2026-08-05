// Boot the mock coefficient, then the squintly Rust binary, before any tests
// run. We use the production-shape pipeline: vite build + cargo build --release
// before spawning, so tests exercise the embedded frontend, not the dev server.
//
// State lives in ~/tmp/squintly-e2e/squintly.db; we wipe it on each run to keep
// tests deterministic.

import { spawn, type ChildProcess } from 'node:child_process';
import { mkdirSync, rmSync, existsSync } from 'node:fs';
import { homedir } from 'node:os';
import { setTimeout as sleep } from 'node:timers/promises';

import { WORKERS, coefficientPortFor, squintlyPortFor } from '../playwright.config';

// ~/tmp, not /tmp: /tmp can be wiped mid-run on this box (see global CLAUDE.md).
const STATE_DIR = `${homedir()}/tmp/squintly-e2e`;

const started: ChildProcess[] = [];

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

  // One isolated stack per worker. Started in parallel — serially this would
  // add a cargo-free but still real few seconds per worker to every run.
  await Promise.all(
    Array.from({ length: WORKERS }, async (_, w) => {
    // 1. Mock coefficient.
    const mock = spawn('node', ['--import', 'tsx', 'e2e/mock-coefficient.ts'], {
      env: { ...process.env, COEFFICIENT_PORT: String(coefficientPortFor(w)) },
      stdio: ['ignore', 'inherit', 'inherit'],
    });
    await waitForOk(`http://127.0.0.1:${coefficientPortFor(w)}/health`);

    // 2. squintly binary.
    const server = spawn(
      binPath,
      [
        '--coefficient-http', `http://127.0.0.1:${coefficientPortFor(w)}`,
        '--bind', `127.0.0.1:${squintlyPortFor(w)}`,
        '--db', `${STATE_DIR}/w${w}.db`,
      ],
      {
        env: {
          ...process.env,
          RUST_LOG: 'warn,squintly=info',
          // The e2e DB is wiped per run — never mirror it to the Tower NAS.
          SQUINTLY_DISABLE_TOWER_MIRROR: '1',
          // The suite drives the mixed study, not the deployment default.
          //
          // Production defaults to `ssim2-nonphoto` while imazen/squintly#4
          // collects — pairwise-only, and with `main` unlisted there is no picker
          // to reach a rating trial through the UI. Most specs here exercise the
          // single-stimulus path (rating panel, hold-to-reveal, staircases), so
          // the harness pins the study that emits both trial types. Specs that
          // care about the non-photo study name it explicitly, and
          // `studies::the_resolved_default_study_is_listed` covers the real
          // default.
          SQUINTLY_DEFAULT_STUDY: 'main',
          // Admin is real here: curator writes are admin-only, so the suite signs
          // in through the actual magic-link flow against the mock's mail sink
          // rather than through a test-only backdoor.
          SQUINTLY_ADMIN_EMAILS: 'admin@e2e.test',
          SQUINTLY_SUGGESTION_ADMIN_TOKEN: 'e2e-admin-token',
          POSTMARK_SERVER_TOKEN: 'stub',
          POSTMARK_FROM_EMAIL: 'noreply@e2e.test',
          POSTMARK_API_BASE: `http://127.0.0.1:${coefficientPortFor(w)}`,
          // The verify link is opened over plain http in the harness; a Secure
          // cookie would be dropped and sign-in would appear to succeed while
          // granting nothing.
          SQUINTLY_INSECURE_COOKIES: '1',
          // Every curator spec signs in as the same admin, so the 60s per-address
          // cooldown would block all but the first — the suite is not testing the
          // limiter here, and `auth_rate_limit_and_admin` covers it with explicit
          // values of its own.
          SQUINTLY_AUTH_COOLDOWN_MS: '0',
          SQUINTLY_AUTH_PER_EMAIL_HOURLY: '0',
          SQUINTLY_AUTH_PER_IP_HOURLY: '0',
          // The mock coefficient serves blobs from 127.0.0.1, which the SSRF
          // guard blocks by default. Opt in here only; never on a public deploy.
          SQUINTLY_ALLOW_PRIVATE_BLOB_HOSTS: '1',
        },
        stdio: ['ignore', 'inherit', 'inherit'],
      },
    );
    await waitForOk(`http://127.0.0.1:${squintlyPortFor(w)}/api/stats`);

      started.push(mock, server);
    }),
  );

  // Hand the child handles to teardown via globalThis so the matching teardown
  // file can find them.
  (globalThis as unknown as { __squintly_e2e: { children: ChildProcess[] } })
    .__squintly_e2e = { children: started };
}
