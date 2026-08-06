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
import { detectCodecs, jxlAdvice } from './codec-probe';
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


export interface LandingActions {
  onStart: () => void;
  onSignIn: () => void;
  onSignOut: () => void;
  onCalibrate: () => void;
  /// The reviewer board, now its own route rather than a panel here — a board
  /// is something people return to, so it wants a URL.
  onBoard: () => void;
  /// Only rendered for an admin; the server re-checks anyway.
  onAdmin: () => void;
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
  const mine = board.find((r) => r.is_you);

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
            ? `<p class="muted">${board.length.toLocaleString()}
                 ${board.length === 1 ? 'person has' : 'people have'} rated so far${
                   mine ? `, including you — ${mine.trials.toLocaleString()} ratings` : ''
                 }.</p>
               <div class="row"><button id="landing-board">See the board</button></div>`
            : '<p class="muted">No reviewers yet — you would be the first.</p>'
        }
      </section>

      <div id="landing-jxl"></div>

      <p class="muted tiny landing-foot">
        <a id="landing-calibrate" href="#">Calibrate screen size</a> ·
        ${me?.is_admin ? '<a id="landing-admin" href="#">Operator view</a> · ' : ''}
        <a href="https://github.com/imazen/squintly" target="_blank" rel="noreferrer noopener">Source</a>
      </p>
    </div>`;

  // Codec support, resolved after paint. The probe decodes a tiny test image
  // per format and is not worth blocking the page on — and the panel it feeds
  // is advice, not a gate. Deciding whether to take part is exactly the moment
  // somebody can spare thirty seconds to flip a browser flag; mid-session it
  // would be an interruption, which is why the terse `jxlEnableHint` stays
  // where it is and this instructional form lives only here.
  void (async () => {
    const host = root.querySelector<HTMLElement>('#landing-jxl');
    if (!host) return;
    try {
      const support = await detectCodecs();
      const a = jxlAdvice(support.supported);
      if (a.supported) return;
      host.innerHTML = `
        <section class="landing-panel jxl-advice">
          <h2>${escapeHtml(a.headline)}</h2>
          <p class="muted">${escapeHtml(a.detail)}</p>
          ${
            a.flag
              ? `<div class="row jxl-flag-row">
                   <code id="jxl-flag">${escapeHtml(a.flag)}</code>
                   <button id="jxl-copy">Copy</button>
                 </div>`
              : ''
          }
        </section>`;
      // A chrome:// URL cannot be linked — the browser refuses navigation to it
      // from a page — so copy-to-clipboard is the only followable form.
      host.querySelector<HTMLButtonElement>('#jxl-copy')?.addEventListener('click', async (e) => {
        const btn = e.currentTarget as HTMLButtonElement;
        try {
          await navigator.clipboard.writeText(a.flag!);
          btn.textContent = 'Copied';
        } catch {
          // Clipboard needs permission in some contexts; select the text so it
          // can be copied by hand rather than leaving a button that does nothing.
          const code = host.querySelector('#jxl-flag');
          if (code) getSelection()?.selectAllChildren(code);
          btn.textContent = 'Select and copy';
        }
      });
    } catch {
      /* codec probing is best-effort; the page works without it */
    }
  })();

  root.querySelector<HTMLButtonElement>('#landing-start')!.addEventListener('click', actions.onStart);
  root.querySelector<HTMLButtonElement>('#landing-signin')?.addEventListener('click', actions.onSignIn);
  root.querySelector<HTMLButtonElement>('#landing-signout')?.addEventListener('click', actions.onSignOut);
  root.querySelector<HTMLButtonElement>('#landing-board')?.addEventListener('click', actions.onBoard);
  root.querySelector<HTMLAnchorElement>('#landing-admin')?.addEventListener('click', (e) => {
    e.preventDefault();
    actions.onAdmin();
  });
  root.querySelector<HTMLAnchorElement>('#landing-calibrate')!.addEventListener('click', (e) => {
    e.preventDefault();
    actions.onCalibrate();
  });
}
