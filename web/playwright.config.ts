import { defineConfig, devices } from '@playwright/test';

const SQUINTLY_PORT = 18030;
const COEFFICIENT_PORT = 18081;

export default defineConfig({
  testDir: './e2e',
  fullyParallel: false,           // shared SQLite, mock coefficient, single binary
  workers: 1,
  retries: 0,
  timeout: 30_000,
  expect: { timeout: 5_000 },
  reporter: process.env.CI ? [['github'], ['html', { open: 'never' }]] : [['list']],
  use: {
    baseURL: `http://127.0.0.1:${SQUINTLY_PORT}`,
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
    video: 'retain-on-failure',
  },
  projects: [
    // Chromium engine in a phone-shaped viewport. Pixel 7 default; Squintly is
    // phone-first so this is the canonical project.
    {
      name: 'chromium-phone',
      use: { ...devices['Pixel 7'] },
    },
    // Plain desktop Chromium for the wider-viewport flow.
    {
      name: 'chromium-desktop',
      use: { ...devices['Desktop Chrome'] },
    },
    // Optional iOS Safari project — only useful if WebKit is installed locally
    // (`npx playwright install webkit`). Tests do skip cleanly when WebKit is
    // missing because this project will simply fail to launch and we report it
    // as such; for now we keep it commented to avoid blocking quick local runs.
    // {
    //   name: 'safari-iphone',
    //   use: { ...devices['iPhone 14'] },
    // },
  ],
  globalSetup: './e2e/global-setup.ts',
  globalTeardown: './e2e/global-teardown.ts',
});

export { SQUINTLY_PORT, COEFFICIENT_PORT };
