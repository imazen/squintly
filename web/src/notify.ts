// Transient notifications.
//
// One place that knows how a message appears, how long it stays, how it leaves,
// and how somebody dismisses it early. Before this there were three different
// answers in the codebase — the milestone toast, the "can't tell" nudge's
// breathing button, the curator's inline status lines — and each new one was
// re-deciding timing and placement from scratch.
//
// # The constraint that shapes it
//
// This app puts a stimulus under psychovisual judgement on the screen. So a
// notification may NEVER paint over the picture: it pins to the top edge and is
// kept to one short line, overlaying the header band and at most a few pixels
// of the stage's very top — the same latitude the edge frame takes, and far
// outside the region anybody is judging. It is semi-transparent even there.
//
// It lives in its own fixed layer on `document.body`, NOT inside the screen
// that raised it. A notification is raised by an event — an answer landing —
// and the app re-renders `root.innerHTML` immediately after that event, so a
// notice parented to the current screen is destroyed milliseconds after it
// appears. (It was, and the test caught it.) A layer outside the app root
// outlives every re-render, which is also what makes this usable from anywhere
// rather than only from a screen that happens to stay mounted.

export type NoticeTone = 'good' | 'info' | 'warn';

export interface NoticeOptions {
  /// Short, bold, leading. A count, a score, a label — the thing to read first.
  badge?: string;
  /// The sentence.
  text: string;
  tone?: NoticeTone;
  /// How long before it leaves on its own. Dismissable by tap throughout.
  ms?: number;
  /// What raised it, as `data-notice` on the element.
  ///
  /// The layer holds one notice at a time and several unrelated things can
  /// raise it, so "a notice is showing" does not say WHICH. A test that meant
  /// to check the milestone matched the process nudge that had replaced it and
  /// failed on a missing badge — the assertion was right and the locator could
  /// not express what it meant. Identity belongs on the element.
  kind?: string;
}

/// Default dwell. Long enough to read one line, short enough that nobody waits
/// for it — and it can always be tapped away sooner.
export const NOTICE_MS = 2000;

/// How long the fade-out runs. The element is removed after it, not on click,
/// so the exit is visible whether it was tapped or timed out.
const FADE_MS = 400;

/// The fixed layer every notice is rendered into, created on first use.
function noticeLayer(): HTMLElement {
  let layer = document.getElementById('notice-layer');
  if (!layer) {
    layer = document.createElement('div');
    layer.id = 'notice-layer';
    document.body.appendChild(layer);
  }
  return layer;
}

/**
 * Show a notification.
 *
 * Replaces any notification already showing there: two stacked transient
 * messages compete for the same two seconds and neither gets read. The newer
 * one is the one that just happened, so it wins.
 *
 * Returns a dismiss function, so a caller that has its own reason to clear it
 * early (a screen being torn down, a trial being replaced) can do so without
 * reaching into the DOM.
 */
export function notify(opts: NoticeOptions): () => void {
  const layer = noticeLayer();
  layer.replaceChildren();

  // A button, not a div: it is tappable to dismiss, and making that the
  // element's real role means it is focusable and announced rather than being
  // a div a screen reader has to guess at.
  const el = document.createElement('button');
  el.type = 'button';
  el.className = `notice notice-${opts.tone ?? 'info'}`;
  el.setAttribute('aria-live', 'polite');
  if (opts.kind) el.dataset.notice = opts.kind;

  if (opts.badge) {
    const b = document.createElement('span');
    b.className = 'notice-badge';
    b.textContent = opts.badge;
    el.appendChild(b);
  }
  const t = document.createElement('span');
  t.className = 'notice-text';
  // textContent, not innerHTML: a notification carries counts and study names,
  // and there is no reason for this path to be able to render markup at all.
  t.textContent = opts.text;
  el.appendChild(t);

  layer.appendChild(el);

  let gone = false;
  const dismiss = () => {
    if (gone) return;
    gone = true;
    el.classList.add('out');
    window.setTimeout(() => el.remove(), FADE_MS);
  };
  el.addEventListener('click', dismiss);
  window.setTimeout(dismiss, opts.ms ?? NOTICE_MS);
  return dismiss;
}
