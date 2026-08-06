// The reviewer board, on its own page.
//
// It used to be a panel on the front page, below the study goals. Two problems
// with that. It made the front page long enough that the one thing it exists to
// carry — the decision to take part, and the button that acts on it — was
// competing with a table nobody needs before they have rated anything. And a
// board is something people come back to look at, which means it wants a URL
// they can return to rather than a scroll position.
//
// So `/board` is its own route, linked from the front page and from the
// end-of-session screen, and the front page keeps a single line saying how many
// reviewers there are.

import { listLeaderboard, whoami, type LeaderboardRow } from './api';
import { getObserverId } from './conditions';
import { TASK_BLURB } from './vocab';

function escapeHtml(s: string): string {
  return s.replace(
    /[&<>"']/g,
    (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]!,
  );
}

const pct = (v: number | null) => (v === null ? '—' : `${Math.round(v * 100)}%`);

function row(r: LeaderboardRow, rank: number): string {
  return `<tr class="${r.is_you ? 'me' : ''}">
    <td class="rank">${rank}</td>
    <td><code>${escapeHtml(r.handle)}</code>${r.is_you ? ' <span class="you">you</span>' : ''}</td>
    <td>${r.trials.toLocaleString()}</td>
    <td>${pct(r.self_agreement)}</td>
    <td>${pct(r.golden_pass_rate)}</td>
    <td>${(r.active_seconds / 3600).toFixed(1)}</td>
  </tr>`;
}

export interface BoardActions {
  onBack: () => void;
  onStart: () => void;
}

/**
 * Render the reviewer board.
 *
 * Everyone is listed, not a top ten: at the scale this study runs, a cut-off
 * would hide most of the participants from a board whose whole purpose is that
 * people can see themselves on it.
 */
export async function renderBoard(root: HTMLElement, actions: BoardActions): Promise<void> {
  root.innerHTML = `
    <div class="screen landing" data-screen="board">
      <h1>Reviewers</h1>
      <p class="muted">${escapeHtml(TASK_BLURB)}</p>
      <div id="board-body" class="muted">Loading…</div>
      <div class="row">
        <button id="board-start" class="primary">Rate some images</button>
        <button id="board-back">Back</button>
      </div>
    </div>`;
  root.querySelector<HTMLButtonElement>('#board-start')!.addEventListener('click', actions.onStart);
  root.querySelector<HTMLButtonElement>('#board-back')!.addEventListener('click', actions.onBack);

  const host = root.querySelector<HTMLElement>('#board-body')!;
  const [board, me] = await Promise.all([
    listLeaderboard(getObserverId()).catch(() => [] as LeaderboardRow[]),
    whoami().catch(() => null),
  ]);
  if (!board.length) {
    host.innerHTML = `<p class="muted">No reviewers yet — you would be the first.</p>`;
    return;
  }

  // Ranked by volume, but the caller's own row is pulled to the top as well as
  // shown in place: a board you have to hunt yourself on is a scoreboard for
  // other people. `is_you` marks both copies so the duplicate reads as emphasis
  // rather than as two reviewers.
  const mine = board.find((r) => r.is_you);
  host.innerHTML = `
    ${
      mine
        ? `<p class="lede">You have ${mine.trials.toLocaleString()} ratings, agreeing with
             yourself ${pct(mine.self_agreement)} of the time on repeats.</p>`
        : `<p class="lede">You are not on the board yet — it takes one rating.</p>`
    }
    <table class="board landing-board">
      <thead><tr><th></th><th>Reviewer</th><th>Ratings</th>
        <th title="Agreement with themselves on repeated pairs — the ceiling any metric could reach against this reviewer">Self-agree</th>
        <th title="Attention checks passed">Checks</th>
        <th title="Engaged time: gaps between consecutive answers, each capped so a break is not counted">Hours</th></tr></thead>
      <tbody>${board.map((r, i) => row(r, i + 1)).join('')}</tbody>
    </table>
    <p class="muted tiny">Names come from a salted hash and cannot be reversed — not even here.
      Ranked by volume, with self-agreement beside it on purpose: rating a lot quickly is only
      worth something if the answers are consistent.
      ${me?.is_admin ? 'Operator view has the rest.' : ''}</p>`;
}
