// Operator view: what the study has, and who it came from.
//
// Everything here was already reachable — as JSON, from a terminal, by someone
// who knew the endpoint names. That is not the same as being available: the
// person who most needs to see whether a study is close to viable, or whether a
// reviewer is contributing noise, is the one running it from a phone between
// other things.
//
// Read-only on purpose. Exclusion is a recorded disposition, not a button; a
// screen that let an operator drop a reviewer with a tap would make the
// screening decision unauditable, which is the thing `exclusion.rs` exists to
// prevent.

import {
  listLeaderboard,
  studyProgress,
  whoami,
  type LeaderboardRow,
  type StudyProgress,
} from './api';

function escapeHtml(s: string): string {
  return s.replace(
    /[&<>"']/g,
    (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]!,
  );
}

interface Stats {
  observers: number;
  sessions: number;
  trials: number;
  responses: number;
  manifest_sources: number;
  manifest_encodings: number;
  build_commit: string;
}

function studyLine(s: StudyProgress): string {
  const viable = s.responses >= s.min_viable_ratings;
  return `<tr>
    <td>${escapeHtml(s.short_name)}${s.is_default ? ' <span class="you">default</span>' : ''}</td>
    <td>${s.responses.toLocaleString()}</td>
    <td>${s.min_viable_ratings.toLocaleString()}</td>
    <td>${s.ideal_ratings.toLocaleString()}</td>
    <td>${s.observers}</td>
    <td class="${viable ? 'ok' : 'warn-cell'}">${viable ? 'usable' : 'below minimum'}</td>
  </tr>`;
}

function reviewerLine(r: LeaderboardRow): string {
  const pct = (v: number | null) => (v === null ? '—' : `${Math.round(v * 100)}%`);
  const num = (v: number | null, d = 1) => (v === null ? '—' : v.toFixed(d));
  return `<tr class="${r.is_you ? 'me' : ''}">
    <td><code>${escapeHtml(r.handle)}</code></td>
    <td>${r.trials.toLocaleString()}</td>
    <td>${r.instrumented_trials.toLocaleString()}</td>
    <td>${pct(r.self_agreement)}</td>
    <td>${pct(r.golden_pass_rate)}</td>
    <td>${num(r.median_seconds)}</td>
    <td>${num(r.median_switches, 0)}</td>
    <td>${(r.active_seconds / 3600).toFixed(2)}</td>
  </tr>`;
}

/**
 * Render the admin view. `onBack` returns to whatever was on screen.
 *
 * Re-checks admin status rather than trusting the caller: the menu only shows
 * the entry to an admin, but a menu is display logic and this reads real data.
 * Every privileged endpoint re-checks server-side too — this is the third
 * layer, not the only one.
 */
export async function showAdmin(root: HTMLElement, onBack: () => void): Promise<void> {
  const me = await whoami().catch(() => null);
  if (!me?.is_admin) {
    root.innerHTML = `
      <div class="screen center" data-screen="admin">
        <h1>Not available</h1>
        <p class="muted">This view is for study operators.</p>
        <button id="admin-back" class="primary">Back</button>
      </div>`;
    root.querySelector<HTMLButtonElement>('#admin-back')!.addEventListener('click', onBack);
    return;
  }

  root.innerHTML = `
    <div class="screen admin" data-screen="admin">
      <h1>Study operations</h1>
      <p class="muted">Read-only. Exclusion is a recorded disposition, not a button —
        see <code>exclusion.rs</code>.</p>
      <div id="admin-body" class="muted">Loading…</div>
      <div class="row"><button id="admin-back" class="primary">Back to rating</button></div>
    </div>`;
  root.querySelector<HTMLButtonElement>('#admin-back')!.addEventListener('click', onBack);

  const host = root.querySelector<HTMLElement>('#admin-body')!;
  try {
    const [stats, goals, board] = await Promise.all([
      fetch('/api/stats').then((r) => r.json() as Promise<Stats>),
      studyProgress(),
      listLeaderboard(),
    ]);
    host.innerHTML = `
      <section class="landing-panel">
        <h2>Corpus and totals</h2>
        <table class="board">
          <tbody>
            <tr><td>Responses</td><td>${stats.responses.toLocaleString()}</td></tr>
            <tr><td>Trials served</td><td>${stats.trials.toLocaleString()}</td></tr>
            <tr><td>Sessions</td><td>${stats.sessions.toLocaleString()}</td></tr>
            <tr><td>Observers</td><td>${stats.observers.toLocaleString()}</td></tr>
            <tr><td>Corpus</td><td>${stats.manifest_sources.toLocaleString()} sources ·
              ${stats.manifest_encodings.toLocaleString()} encodings</td></tr>
            <tr><td>Build</td><td><code>${escapeHtml(stats.build_commit.slice(0, 12))}</code></td></tr>
          </tbody>
        </table>
      </section>

      <section class="landing-panel">
        <h2>Studies</h2>
        <table class="board">
          <thead><tr><th>Study</th><th>Ratings</th><th>Min viable</th><th>Ideal</th>
            <th>Reviewers</th><th>State</th></tr></thead>
          <tbody>${goals.map(studyLine).join('')}</tbody>
        </table>
      </section>

      <section class="landing-panel">
        <h2>Reviewers</h2>
        <table class="board">
          <thead><tr><th>Handle</th><th>Ratings</th>
            <th title="How many carry the per-view effort counters">Instr.</th>
            <th title="Agreement with themselves on repeated pairs">Self-agree</th>
            <th title="Attention-check pass rate">Checks</th>
            <th>s/trial</th><th>Swaps</th><th>Hours</th></tr></thead>
          <tbody>${board.map(reviewerLine).join('')}</tbody>
        </table>
        <p class="muted tiny">Handles are salted hashes; this view cannot reverse them
          either. Self-agreement is the ceiling any metric could reach against that
          reviewer — read a low one beside a high rating count as noise, not output.</p>
      </section>

      <section class="landing-panel">
        <h2>Exports</h2>
        <p class="muted tiny">
          <a href="/api/export/responses.tsv">responses.tsv</a> ·
          <a href="/api/export/pareto.tsv">pareto.tsv</a> ·
          <a href="/api/export/thresholds.tsv">thresholds.tsv</a> ·
          <a href="/api/export/unified.tsv">unified.tsv</a>
        </p>
      </section>`;
  } catch (e) {
    host.innerHTML = `<p class="muted">Couldn't load operations data: ${escapeHtml(
      (e as Error).message,
    )}</p>`;
  }
}
