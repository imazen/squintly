// The front door.
//
// Opening squintly used to walk straight into onboarding and then into trials,
// so the only way to find out what the app was, how far the study had got, or
// who else was contributing was to start rating and then go looking. That is
// backwards for a volunteer study: the decision to take part is made on this
// screen, and it was the one screen with nothing on it to decide from.
//
// So: what it is for, what the study still needs, who is contributing, and two
// ways in — sign in, or take part as a guest. Neither is presented as the
// lesser option, because an anonymous observer's data is worth exactly as much
// as a signed-in one's. Signing in buys one thing (carrying your observer id to
// another device) and it says so.

import { listLeaderboard, studyProgress, whoami, type LeaderboardRow, type StudyProgress } from './api';
import { getObserverId } from './conditions';

/// Percentage of a study's minimum-viable target, capped for display.
function pct(done: number, target: number): number {
  if (target <= 0) return 0;
  return Math.min(100, Math.round((done / target) * 100));
}

function escapeHtml(s: string): string {
  return s.replace(
    /[&<>"']/g,
    (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]!,
  );
}

/// A study's progress toward the two numbers it pre-registered.
///
/// Two bars, not one: "usable" and "done" are different thresholds and a single
/// bar to an unlabelled target cannot say which one it is filling. Below the
/// minimum the number is honest about being unusable rather than showing a
/// cheerful sliver.
function studyRow(s: StudyProgress): string {
  const viable = s.responses >= s.min_viable_ratings;
  const p = pct(s.responses, viable ? s.ideal_ratings : s.min_viable_ratings);
  return `
    <div class="goal" data-study="${escapeHtml(s.id)}">
      <div class="goal-head">
        <span class="goal-name">${escapeHtml(s.short_name)}</span>
        <span class="goal-count muted">${s.responses.toLocaleString()} ratings · ${s.observers} ${
          s.observers === 1 ? 'reviewer' : 'reviewers'
        }</span>
      </div>
      <div class="goal-bar"><div class="goal-fill${viable ? ' viable' : ''}" style="width:${p}%"></div></div>
      <div class="goal-legend muted">
        ${
          viable
            ? `past the usable threshold (${s.min_viable_ratings.toLocaleString()}) — ${p}% of the way to ${s.ideal_ratings.toLocaleString()}`
            : `${s.min_viable_ratings.toLocaleString()} needed before this study says anything · ${(
                s.min_viable_ratings - s.responses
              ).toLocaleString()} to go`
        }
      </div>
    </div>`;
}

function boardRow(r: LeaderboardRow): string {
  const isMe = r.is_you;
  const pctOf = (v: number | null) => (v === null ? '—' : `${Math.round(v * 100)}%`);
  return `<tr class="${isMe ? 'me' : ''}">
    <td><code>${escapeHtml(r.handle)}</code>${isMe ? ' <span class="you">you</span>' : ''}</td>
    <td>${r.trials.toLocaleString()}</td>
    <td>${pctOf(r.self_agreement)}</td>
    <td>${(r.active_seconds / 3600).toFixed(1)}</td>
  </tr>`;
}

export interface LandingActions {
  onStart: () => void;
  onSignIn: () => void;
  onSignOut: () => void;
  onCalibrate: () => void;
}

/**
 * Render the landing page.
 *
 * Everything except the two buttons is best-effort: a leaderboard that will not
 * load must not stop somebody taking part, so each panel fails to a short
 * message of its own rather than taking the page down with it.
 */
export async function renderLanding(root: HTMLElement, actions: LandingActions): Promise<void> {
  const [me, goals, board] = await Promise.all([
    whoami().catch(() => ({ signed_in: false, email: null, observer_id: null, is_admin: false })),
    studyProgress().catch(() => [] as StudyProgress[]),
    listLeaderboard(getObserverId()).catch(() => [] as LeaderboardRow[]),
  ]);

  // Your own row is pulled to the top of the shown slice as well as marked —
  // a board you cannot find yourself on is a scoreboard for other people.
  const shown = [...board.filter((r) => r.is_you), ...board.filter((r) => !r.is_you)].slice(0, 10);

  root.innerHTML = `
    <div class="screen landing" data-screen="landing">
      <header class="landing-head">
        <h1>Squintly</h1>
        <p class="lede">Help make the web faster. You are shown two compressed versions of
          the same picture and asked which is closer to the original — a few seconds each.
          Those judgements train an open-source quality metric, so image CDNs can ship
          smaller files without anyone seeing the difference.</p>
        <p class="muted">Works on any browser; phones are the most useful, because that is
          where most web images are actually looked at.</p>
      </header>

      <div class="landing-cta">
        <div class="cta-row">
          <button id="landing-start" class="primary big">${
            me.signed_in ? 'Continue rating' : 'Start rating as a guest'
          }</button>
          ${
            me.signed_in
              ? ''
              : '<button id="landing-signin" class="big secondary">Sign in with email</button>'
          }
        </div>
        ${
          me.signed_in
            ? `<div class="signed-in">Signed in as <b>${escapeHtml(me.email ?? '')}</b>
                 <button id="landing-signout" class="secondary">Sign out</button></div>`
            : `<p class="cta-note">Both start rating straight away. Signing in only means your
                 reviewer name and totals follow you to another device — and that we can tell
                 which sessions were yours if we ever need to ask about one.</p>`
        }
      </div>

      <section class="landing-panel">
        <h2>What the study still needs</h2>
        <div id="landing-goals">${
          goals.length
            ? goals.map(studyRow).join('')
            : '<p class="muted">Study progress is unavailable right now.</p>'
        }</div>
      </section>

      <section class="landing-panel">
        <h2>Reviewers</h2>
        ${
          board.length
            ? `<table class="board landing-board">
                 <thead><tr><th>Reviewer</th><th>Ratings</th>
                   <th title="Agreement with themselves on repeated pairs — the ceiling any metric could reach against this reviewer">Self-agree</th>
                   <th title="Engaged time, breaks excluded">Hours</th></tr></thead>
                 <tbody>${shown.map(boardRow).join('')}</tbody>
               </table>
               <p class="muted tiny">Names are derived from a salted hash and cannot be reversed.
                 Ranked by volume, but self-agreement is shown beside it on purpose: rating a lot
                 quickly is only worth something if the answers are consistent.</p>`
            : '<p class="muted">No reviewers yet — you would be the first.</p>'
        }
      </section>

      <p class="muted tiny landing-foot">
        <a id="landing-calibrate" href="#">Calibrate screen size</a> ·
        <a href="https://github.com/imazen/squintly" target="_blank" rel="noreferrer noopener">Source</a>
      </p>
    </div>`;

  root.querySelector<HTMLButtonElement>('#landing-start')!.addEventListener('click', actions.onStart);
  root.querySelector<HTMLButtonElement>('#landing-signin')?.addEventListener('click', actions.onSignIn);
  root.querySelector<HTMLButtonElement>('#landing-signout')?.addEventListener('click', actions.onSignOut);
  root.querySelector<HTMLAnchorElement>('#landing-calibrate')!.addEventListener('click', (e) => {
    e.preventDefault();
    actions.onCalibrate();
  });
}
