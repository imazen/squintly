# Motivation, Engagement, Corpus, and Compensation

The data is only as good as the rater pool we attract and retain. This document
captures what the literature says works for citizen-science perceptual-rating
platforms and how we apply it to Squintly. Companion to `SPEC.md`,
`docs/participant-grading.md`, and `CHANGELOG.md`.

## Headline findings

1. **Lead with research-impact framing, not social features.** Galaxy Zoo's
   11,000-volunteer survey (Raddick et al. 2013, [arXiv:1303.6886](https://arxiv.org/abs/1303.6886))
   found 39.8% cite "Contribute to original scientific research" as primary
   motivation; *community* came last at 0.2%. The Zooniverse 2021 survey
   (n=6,030, [SSRN 4830179](https://papers.ssrn.com/sol3/papers.cfm?abstract_id=4830179))
   replicated this: top motivations were "helping, interest and curiosity, and
   enjoyment; social engagement and reputation were very rare."

2. **Volunteer-mode data quality beats paid.** AAAI "To Play or Not to Play"
   showed gamified volunteers at 92% accuracy vs. paid at 78% on a complex
   annotation task. The 2024 PLOS One follow-up (Eyal et al., [PMC10013894](https://pmc.ncbi.nlm.nih.gov/articles/PMC10013894/))
   gives Prolific high-quality 67.94% vs. MTurk 26.40%; cost per high-quality
   respondent Prolific $1.90 vs. MTurk $4.36; test-retest Prolific r=0.87 vs.
   MTurk r=0.54.

   **Operational rule: never use MTurk for IQA work in 2026.** Use volunteer
   mode by default; use Prolific for cohort completion only.

3. **90-9-1 participation inequality.** Nielsen's rule applies to citizen
   science: 90% of registered users contribute nothing, 9% infrequently, 1%
   produce most data. Plan recruiting for the long tail; treat the 1% with
   extreme care.

4. **Public raw-count leaderboards poison data.** A 2025 study
   ([ScienceDirect S1041608025000846](https://www.sciencedirect.com/science/article/abs/pii/S1041608025000846))
   found leaderboards improved *quantitative* performance but not qualitative —
   participants got faster, not more accurate. For IQA, "faster" = clicking
   without looking.

## Self-Determination Theory mapping

Ryan & Deci's SDT identifies three needs that drive intrinsic motivation:

| Need | Squintly affordance |
|---|---|
| **Autonomy** | Theme picker; quit anytime; no penalty for skipping a session; data-export and erasure routes available from day one |
| **Competence** | Calibrated skill score (Bayesian, golden-trial-derived); progressive trial difficulty; per-session feedback ("you contributed N rated images") |
| **Relatedness** | "Your trials trained zensim vN" notification; opt-in named contributor wall; named credit in the released metric's notes |

**Deliberately not** building: real-time global chat, community forums (out of
scope for v0.1–v0.3), social-network sign-in.

## Zooniverse playbook, distilled

UX patterns we explicitly adopt:

1. **"Never waste the volunteer's time."** No signup wall before the first
   trial. The UUID in `localStorage` is enough to start contributing.
2. **Tutorial = onboarding test.** First 5–8 trials are calibration, with
   answer feedback after each. Drop below-60% raters from training but keep
   them sampling (soft-recruit later).
3. **Show research outputs to volunteers.** When a metric trained on their
   data ships, push a notification + email with a release-notes link.
4. **Diverse reward structures, all at once.** Foldit explicitly mixes
   short-term (per-trial feedback), long-term (skill score), social (named
   credit, opt-in), individual + the science-impact connection.
5. **Co-adaptation.** Plan to redesign the UX every 2 weeks based on session
   telemetry. The first version will be wrong about onboarding length.

## Streak / milestone / variable-reward UX

10 features keyed to Squintly's existing schema:

| # | Feature | Where it lives | Schema |
|---|---|---|---|
| 1 | **Day streak** ("3 days in a row"). Duolingo data: 7-day streak users 3.6× more engaged. | Trial progress bar + post-session card | `observers.streak_days`, `observers.streak_last_date` |
| 2 | **Streak freeze** (one auto-save per week). Duolingo: -21% churn for at-risk users. | Settings | `observers.freezes_remaining` |
| 3 | **Trial milestones** at 10, 50, 100, 250, 500, 1000. Celebration card. | Inline | aggregate from `responses`; flagged via `observer_milestones` |
| 4 | **Variable mystery image.** Every ~30 trials, surface a "wow" image with a one-line caption between trials (not as a rated stimulus — halo bias risk). | Between-trial card | `corpus_themes.is_wow` flag |
| 5 | **Calibrated skill score** (Bayesian, golden-trial accuracy). Competence feedback. | Profile | `observers.skill_score` |
| 6 | **Theme picker** ("rate nature today"). Autonomy support. | Pre-session | `corpus_themes` table |
| 7 | **Research-impact ticker.** "1,237 raters contributed today; v0.3 ships at 5M trials." | Footer of every screen | aggregate from `sessions` |
| 8 | **Weekly digest email** (opt-in). | Email | `observers.email`, `observers.weekly_digest_optin` |
| 9 | **Named contributor wall** (opt-in pseudonym). | Public page | `observers.display_name` |
| 10 | **"Your data was used" notification.** When a metric trained on the observer's data ships. | Push + email | `metric_releases` + `observer_metric_credit` (v0.3+) |

**Deliberately omitted: raw-trial-count public leaderboards.** They reward
speed and break data quality. If we ever ship a leaderboard, it ranks
*weighted-correct-against-goldens accuracy*, updated daily, not real-time.

## Corpus

### Engaging mix (target ~1,650 source images for v0.2)

| Class | n | Rationale |
|---|---|---|
| Nature / landscape / wildlife | 400 | Universal phone-screen appeal; Wikimedia FP heavy here |
| Macro / food / textile | 200 | High-detail, rewards careful looking |
| Portraits (historical / consenting) | 200 | Faces drive attention |
| Architecture / urban | 200 | Geometric, codec-edge-heavy |
| Art reproductions | 300 | Galaxy-Zoo–style aesthetic pull |
| Screen content / UI / line art | 150 | Codec evaluation; CID22 weakness |
| Astrophotography / scientific | 100 | Direct Galaxy-Zoo pull |
| Documents / charts | 100 | Codec edge case |

At ~50 codec/quality variants per source × pairwise pairs, this gives ~80k
candidate trials — roughly 6 months of throughput at 1k phone-observers ×
100 trials each, well within the 90-9-1 expectation if we recruit broadly.

### Source inventory

| Source | License | Volume | Notes |
|---|---|---|---|
| [Wikimedia Commons Featured Pictures](https://commons.wikimedia.org/wiki/Commons:Featured_pictures) | various CC, attribution required | 11,192 | Top of pyramid; top aesthetic |
| [Wikimedia Commons Quality Images](https://commons.wikimedia.org/wiki/Commons:Quality_images) | various CC | 216,765 | Technical-merit ≥ 2 MP |
| [Met Museum Open Access](https://metmuseum.github.io/) | **CC0** | 406,000 | API: `/public/collection/v1/objects/{id}`, no attribution required |
| [Smithsonian Open Access](https://www.si.edu/openaccess) | **CC0** | 5,100,000 | api.data.gov, AWS Open Data |
| [YFCC100M](https://arxiv.org/abs/1503.01817) | various CC | 99,200,000 | Established IQA precedent (KonIQ-10k, PaQ-2-PiQ) |
| Artvee | public domain | 200,000+ | Curated public-domain art |
| NASA Image Library | public domain | millions | Astronomy / earth |

**Avoid: Unsplash and Pexels.** Both explicitly prohibit API use for ML
training without negotiated permission ([Pexels AI/ML FAQ](https://help.pexels.com/hc/en-us/articles/27292485713945-AI-and-ML-FAQ)).
Squintly trains on judgments-about-images, not the images directly, but the
safe path is sticking to CC0 / CC-BY / public domain.

### Privacy / safety pipeline (mandatory before exposure)

1. **NSFW classifier** at ingest: `Falconsai/nsfw_image_detection` or a
   `siglip2-base-patch16-512` fine-tune. Three-bucket: safe / questionable /
   unsafe. Only "safe" ships.
2. **Face-presence filter** for non-historical sources. Exclude detected faces
   unless the source is "consenting subject" (named public figure on
   Wikimedia, pre-1923 art, Met OA portrait).
3. **License verification** in pipeline: every image carries a machine-readable
   tag (CC0 / CC-BY-4.0 / PD-old / …) before ingestion. Reject ambiguous.
4. **In-app report button** on every trial: one report → quarantine pending
   manual review.

## Compensation modes

Three deployable modes, observer-selectable at signup, configured per-instance.

### Mode A — Volunteer (DEFAULT in v0.2)

No payment. Frame as: "help train an open-source perceptual metric used by
Wikipedia, Imageflow, and the JPEG XL ecosystem." Surface impact in real time.

- **Quality:** highest per-trial. AAAI volunteer-vs-paid +14 pp accuracy.
- **Volume:** lowest per-recruit. Plan for the 90-9-1 split.
- **Cost:** $0/trial direct; UX + outreach only.

### Mode B — Paid (Prolific, NOT MTurk)

Prolific-recruited cohorts at the 2025 floor of £6/hr (~$8/hr); recommended
£9–12/hr. Used for *cohort completion* only — when we need ratings on a
specific corpus subset that the volunteer pipeline hasn't reached.

- **Quality:** Prolific-paid 67.94% high-quality (PLOS 2023). Quality-bonus
  +20% on golden-pass-rate >90%.
- **Volume:** Reliable. ~$0.04/trial quality-adjusted at a 100-trial unit.
- **Hard rule:** never MTurk. The 2024 follow-up (Royal Society Open Science
  2024) confirmed MTurk straight-lining and master-tier degradation persist.

### Mode C — Charity (recommended for v0.3)

Squintly donates $0.02–0.05 per validated trial to the observer's choice of
charity (Wikimedia Foundation, Internet Archive, EFF, Doctors Without
Borders). Caps at $X/observer/month; weekly batch payouts. Removes
self-payment coercion but adds warm-glow + social-proof; the charitable-giving
literature confirms altruism + warm glow are real distinct motivators
(DellaVigna et al. 2012, [NBER w15629](https://eml.berkeley.edu/~sdellavi/wp/CharityQJEFeb12.pdf)).

**Trial weighting in training:** volunteer-mode trials carry the highest trust
weight; charity-mode trials slightly less; paid-mode trials get aggressive
golden screening + bias correction. The weight column in `responses.tsv`
already supports this.

## Account tiers

Anonymous-first, tiered upgrade.

| Tier | Cost to start | What unlocks | Upgrade trigger |
|---|---|---|---|
| **T0 — Anonymous** | UUID in `localStorage` on first trial | Trial submission, theme picker, local streak | Default |
| **T1 — Email** | Magic link | Cross-device sync, weekly digest, public username, restore on phone-loss | Soft prompt after 25 trials |
| **T2 — Verified** | Passkey (WebAuthn) | Eligibility for Prolific/Charity payouts, researcher communications | Required only at first payout |
| **T3 — Researcher** | Application + IRB | Dataset access, attribution on releases | Manual |

**Auth choices, justified:**

- **Magic link** for T1 — minimal friction, no password to forget on a phone.
- **Passkey** for T2 — phones support WebAuthn natively; FIDO 2025 reports
  meaningful adoption. *Don't* use SMS OTP (FBI/CISA 2025 guidance against).
- **No social login as primary** — too privacy-coupled for an anonymous-first
  study.

## Ethics & legal

- **IRB.** Get IRB review even for the volunteer mode. Pro-rated rather than
  completion-bonused payment is preferred to preserve withdrawal rights. NIH
  policy 3014-302 covers compensation review criteria.
- **GDPR.** UUID + IP can become personal data under recital 26. Use:
  - **Legitimate interest** (Art. 6(1)(f)) for trial data, with a Legitimate
    Interest Assessment and clear disclosure.
  - **Explicit consent** for email storage (T1), payout banking info (T2
    paid), any non-strictly-necessary cookie or analytics.
  - **Always-available data-export and deletion route** keyed by UUID.
- **US tax.** $600/year/recipient triggers 1099-MISC. At $0.04/trial ⇒
  15,000 trials/year — vanishingly rare.
- **Children.** Age-gate at 16+ at signup (some EU member states require 16
  for GDPR; US COPPA below 13). Galaxy Zoo's IRB excluded under-18s.
- **Consent UX.** Plain language, surfaced (not buried), covers: what is
  collected, why, who sees it, retention, how to delete, voluntariness, and
  whether compensation applies.

## Quality-vs-engagement tradeoffs (explicit)

| Risk | Mitigation |
|---|---|
| Public raw-count leaderboards reward speed | Show only weighted-against-goldens accuracy; daily updates, not real-time |
| Streaks → forced sessions → fatigue → straight-lining | Cap counted trials at 30/session, soft-warn at 50, hard at 100; jitter golden spacing (8–12, not every 5) |
| Charity-per-trial → grinding low-quality | Pay only validated trials; weekly batch (weakens dopamine link); monthly cap |
| "Wow" images halo-bias next rating | Surface "wow" *between* trials as a celebration screen, never as a rated stimulus |
| Onboarding too long → drop-off; too short → uncalibrated | 5–8 calibration trials with answer feedback; soft-fail observers below 60% but keep them sampling |

## v0.2 / v0.3 / v0.4 plan

**v0.2 ships next:**

1. Anonymous UUID + 5-trial onboarding calibration with answer feedback.
2. Day-streak counter + freeze-saver.
3. Theme picker (3 themes initially: Nature, Art, In-the-wild).
4. Corpus loader for Wikimedia FP/QI + Met OA + Smithsonian OA (CC0/CC-BY).
5. NSFW filter + manual review queue + per-trial report button.
6. Server-side weighted-correct score (computed against converged golden
   posterior; not exposed to UI yet).
7. Volunteer mode only.
8. GDPR consent flow + data-export and erasure routes.
9. Telemetry: time-per-trial, golden pass rate, session length, drop-off
   curve.

**v0.3:**

1. Email magic-link upgrade (T1).
2. Weekly digest + named contributor wall (opt-in).
3. Filtered "weighted accuracy" leaderboard (daily).
4. Charity payout option (Wikimedia, Internet Archive, EFF — caps + weekly
   batch).
5. "Your data trained metric vN" notification.
6. Theme expansion to the full 8 classes from the corpus table above.

**v0.4+:**

1. T2 passkey + Prolific batch integration for cohort completion.
2. A/B framing tests (research-impact vs gamification).
3. Researcher-tier dataset access portal.

## Twelve design implications for Squintly

1. **Lead with "contribute to scientific research."** Don't lead with
   gamification or community.
2. **Anonymous-first; no signup wall before the first trial.** Email after
   ~25 trials.
3. **5–8 calibration trials with answer feedback.** Soft-fail under 60% but
   keep sampling.
4. **Inject goldens at jittered intervals (8–12).** Not every 5 — observers
   learn predictable spacing.
5. **No raw-count leaderboards. Ever.** If a leaderboard ships, it ranks
   weighted-correct accuracy on goldens, daily-updated.
6. **Day-streak with weekly streak-freeze.** Cap counted trials at
   30/session, hard at 100.
7. **Default = volunteer mode.** Charity in v0.3. Prolific only for
   cohort completion. Never MTurk.
8. **Corpus mix biased toward inherently engaging classes.** Nature, art,
   portraits (historical), with ~10% screen/diagram for codec coverage.
9. **CC0 / CC-BY / public domain only.** Wikimedia, Met OA, Smithsonian OA,
   Artvee, NASA. Avoid Unsplash + Pexels.
10. **NSFW + face filter pipeline before any image goes live.** Quarantine
    on user report.
11. **Plan for 90-9-1.** Recruit 10,000 to get 100 deeply-engaged raters.
    Treat the 1% with named credit, weekly digest, "your data trained vN"
    notifications.
12. **GDPR posture: legitimate-interest for trial data, explicit consent for
    email and payouts.** IRB even for volunteer mode. Build data-export and
    erasure routes from day one.

## References

Read in full or substantially:
- [Galaxy Zoo: Motivations of Citizen Scientists (Raddick 2013, arXiv:1303.6886)](https://arxiv.org/abs/1303.6886)
- [Eyal et al. 2023 (PLOS One / PMC10013894)](https://pmc.ncbi.nlm.nih.gov/articles/PMC10013894/)
- [KonIQ-10k methodology (Lin/Hosu/Saupe, arXiv:1803.08489)](https://arxiv.org/abs/1803.08489)

Skimmed:
- [Foldit (Cooper et al. PNAS 2010)](https://www.pnas.org/doi/10.1073/pnas.1115898108)
- [Zooniverse 2021 Survey (SSRN 4830179)](https://papers.ssrn.com/sol3/papers.cfm?abstract_id=4830179)
- [Hossfeld et al. QoE crowdtesting](https://www.keimel.org/publication/hossfeld-tom-2014/Hossfeld-TOM2014.pdf)
- [Mantiuk subjective methods comparison](https://www.cl.cam.ac.uk/~rkm38/pdfs/mantiuk12cfms.pdf)
- [AAAI "To Play or Not to Play"](https://aaai.org/papers/00102-13226-to-play-or-not-to-play-interactions-between-response-quality-and-task-complexity-in-games-and-paid-crowdsourcing/)
- [2025 leaderboard study](https://www.sciencedirect.com/science/article/abs/pii/S1041608025000846)
- [MTurk reliability 2024 (Royal Society Open Science)](https://royalsocietypublishing.org/doi/10.1098/rsos.250361)

Reference docs:
- [Wikimedia Commons Featured Pictures](https://commons.wikimedia.org/wiki/Commons:Featured_pictures)
- [Wikimedia Commons Quality Images](https://commons.wikimedia.org/wiki/Commons:Quality_images)
- [Met Museum Open Access](https://metmuseum.github.io/)
- [Smithsonian Open Access](https://www.si.edu/openaccess)
- [Pexels AI/ML FAQ (prohibits ML training)](https://help.pexels.com/hc/en-us/articles/27292485713945-AI-and-ML-FAQ)
- [Falconsai NSFW classifier](https://huggingface.co/Falconsai/nsfw_image_detection)
- [Prolific pricing](https://researcher-help.prolific.com/en/articles/445230)
- [Self-Determination Theory](https://selfdeterminationtheory.org/theory/)
- [Duolingo streak design](https://www.orizon.co/blog/duolingos-gamification-secrets)
- [Nielsen 90-9-1 rule](https://www.nngroup.com/articles/participation-inequality/)
- [GDPR consent](https://gdpr-info.eu/issues/consent/)
- [NIH IRB compensation policy 3014-302](https://policymanual.nih.gov/3014-302)
