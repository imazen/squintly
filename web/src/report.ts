// The disposition report: how close the study is to its answer.
//
// # The one rule this screen exists to enforce
//
// A rank-agreement number is meaningless without a noise ceiling. "ssim2 scored
// 0.7" reads completely differently against a ceiling of 0.95 than against one
// of 0.72 — in the second case the metric is at 97% of what is achievable and
// the observers are simply noisy on this content. So the ceiling is drawn ON
// the same axis as every ρ bar, not relegated to a stat below, and the number
// printed largest is ρ/ceiling rather than ρ.
//
// When there is no ceiling yet, the bars are drawn in a muted colour and the
// headline says so. A report that shows a confident ρ against an unmeasured
// ceiling is worse than one that shows nothing, because it invites exactly the
// conclusion the study is not yet entitled to.
//
// # Charts are inline SVG, no library
//
// Two reasons beyond page weight. The artifact CSP forbids external scripts
// anyway, and — more to the point — these are three small charts whose whole
// value is in details a general-purpose charting library would flatten: the
// ceiling line across the bars, the "not enough data" state that must look
// different from zero, and the coverage bar that has to show what the metric
// was NOT scored on.

import { disposition, metricCatalog, whoami, type Disposition, type MetricAgreement } from './api';

function escapeHtml(s: string): string {
  return s.replace(
    /[&<>"']/g,
    (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]!,
  );
}

const pct = (v: number | null, digits = 0) =>
  v === null ? '—' : `${(v * 100).toFixed(digits)}%`;

/// Chart geometry. One place, so the bars, the ceiling line and the axis cannot
/// disagree about where 100% is.
const CHART = { w: 640, rowH: 34, barH: 18, labelW: 168, padR: 56, padT: 26, padB: 8 };

function plotW(): number {
  return CHART.w - CHART.labelW - CHART.padR;
}

/// x for a 0..1 value.
const xOf = (v: number) => CHART.labelW + Math.max(0, Math.min(1, v)) * plotW();

/**
 * ρ per metric, with the noise ceiling drawn across them.
 *
 * A metric below the minimum sample gets a hatched "not enough yet" bar rather
 * than a zero-length one: an absent measurement and a measured zero look
 * nothing alike in what they license, and the whole DEFAULT-0 incident on the
 * leaderboard came from a chart that could not tell them apart.
 */
function agreementChart(d: Disposition): string {
  const rows = d.metrics;
  if (!rows.length) {
    return `<p class="muted">No metric has enough coverage in this study yet.</p>`;
  }
  const h = CHART.padT + rows.length * CHART.rowH + CHART.padB;
  const ceiling = d.ceiling.ceiling;

  const grid = [0, 0.25, 0.5, 0.75, 1]
    .map(
      (v) => `
      <line x1="${xOf(v)}" y1="${CHART.padT - 6}" x2="${xOf(v)}" y2="${h - CHART.padB}"
            class="chart-grid"/>
      <text x="${xOf(v)}" y="${CHART.padT - 12}" class="chart-axis" text-anchor="middle">
        ${v * 100}%</text>`,
    )
    .join('');

  // 0.5 is chance on a two-alternative forced choice. A metric at 0.5 carries
  // no information about the ordering at all, so it is marked distinctly from
  // the ordinary gridlines — otherwise a bar reaching halfway reads as
  // "reasonable" when it means "coin flip".
  const chance = `
    <line x1="${xOf(0.5)}" y1="${CHART.padT - 6}" x2="${xOf(0.5)}" y2="${h - CHART.padB}"
          class="chart-chance"/>`;

  const ceilingMark =
    ceiling === null
      ? ''
      : `<line x1="${xOf(ceiling)}" y1="${CHART.padT - 6}" x2="${xOf(ceiling)}"
               y2="${h - CHART.padB}" class="chart-ceiling"/>
         <text x="${xOf(ceiling)}" y="${h - CHART.padB + 2}" class="chart-ceiling-label"
               text-anchor="middle">ceiling ${pct(ceiling)}</text>`;

  const bars = rows
    .map((m, i) => {
      const y = CHART.padT + i * CHART.rowH;
      const cy = y + CHART.barH / 2;
      const short = m.metric.length > 22 ? `${m.metric.slice(0, 21)}…` : m.metric;
      const title =
        m.rho === null
          ? `${m.metric}: ${m.comparisons} comparisons — below the minimum to report an agreement`
          : `${m.metric}: agreed on ${m.agreed} of ${m.comparisons} comparisons (${pct(m.rho, 1)})` +
            (m.rho_over_ceiling === null
              ? ' — no ceiling measured yet, so this cannot be read as a score'
              : `, which is ${pct(m.rho_over_ceiling, 1)} of the ceiling`) +
            `. ${m.ties} ties, ${m.uncovered} pairs it could not score.`;

      const bar =
        m.rho === null
          ? `<rect x="${CHART.labelW}" y="${y}" width="${plotW()}" height="${CHART.barH}"
                   class="chart-bar-empty" rx="3"/>
             <text x="${CHART.labelW + 8}" y="${cy + 4}" class="chart-pending">
               ${m.comparisons} so far — not enough to report</text>`
          : `<rect x="${CHART.labelW}" y="${y}" width="${xOf(m.rho) - CHART.labelW}"
                   height="${CHART.barH}" class="chart-bar${ceiling === null ? ' unscaled' : ''}"
                   rx="3"/>
             <text x="${xOf(m.rho) + 6}" y="${cy + 4}" class="chart-value">
               ${m.rho_over_ceiling !== null ? pct(m.rho_over_ceiling) : pct(m.rho)}</text>`;

      return `<g class="chart-row"><title>${escapeHtml(title)}</title>
        <text x="0" y="${cy + 4}" class="chart-label">${escapeHtml(short)}</text>
        ${bar}</g>`;
    })
    .join('');

  return `
    <svg viewBox="0 0 ${CHART.w} ${h}" class="chart" role="img"
         aria-label="Metric agreement against the noise ceiling">
      ${grid}${chance}${bars}${ceilingMark}
    </svg>
    <p class="muted tiny">
      Bars are ρ — how often the metric ordered a pair the way the observer did.
      The label is ${
        ceiling === null
          ? 'ρ itself, because no ceiling has been measured yet'
          : 'ρ ÷ ceiling, which is the figure to report'
      }.
      The dotted line at 50% is chance on a forced choice.
    </p>`;
}

/**
 * How much of the collected data each metric can actually be judged on.
 *
 * Separate from agreement on purpose. A metric with ρ=0.9 on 3% of the pairs
 * has not been evaluated, and one chart showing both numbers as a single bar
 * would let the second fact hide behind the first.
 */
function coverageChart(d: Disposition): string {
  const rows = d.metrics;
  if (!rows.length) return '';
  const h = CHART.padT + rows.length * CHART.rowH + CHART.padB;
  const bars = rows
    .map((m, i) => {
      const total = m.comparisons + m.ties + m.uncovered;
      const y = CHART.padT + i * CHART.rowH;
      const cy = y + CHART.barH / 2;
      const scored = total > 0 ? m.comparisons / total : 0;
      const tied = total > 0 ? m.ties / total : 0;
      const short = m.metric.length > 22 ? `${m.metric.slice(0, 21)}…` : m.metric;
      const wScored = scored * plotW();
      const wTied = tied * plotW();
      return `<g class="chart-row"><title>${escapeHtml(
        `${m.metric}: ${m.comparisons} scored, ${m.ties} ties, ${m.uncovered} with no score for one or both encodings`,
      )}</title>
        <text x="0" y="${cy + 4}" class="chart-label">${escapeHtml(short)}</text>
        <rect x="${CHART.labelW}" y="${y}" width="${plotW()}" height="${CHART.barH}"
              class="chart-bar-empty" rx="3"/>
        <rect x="${CHART.labelW}" y="${y}" width="${wScored}" height="${CHART.barH}"
              class="chart-bar" rx="3"/>
        <rect x="${CHART.labelW + wScored}" y="${y}" width="${wTied}" height="${CHART.barH}"
              class="chart-bar-tie"/>
        <text x="${CHART.labelW + plotW() + 6}" y="${cy + 4}" class="chart-value">
          ${pct(scored)}</text></g>`;
    })
    .join('');
  return `<svg viewBox="0 0 ${CHART.w} ${h}" class="chart" role="img"
               aria-label="How much of the collected data each metric covers">${bars}</svg>
    <p class="muted tiny">Filled = comparisons the metric could score. The lighter
      segment is ties, which are an outcome rather than a miss and are kept out of
      ρ's denominator. The remainder is pairs where one or both encodings have no
      score for that metric.</p>`;
}

/// Progress toward the two pre-registered targets.
function progressChart(d: Disposition): string {
  const w = CHART.w;
  const h = 74;
  const frac = Math.min(1, d.comparisons / Math.max(1, d.ideal_ratings));
  const viable = d.min_viable_ratings / Math.max(1, d.ideal_ratings);
  const met = d.comparisons >= d.min_viable_ratings;
  return `
    <svg viewBox="0 0 ${w} ${h}" class="chart" role="img"
         aria-label="Comparisons collected against the study's targets">
      <rect x="0" y="18" width="${w}" height="22" class="chart-bar-empty" rx="4"/>
      <rect x="0" y="18" width="${frac * w}" height="22"
            class="chart-bar${met ? ' good' : ''}" rx="4"/>
      <line x1="${viable * w}" y1="12" x2="${viable * w}" y2="46" class="chart-ceiling"/>
      <text x="${viable * w}" y="8" class="chart-axis" text-anchor="middle">
        min viable ${d.min_viable_ratings.toLocaleString()}</text>
      <text x="${w}" y="58" class="chart-axis" text-anchor="end">
        ideal ${d.ideal_ratings.toLocaleString()}</text>
      <text x="0" y="58" class="chart-axis">
        ${d.comparisons.toLocaleString()} comparisons · ${d.distinct_pairs.toLocaleString()} distinct pairs
        · ${d.observers} ${d.observers === 1 ? 'observer' : 'observers'}</text>
    </svg>`;
}

/// The headline: what can and cannot be concluded right now.
///
/// Written as a sentence rather than a scoreboard because the honest answer is
/// usually conditional, and a number in a big font invites being quoted without
/// the condition attached.
function verdict(d: Disposition): string {
  const scored = d.metrics.filter((m) => m.rho !== null);
  const ceiling = d.ceiling.ceiling;

  if (!d.metrics.length && !d.unusable.length) {
    return `<p class="verdict warn">No metric scores have been ingested, so only one side
      of the correlation exists. Squintly can say how people ranked these encodings; it
      cannot yet say whether any metric agrees. Ingest a metrics file to change that.</p>`;
  }
  if (ceiling === null) {
    return `<p class="verdict warn">Not enough repeated pairs yet to measure the noise
      ceiling (${d.ceiling.repeat_pairs} so far). Until there is one, no agreement figure
      below can be read as a score — a metric cannot agree with a person more than that
      person agrees with themselves, and we do not yet know what that is.</p>`;
  }
  if (!scored.length) {
    return `<p class="verdict warn">The ceiling is ${pct(ceiling)}, but no metric has
      enough scored comparisons yet to report an agreement against it.</p>`;
  }
  const best = scored[0]!;
  return `<p class="verdict">Observers agree with themselves ${pct(ceiling)} of the time,
    so that is the most any metric could score. The best so far is
    <strong>${escapeHtml(best.metric)}</strong> at ${pct(best.rho, 1)} — which is
    <strong>${pct(best.rho_over_ceiling, 1)} of the ceiling</strong>, over
    ${best.comparisons.toLocaleString()} comparisons. Read that fraction, not the raw ρ.</p>`;
}

function unusableSection(d: Disposition): string {
  if (!d.unusable.length) return '';
  return `<section class="landing-panel">
    <h2>Ingested but not scored</h2>
    <table class="board"><tbody>${d.unusable
      .map(
        (u) =>
          `<tr><td><code>${escapeHtml(u.metric)}</code></td>
           <td class="muted tiny">${escapeHtml(u.reason)}</td></tr>`,
      )
      .join('')}</tbody></table>
  </section>`;
}

function metricRow(m: MetricAgreement): string {
  return `<tr>
    <td><code>${escapeHtml(m.metric)}</code></td>
    <td class="tiny muted">${m.direction === 'lower_is_better' ? 'lower better' : 'higher better'}</td>
    <td>${m.comparisons.toLocaleString()}</td>
    <td>${m.rho === null ? '—' : pct(m.rho, 1)}</td>
    <td>${m.rho_over_ceiling === null ? '—' : pct(m.rho_over_ceiling, 1)}</td>
    <td>${m.ties.toLocaleString()}</td>
    <td>${m.uncovered.toLocaleString()}</td>
  </tr>`;
}

/**
 * Render the report. `onBack` returns to whatever was on screen.
 *
 * Admin status is re-checked here rather than trusted from the caller: this
 * reads how well the metric under test agrees with observers, and an observer
 * who saw that would have been told something about the answer to the question
 * they are being asked.
 */
export async function showReport(root: HTMLElement, onBack: () => void): Promise<void> {
  const me = await whoami().catch(() => null);
  if (!me?.is_admin) {
    root.innerHTML = `
      <div class="screen center" data-screen="report">
        <h1>Not available</h1>
        <p class="muted">This report is for study operators.</p>
        <button id="report-back" class="primary">Back</button>
      </div>`;
    root.querySelector<HTMLButtonElement>('#report-back')!.addEventListener('click', onBack);
    return;
  }

  root.innerHTML = `
    <div class="screen admin" data-screen="report">
      <h1>Disposition</h1>
      <p class="muted">Where the study stands against its own question, and what that
        does and does not license.</p>
      <div id="report-body" class="muted">Loading…</div>
      <div class="row"><button id="report-back" class="primary">Back</button></div>
    </div>`;
  root.querySelector<HTMLButtonElement>('#report-back')!.addEventListener('click', onBack);

  const host = root.querySelector<HTMLElement>('#report-body')!;
  try {
    const [d, catalog] = await Promise.all([disposition(), metricCatalog().catch(() => [])]);
    host.innerHTML = `
      ${verdict(d)}

      <section class="landing-panel">
        <h2>Collection</h2>
        ${progressChart(d)}
        <table class="board"><tbody>
          <tr><td>Noise ceiling (self-agreement on repeats)</td>
            <td>${d.ceiling.ceiling === null ? 'not yet measured' : pct(d.ceiling.ceiling, 1)}
              <span class="muted tiny">${d.ceiling.agreed}/${d.ceiling.repeat_pairs} repeated pairs</span></td></tr>
          <tr><td>Attention checks passed</td>
            <td>${d.golden_pass_rate === null ? '—' : pct(d.golden_pass_rate, 1)}
              <span class="muted tiny">${d.golden_trials} served</span></td></tr>
        </tbody></table>
      </section>

      <section class="landing-panel">
        <h2>Agreement against the ceiling</h2>
        ${agreementChart(d)}
      </section>

      <section class="landing-panel">
        <h2>Coverage</h2>
        ${coverageChart(d)}
      </section>

      ${
        d.metrics.length
          ? `<section class="landing-panel">
        <h2>Numbers</h2>
        <table class="board">
          <thead><tr><th>Metric</th><th>Dir</th><th>Compared</th><th>ρ</th>
            <th title="The reportable figure">ρ/ceiling</th><th>Ties</th>
            <th title="Pairs with no score for one or both encodings">Unscored</th></tr></thead>
          <tbody>${d.metrics.map(metricRow).join('')}</tbody>
        </table>
      </section>`
          : ''
      }

      ${unusableSection(d)}

      <section class="landing-panel">
        <h2>Ingested metrics</h2>
        ${
          catalog.length
            ? `<table class="board">
              <thead><tr><th>Metric</th><th>Encodings</th><th>In this study</th>
                <th>Range</th></tr></thead>
              <tbody>${catalog
                .map(
                  (c) => `<tr><td><code>${escapeHtml(c.metric)}</code>
                    ${c.blurb ? `<div class="muted tiny">${escapeHtml(c.blurb)}</div>` : ''}</td>
                    <td>${c.encodings.toLocaleString()}</td>
                    <td class="${c.covered_encodings === 0 ? 'warn-cell' : ''}">${c.covered_encodings.toLocaleString()}</td>
                    <td class="tiny">${c.min.toPrecision(4)} … ${c.max.toPrecision(4)}</td></tr>`,
                )
                .join('')}</tbody></table>`
            : `<p class="muted">Nothing ingested yet.</p>`
        }
        <p class="muted tiny">Ingest with
          <code>curl -X POST '/api/admin/metrics?source=NAME&amp;format=parquet'
          --data-binary @scores.parquet</code>. TSV and CSV work the same way.
          Any column that is not metadata becomes a metric; a blank cell means
          not measured, never zero.</p>
      </section>`;
  } catch (e) {
    host.innerHTML = `<p class="muted">Couldn't load the report: ${escapeHtml(
      (e as Error).message,
    )}</p>`;
  }
}
