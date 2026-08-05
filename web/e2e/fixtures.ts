// Point each worker at its own stack.
//
// `global-setup` starts one squintly + one mock coefficient per worker on
// consecutive ports, each with its own SQLite file. This binds a test to the
// pair belonging to the worker running it, so parallel workers cannot see each
// other's observers, sessions or responses.
//
// Specs import `test`/`expect` from here instead of `@playwright/test`. That is
// the only change they need: `baseURL` is a built-in option fixture, so
// overriding it re-points every `page.goto('/')` and every relative
// `page.request` call at once.

import { test as base, expect } from '@playwright/test';

import { coefficientPortFor, squintlyPortFor } from '../playwright.config';

export const test = base.extend<{ coefficientPort: number }>({
  baseURL: async ({}, use, testInfo) => {
    await use(`http://127.0.0.1:${squintlyPortFor(testInfo.parallelIndex)}`);
  },
  // The mock doubles as the mail sink (`/outbox`) and as the blob origin, so
  // anything reaching for it directly needs this worker's instance, not a
  // fixed port.
  coefficientPort: async ({}, use, testInfo) => {
    await use(coefficientPortFor(testInfo.parallelIndex));
  },
});

export { expect };
