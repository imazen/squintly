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
    // Galaxy Z Fold 7 cover display — folded portrait (tall narrow phone).
    // Physical: 904×2316 device px, DPR ~3 → CSS-px viewport ≈ 304×772.
    // The curator UI must work one-handed-thumb here.
    {
      name: 'zfold7-cover',
      use: {
        ...devices['Pixel 7'],
        viewport: { width: 304, height: 772 },
        deviceScaleFactor: 3,
        isMobile: true,
        hasTouch: true,
      },
    },
    // Galaxy Z Fold 7 inner display — unfolded portrait (small tablet).
    // Physical: 2184×1968 device px, DPR ~2.625 → CSS-px viewport ≈ 749×832.
    // Square-ish: curator layout uses the >720px side-by-side breakpoint here.
    {
      name: 'zfold7-inner',
      use: {
        ...devices['Pixel 7'],
        viewport: { width: 749, height: 832 },
        deviceScaleFactor: 2.625,
        isMobile: true,
        hasTouch: true,
      },
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
