// Asking an observer about work they have already done.
//
// # Where this appears, and why not where you'd expect
//
// Almost nobody clicks "End session" — they answer some trials and shut the
// tab. So the primary moment for this prompt is the NEXT visit: "last time you
// did 21 comparisons — anything we should know?" If somebody does sign off
// deliberately, the same prompt is raised there instead, where it is immediate
// rather than recalled. The server decides which bout is being asked about
// (`src/debrief.rs`), because a bout is a run of answers with no long gap and
// that is only knowable from the response timestamps.
//
// # Circumstances, never a self-rating
//
// Every option here names something that HAPPENED. None asks how well the
// observer did. The difference is which of the two a person actually knows:
// "I didn't realise I could answer can't-tell" is a fact about what they
// understood, and it maps to a concrete analysis. "Rate your attention 1–5" is
// an outcome self-judgement — poorly calibrated, and it invites answering
// whatever seems safest, especially if the observer suspects a low score might
// get their work thrown out.
//
// Which is also why the copy says the work counts either way, and means it:
// `ExclusionPolicy::enabled` is off, and a debrief is a recorded disposition,
// never a delete. Saying otherwise would be false as well as discouraging.

import { fetchPendingDebrief, submitDebrief, type PendingDebrief } from './api';

function escapeHtml(s: string): string {
  return s.replace(
    /[&<>"']/g,
    (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]!,
  );
}

/// "yesterday evening", "on Tuesday" — enough to locate the sitting in memory
/// without pretending to a precision nobody has about their own past.
function whenPhrase(endMs: number): string {
  const d = new Date(endMs);
  const days = Math.floor((Date.now() - endMs) / 86_400_000);
  const hour = d.getHours();
  const partOfDay = hour < 12 ? 'morning' : hour < 18 ? 'afternoon' : 'evening';
  if (days <= 0) return `earlier this ${partOfDay}`;
  if (days === 1) return `yesterday ${partOfDay}`;
  if (days < 7) return `on ${d.toLocaleDateString(undefined, { weekday: 'long' })}`;
  return `on ${d.toLocaleDateString(undefined, { month: 'short', day: 'numeric' })}`;
}

export interface DebriefOptions {
  /// Where it was raised. `end` when the observer clicked End session, `return`
  /// on a later visit. Recorded because they are different measurement
  /// conditions — one is immediate, the other is recall.
  promptedAt: 'end' | 'return';
  /// Called once the observer has answered or skipped. Always called exactly
  /// once, including on failure: a debrief must never be able to strand
  /// somebody between the front page and a trial.
  onDone: () => void;
}

/**
 * Show the debrief if there is a bout to ask about; otherwise call `onDone`
 * immediately.
 *
 * Returns whether it rendered, so a caller can tell "asked" from "nothing to
 * ask" without inspecting the DOM.
 */
export async function maybeShowDebrief(
  root: HTMLElement,
  observerId: string,
  opts: DebriefOptions,
): Promise<boolean> {
  let pending: PendingDebrief | null = null;
  try {
    pending = await fetchPendingDebrief(observerId, opts.promptedAt === 'end');
  } catch {
    // A debrief is a nice-to-have; a trial is not. Any failure here yields to
    // the task rather than blocking it.
    pending = null;
  }
  if (!pending) {
    opts.onDone();
    return false;
  }
  render(root, observerId, pending, opts);
  return true;
}

function render(
  root: HTMLElement,
  observerId: string,
  pending: PendingDebrief,
  opts: DebriefOptions,
): void {
  const { bout, reasons } = pending;
  const n = bout.comparisons || bout.responses;
  const noun = n === 1 ? 'comparison' : 'comparisons';
  const when =
    opts.promptedAt === 'end' ? 'in this sitting' : `${whenPhrase(bout.end_ms)}`;

  root.innerHTML = `
    <div class="screen debrief" data-screen="debrief">
      <h1>${opts.promptedAt === 'end' ? 'Thanks — before you go' : 'Welcome back'}</h1>
      <p class="lede">You did <strong>${n.toLocaleString()} ${noun}</strong> ${escapeHtml(when)}.
        Anything we should know about how that went?</p>
      <p class="muted">
        Tick anything that applies, or none. This is about the circumstances, not
        about how well you did — and your ratings count either way. It just helps
        us read them correctly.</p>

      <fieldset class="debrief-reasons">
        <legend class="sr-only">What happened</legend>
        ${reasons
          .map(
            (r, i) => `
          <label class="debrief-reason">
            <input type="checkbox" value="${escapeHtml(r.key)}" id="reason-${i}" />
            <span>${escapeHtml(r.label)}</span>
          </label>`,
          )
          .join('')}
      </fieldset>

      <label class="debrief-note-label" for="debrief-note">Anything else? (optional)</label>
      <textarea id="debrief-note" class="debrief-note" rows="2"
                placeholder="Only if the list missed something."></textarea>

      <div class="row">
        <button id="debrief-send" class="primary">Send and carry on</button>
        <button id="debrief-skip">Nothing to report</button>
      </div>
    </div>`;

  // Both buttons record something. A skip is a FACT — that this observer was
  // asked about this bout and declined — and without recording it the only
  // evidence would be the absence of a row, which is indistinguishable from
  // never having been asked. They would then face the same question about the
  // same evening on every future visit, which at two participants is an
  // expensive way to annoy half the study.
  // One submission, whatever happens to the buttons. A double-tap on a phone
  // would otherwise write two rows for the same bout and resolve `onDone`
  // twice — and the second row is indistinguishable in the data from somebody
  // genuinely answering twice.
  let sent = false;
  const send = async (skipped: boolean) => {
    if (sent) return;
    sent = true;
    root.querySelectorAll<HTMLButtonElement>('.screen.debrief button').forEach((b) => {
      b.disabled = true;
    });
    const checked = [...root.querySelectorAll<HTMLInputElement>('.debrief-reason input:checked')].map(
      (el) => el.value,
    );
    const note = root.querySelector<HTMLTextAreaElement>('#debrief-note')?.value ?? '';
    try {
      await submitDebrief({
        observer_id: observerId,
        bout_start_ms: bout.start_ms,
        bout_end_ms: bout.end_ms,
        responses: bout.responses,
        reasons: skipped ? [] : checked,
        note: skipped ? null : note,
        skipped,
        prompted_at: opts.promptedAt,
      });
    } catch {
      // Losing a debrief is not worth stranding somebody on this screen.
    }
    opts.onDone();
  };

  root.querySelector<HTMLButtonElement>('#debrief-send')!.addEventListener('click', () => void send(false));
  root.querySelector<HTMLButtonElement>('#debrief-skip')!.addEventListener('click', () => void send(true));
}
