// What the task is, before the task starts.
//
// The welcome screen sells the study; this explains the judgement. Those are
// different jobs, and merging them meant the instructions were the part people
// skimmed on the way to the button.
//
// It is shown on every OPEN, including a returning observer's, because the
// thing it protects is criterion consistency: an observer who has drifted on
// what "closer to the original" means contributes answers that look like data
// and are not. Re-reading costs a few seconds; a drifted criterion costs the
// session.
//
// "Open" means a browser session, tracked in sessionStorage — NOT every page
// load. A reload mid-sitting is not someone arriving to do a session, and
// gating it would spend the hold on somebody who read the text four minutes
// ago. The pause menu can reopen it on demand.

/// How long the continue button stays disabled.
///
/// The point is not to make anyone wait — it is that a button which is
/// clickable on arrival gets clicked on arrival, before the text under it has
/// been looked at. A few seconds is enough to break the reflex without being
/// worth resenting, and the countdown says plainly that it is deliberate rather
/// than the page being slow.
export const INSTRUCTIONS_HOLD_MS = 3000;

const SEEN_KEY = 'squintly:instructions_seen';

/// Has this browser session already been shown them?
function seenThisSession(): boolean {
  try {
    return sessionStorage.getItem(SEEN_KEY) === '1';
  } catch {
    // Storage disabled: showing it every load would be a wall, so treat it as
    // seen and leave the menu entry as the way back to it.
    return true;
  }
}

function markSeen(): void {
  try {
    sessionStorage.setItem(SEEN_KEY, '1');
  } catch {
    /* nothing to remember it with; the menu entry still works */
  }
}

/// Show them unless this browser session already has. `force` is the pause
/// menu's route back in, which ignores that.
export async function maybeShowInstructions(
  root: HTMLElement,
  opts: { returning: boolean; force?: boolean },
): Promise<void> {
  if (!opts.force && seenThisSession()) return;
  await showInstructions(root, opts);
  markSeen();
}

export function showInstructions(root: HTMLElement, opts: { returning: boolean }): Promise<void> {
  return new Promise((resolve) => {
    root.innerHTML = `
      <div class="screen center" data-screen="instructions">
        <h1>${opts.returning ? 'Welcome back' : 'What you are being asked'}</h1>
        <div class="instructions">
          <p><b>You will see two versions of the same picture, and the original.</b>
            Your job is to say which version is closer to the original — not which
            one you prefer, and not which one looks nicer on its own.</p>
          <ul>
            <li><b>Compare, do not inspect.</b> Flick between them and watch what
              moves. A difference you can only find by hunting is a difference that
              does not matter.</li>
            <li><b>Look at both before answering.</b> The buttons stay locked until
              you have — that is the point, not a fault.</li>
            <li><b>"Can't tell" is a real answer.</b> If they look the same to you,
              say so. A guess recorded as a preference is worse than a tie.</li>
            <li><b>Magnify when it helps.</b> Pinch, or the mouse wheel. Judge at
              whatever size makes the difference visible.</li>
            <li><b>Sign off when you stop.</b> The menu has an <i>End session</i>
              button. It asks one short question about how the sitting went, which
              is far more accurate answered there than remembered next time.</li>
          </ul>
          <p class="muted">Some comparisons are deliberately easy, and some pictures
            come round twice. Both are checks on the data, not on you.</p>
        </div>
        <div class="row">
          <button id="instructions-go" class="primary" disabled>Reading…</button>
        </div>
      </div>
    `;
    const go = root.querySelector<HTMLButtonElement>('#instructions-go')!;
    const startedAt = Date.now();
    const tick = () => {
      const left = Math.ceil((INSTRUCTIONS_HOLD_MS - (Date.now() - startedAt)) / 1000);
      if (left > 0) {
        go.textContent = `Reading… ${left}`;
        return;
      }
      clearInterval(timer);
      go.disabled = false;
      go.textContent = opts.returning ? 'Continue' : 'Start';
      go.focus();
    };
    const timer = window.setInterval(tick, 200);
    tick();
    go.addEventListener('click', () => {
      if (go.disabled) return;
      clearInterval(timer);
      resolve();
    });
  });
}
