# Squintly glossary

A plain-English glossary of every term, unit, method, and statistic the squintly docs
use — with hyperlinks to the foundational research. Each entry has a one-sentence
"gist" for the layperson, followed by the technical detail for the curious.

> **How to read this file.** If you only have 5 minutes, read [§1 What we're doing and
> why](#1-what-were-doing-and-why) and skip the rest. The other sections are reference
> material — look up terms as you hit them in [README.md](../README.md) /
> [SPEC.md](../SPEC.md) / [STUDY.md](STUDY.md).

---

## 1. What we're doing and why

People look at compressed images on a phone and decide *that one looks fine* or
*that one looks bad*. We want a computer to do the same job, in real time, while a
website is being delivered, so it can pick the smallest version of an image that
still looks fine.

**A "quality metric"** is a program that takes a clean reference image and a compressed
copy, and outputs a number — higher number = looks better to a human. The best public
quality metric in our family is called [zensim](https://github.com/imazen/zensim);
it's been trained on existing public datasets and tops out at about 82 % agreement with
human judgments. The remaining 18 % gap matters: it's the difference between "ship
this version" and "ship a smaller one."

**The squintly bet** is that those existing datasets were collected on lab monitors at
fixed viewing distances, but real humans on real phones see images very differently
(small screen, close viewing distance, sharp eyes can pick up artefacts that don't
matter on a desktop). If we collect new human judgments **with the viewing conditions
recorded for every single rating**, a metric that takes those conditions as input
should close the gap. That's what squintly is built to test, and [`STUDY.md`](STUDY.md)
locks down the experiment.

---

## 2. Core concepts

### Image quality assessment (IQA)

The field of teaching computers to predict how good an image looks to humans.
[Wikipedia overview](https://en.wikipedia.org/wiki/Image_quality).

- **Full-reference (FR)** — you have both the clean *reference* image and the
  *distorted* (e.g. compressed) version, and rate how similar they are. Squintly is
  FR. zensim is FR.
- **No-reference (NR)** — you only have the distorted image and rate it. Useful when
  the original isn't available (a phone photo straight from the camera).
  [PaQ-2-PiQ](https://github.com/baidut/PaQ-2-PiQ) is NR.

### Reference / stimulus / distortion / encoding

- **Reference image** — the clean original we're comparing against.
- **Distortion** — anything that changes the image (most often, lossy compression like
  JPEG / WebP / AVIF / JXL).
- **Encoding** — one specific compressed version of the reference, defined by
  `(codec, quality knob)`.
- **Stimulus** — what we show to a human in one trial. Could be one image (Type S) or
  three (reference + two encodings; Type P).

### Codec

A program that compresses images. The five we care about:

| Codec | What | Notes |
|---|---|---|
| [JPEG](https://en.wikipedia.org/wiki/JPEG) / [mozjpeg](https://github.com/mozilla/mozjpeg) | The 1992 baseline; mozjpeg is a quality-tuned drop-in | Universal support; the "what everything else has to beat" |
| [WebP](https://en.wikipedia.org/wiki/WebP) | Google, 2010 | ~25 % smaller than JPEG at same quality |
| [AVIF](https://en.wikipedia.org/wiki/AVIF) | AV1 still images, 2019 | Excellent quality but slow to encode |
| [JPEG XL (JXL)](https://en.wikipedia.org/wiki/JPEG_XL) | Successor, 2021 | Best quality-per-byte, also fast |
| [JPEG-AI](https://jpeg.org/jpegai/) | Learned (neural-network) codec, in standardisation | What's coming next |

Each codec has a "quality knob" — typically called `q` — between 0 (terrible / very
small) and 100 (perfect / very big). The squintly thresholds `q_notice` / `q_dislike` /
`q_hate` are values of that knob.

### Viewing conditions (the headline variable)

The physical situation under which a human looks at an image. Different conditions →
different things visible.

- **Device pixel ratio (DPR)** — how many physical pixels make up one "CSS pixel" the
  web treats as a unit. Phones are typically DPR 2.0–3.5; desktops 1.0–2.0.
- **Intrinsic-to-device ratio** — for a given displayed image: `intrinsic_pixels /
  device_pixels`. `1.0` = exact pixel-for-pixel. `< 1.0` = browser is upscaling.
  `> 1.0` = browser is downscaling (the common case).
- **Pixels-per-degree (ppd) / cycles-per-degree (cpd)** — how many physical pixels (or
  alternations of light/dark) fit in one degree of the viewer's visual field at their
  viewing distance. A phone at 25 cm gives ~70 ppd; a desktop at 70 cm gives ~30 ppd.
  This is the *actually relevant* spatial-resolution number. See [Ch.11 mobile
  specifics](https://github.com/imazen/zenpapers/blob/main/docs/iqa-methods/reference-book/ch11_mobile_specific.md)
  for the math.
- **Ambient light** (in [lux](https://en.wikipedia.org/wiki/Lux)) — a dim room is
  ~50 lux; a brightly lit office is ~500 lux; outdoors in shade is ~10,000 lux. Bright
  ambient washes out shadow detail and hides banding.
- **Colour gamut** — the range of colours the screen can show. sRGB (the standard), or
  the wider [Display P3](https://en.wikipedia.org/wiki/DCI-P3) (most modern phones), or
  Rec.2020 (HDR-ish). Wide-gamut screens make some chroma artefacts more visible.
- **Dynamic range** — SDR vs HDR. HDR phones can show much brighter highlights, which
  changes how compression artefacts in bright areas look.

### Trial / response / session / observer

- **Trial** — one screen shown to a human, expecting one response.
- **Response** — what they answered (a rating, a preference, or a tap).
- **Session** — one sitting of one human doing trials, typically 5–15 minutes.
- **Observer** — a single person taking part. Equivalent to "participant" or "rater"
  in research literature. In squintly: anonymous, identified by a UUID stored in their
  browser's localStorage.

---

## 3. Trial types & protocols

The two protocols squintly interleaves:

### Type S — single-stimulus with a 4-tier rating ([SPEC.md §"Type S"](../SPEC.md))

One encoding at a time; observer hides/reveals the reference with a tap. Rates on:

| Code | Caption | Plain English |
|---|---|---|
| 1 | Imperceptible | "I can't tell the difference." |
| 2 | I notice | "It's a bit off but I'd still use it." |
| 3 | I dislike | "The artefacts bother me." |
| 4 | I hate | "Unacceptable." |

This is the [Absolute Category Rating (ACR)](https://en.wikipedia.org/wiki/Mean_opinion_score#Absolute_category_rating)
scale, adapted from [ITU-R BT.500](https://www.itu.int/rec/R-REC-BT.500) (the
standardised methodology for subjective TV picture quality).

Type S powers the **staircase** (next entry).

### Staircase (transformed up–down)

An adaptive method that finds the threshold quality where a person *just* starts to
see distortion, *just* starts to dislike it, etc.

Imagine a slider for codec quality. Show the encoded image; if the observer rates
"I can't tell," the next trial uses slightly lower quality. If they rate "I notice,"
the next trial uses slightly higher quality. After several reversals, you've found
the boundary.

[**Levitt 1971**](https://pubmed.ncbi.nlm.nih.gov/5541744/) is the seminal paper. The
*transformed* part means we can target different probability points by changing the
up/down counts:

- **3-down-1-up** (3 "imperceptible" → step down; 1 "noticed" → step up) converges to
  the **79.4 % imperceptible** quality — what we call `q_notice`.
- **2-down-1-up** → 70.7 % → `q_dislike`.
- **1-down-1-up** → 50 % → `q_hate`.

Implemented in [`src/staircase.rs`](../src/staircase.rs).

### Type P — pairwise with ties ([SPEC.md §"Type P"](../SPEC.md))

A triplet: reference image plus two encodings (A and B). Observer answers "A is
closer / they tie / B is closer."

This is **pairwise comparison** with **ties** (the tie option is what's new vs vanilla
pairwise; see [Davidson 1970](https://www.jstor.org/stable/2284099)).

[Plain Triplet Comparison (PTC)](https://arxiv.org/abs/2410.09501) and its boosted
cousin BTC are the canonical AIC-3 implementation of this protocol; squintly uses
PTC (BTC is on the roadmap, amplifier #14).

---

## 4. Scales and units

### JOD — Just Objectionable Difference

The unit of the latent quality scale that pairwise studies (CID22, UPIQ, AIC-3) report
in. **1 JOD ≈ a difference where 75 % of observers prefer the better one.** The scale
is anchored so that JOD = 0 is the reference image; negative JOD = worse than reference.

UPIQ adopts σ = 1.048 for this convention (so 1 JOD = `Φ⁻¹(0.75)·σ ≈ 0.71` units in the
underlying Thurstone scale). See [Mikhailiuk et al. UPIQ
2012.10758](https://arxiv.org/abs/2012.10758) and
[Pérez-Ortiz & Mantiuk pwcmp guide 1712.03686](https://arxiv.org/abs/1712.03686).

### JND — Just Noticeable Difference

Different from JOD. **The smallest distortion most observers can perceive.** The
threshold question "at what quality do most people *first* see something wrong" is the
JND, and matches our `q_notice` threshold. Common in the JND-specific datasets
([MCL-JCI](http://mcl.usc.edu/mcl-jci-dataset/),
[KonJND-1k](https://database.mmsp-kn.de/konjnd-1k-database.html)).

### MOS / DMOS — Mean Opinion Score / Difference MOS

The classic scale: average a bunch of observers' ratings on a 1–5 ACR scale and you
get MOS. Subtract the rating of the reference from the rating of the distorted version
and you get DMOS (so DMOS = 0 means "as good as the reference"). The currency of
[KADID-10k](https://database.mmsp-kn.de/kadid-10k-database.html),
[TID2013](https://www.ponomarenko.info/tid2013.htm),
[CSIQ](https://s2.smu.edu/~eclarson/csiq.html),
[KonIQ-10k](https://database.mmsp-kn.de/koniq-10k-database.html),
[SPAQ](https://github.com/h4nwei/SPAQ).

### Elo

The chess-rating system, reused for IQA. Treat each encoding as a "player"; pairwise
wins update its rating. [CID22](https://cloudinary.com/research) uses an Elo-ish
scheme via the `MCOS` (Mean Combined Opinion Score) column.

### bpp — bits per pixel

`encoded_file_bytes × 8 / total_pixels`. The headline cost number — lower is smaller,
higher is bigger. **CLAUDE.md sweep discipline** insists we cover low-bpp (q5–q40)
just as densely as high-bpp because the low end is where web traffic actually lives.

### cpd / ppd / DPR / ppi

See [§2 viewing conditions](#viewing-conditions-the-headline-variable). Quick:

- **ppi** (pixels per inch) — physical pixel density of the screen.
- **DPR** (device pixel ratio) — software multiplier: 1 CSS px = N device px.
- **ppd** (pixels per degree) — physical pixels per degree of visual angle, given
  viewing distance. **The actually relevant resolution number.**
- **cpd** (cycles per degree) — Nyquist limit is half of ppd; this is the max spatial
  frequency the screen can show at that viewing distance.

### cd/m² and lux

- **cd/m²** (candela per square metre, sometimes "nits") — screen brightness. Modern
  phone peaks ~1000 cd/m²; HDR screens 1000–4000 cd/m²; SDR 80–300 cd/m² in normal use.
- **lux** — ambient room brightness. See [§2 viewing conditions](#viewing-conditions-the-headline-variable).

---

## 5. Statistical methods & models

### Thurstone Case V

The 1927 model that turns pairwise preferences into a latent quality scale.
[Wikipedia](https://en.wikipedia.org/wiki/Law_of_comparative_judgment). Each
"player" (encoding) has an unobserved quality `q`; observers see `q + noise` where
noise is Gaussian with fixed σ. The probability A beats B is `Φ((q_A − q_B) / σ)`.

Used implicitly in JOD scaling — that's where the σ = 1.048 convention comes from.

### Bradley–Terry (BT) model

Same idea as Thurstone but using the logistic distribution instead of Gaussian.
[Bradley & Terry 1952](https://www.jstor.org/stable/2334029). Each encoding has skill
`θ`; probability A beats B is `exp(θ_A) / (exp(θ_A) + exp(θ_B))`. The
[BradleyTerry2 R package](https://cran.r-project.org/package=BradleyTerry2) and
the [`choix` Python package](https://github.com/lucasmaystre/choix) implement it.

### Bradley–Terry–Davidson (with ties)

[Davidson 1970](https://www.jstor.org/stable/2284099) adds a tie option:
`P(tie) = ν · √(exp(θ_A) · exp(θ_B)) / (Σ)`. The extra parameter `ν` controls how
tie-prone observers are. Implemented in [`src/bt.rs`](../src/bt.rs).

### Maximum likelihood estimation (MLE)

The standard way to fit `θ` and `ν` given observed pairwise outcomes: write down the
probability of seeing the data we saw, and pick the `θ` values that make it largest.
[Wikipedia](https://en.wikipedia.org/wiki/Maximum_likelihood_estimation). Numerically
solved with [L-BFGS](https://en.wikipedia.org/wiki/Limited-memory_BFGS) or similar.

### Gaussian prior (pwcmp regularisation)

When two encodings have never been compared (or always one-sided), MLE alone produces
infinite `θ` values. The fix is a soft prior: assume `θ ~ Normal(0, σ_prior²)` before
seeing data. Strength `σ_prior = 1.0` per [Pérez-Ortiz & Mantiuk
pwcmp](https://arxiv.org/abs/1712.03686). This is why a pair seen "5/5 prefer A" is
recorded as ~99 % preference, not 100 %.

### Logistic regression / GAM for threshold inference

To go from a cloud of (quality, rating) pairs to **the function** `q_threshold(c)`,
we fit a [generalised additive model
(GAM)](https://en.wikipedia.org/wiki/Generalized_additive_model):

```
P(rating ≥ k | q, c) = Φ((q − μ_k(c)) / σ_k(c))
```

where `c` is the viewing-condition vector and `μ_k(c)` is a smooth function of `c`
(non-linear, captures interactions). This is the bit that lets the encoder picker
*condition* on the user's screen.

### Active sampling / Expected Information Gain (EIG)

The core human-effort amplifier (amplifier #1 in [README §3](../README.md#3-how-we-amplify-human-effort-15-levers)).
After every batch of responses, look at which *next pair* would maximally shrink the
uncertainty in `θ`. Show that pair next. Implementations:

- [**ASAP**](https://github.com/gfxdisp/asap) (Active Sampling for Pairwise) —
  Mantiuk's group. [Paper](https://arxiv.org/abs/1810.01421).
- [**Hybrid-MST**](https://github.com/jingnantes/hybrid-mst-python) — minimum spanning
  tree + EIG.
- **HR-active** — active version of [HodgeRank](https://arxiv.org/abs/1711.05957).

Squintly's [`src/asap.rs`](../src/asap.rs) is an ASAP-style implementation; status
"impl-not-yet-wired" in README §3.

### Predictive sampling (PS-PC)

Goes one step further than active. An offline classifier looks at every pair and
marks it `predict` (the answer is so obvious we don't need a human) or `defer` (hard,
needs a human). [Mohammadi 2311.03850](https://arxiv.org/abs/2311.03850) reports
8–22 % defer rates at η = 0.97–0.995. Roadmap amplifier #13 in squintly.

### Crowd-BT — per-observer reliability η

Some observers are noisier than others. [Crowd-BT](https://www.cs.cornell.edu/~xchen/papers/icml13crowdbt.pdf)
extends BT with a per-observer reliability parameter η ∈ [0, 1]. Bad observers get
downweighted automatically. Squintly tracks the TODO at
[`src/grading.rs:340-343`](../src/grading.rs).

### Bootstrap confidence interval

Resample the data (with replacement) 1000 times, refit, get 1000 estimates of `θ`,
take the 2.5 % and 97.5 % percentiles → 95 % CI. **Resample by observer**, not by
trial, because errors within an observer are correlated.
[Wikipedia](https://en.wikipedia.org/wiki/Bootstrapping_(statistics)).

### SROCC / PLCC / KRCC

Three different ways of asking "do these two rankings agree?"

- **SROCC** ([Spearman](https://en.wikipedia.org/wiki/Spearman%27s_rank_correlation_coefficient))
  — based purely on rank order. Robust to monotone transforms. The headline IQA
  metric.
- **PLCC** ([Pearson](https://en.wikipedia.org/wiki/Pearson_correlation_coefficient))
  — measures linear agreement *after* fitting a monotone non-linear mapping (the
  [VQEG 5-parameter logistic](https://www.itu.int/dms_pub/itu-t/opb/sup/T-REC-J.Sup4-200312-S!!PDF-E.pdf)).
- **KRCC** ([Kendall τ](https://en.wikipedia.org/wiki/Kendall_rank_correlation_coefficient))
  — counts inversions. Smaller numbers than SROCC, same idea.

All three should be reported per [zenpapers Ch.6 + Ch.7](https://github.com/imazen/zenpapers/blob/main/docs/iqa-methods/reference-book/ch6_dataset_reproductions.md).

### Krasula AUC

A way to test whether one metric is *meaningfully* better than another (not just a
tiny SROCC bump). Classify each pair as "different" vs "similar" by whether the
metrics agree, plot the ROC, take the AUC. [Krasula et al. 2016](https://ieeexplore.ieee.org/document/7498911).

### Bonferroni correction

If you test 5 hypotheses at α = 0.05 each, your overall false-positive rate is much
higher than 5 %. Bonferroni: divide by the number of tests → α_individual = 0.05 / 5
= 0.01. Conservative but simple. [Wikipedia](https://en.wikipedia.org/wiki/Bonferroni_correction).

### Pre-registration

Lock the hypotheses, design, and analysis plan **before** you collect data, so you
can't retro-fit your model to whatever happens to look interesting. Squintly's
preregistration is [`docs/STUDY.md`](STUDY.md).
[Centre for Open Science overview](https://www.cos.io/initiatives/prereg).

---

## 6. Datasets

The reference datasets cited throughout squintly. Linked to their canonical
source + the [zenpapers reference-book chapter](https://github.com/imazen/zenpapers/blob/main/docs/iqa-methods/reference-book/ch6_dataset_reproductions.md)
that gives the full reproduction recipe.

| Name | What | Size | Method |
|---|---|---|---|
| [**CID22**](https://cloudinary.com/labs) | Cloudinary's contribution to the AIC-3 activity (NOT the AIC-3 dataset itself; different methodology) | 250 source images | Pairwise + Elo → MCOS |
| [**AIC-3 BTC/PTC**](https://github.com/jpeg-aic/dataset-BTC-PTC-24) | JPEG AIC-3 study | 5 source images, 600 triplet conditions | [Boosted Triplet Comparison](https://arxiv.org/abs/2410.09501) → JND |
| [**AIC-4 (sample)**](https://github.com/jpeg-aic/JPEG-AIC-4-datasets) | The Call-for-Proposals example dataset | 5 sources, 305 test images | PTC → JND |
| [**JPEG-AI-SDR25**](https://github.com/jpeg-aic/dataset-JPEG-AI-SDR25) | High-fidelity learning-based codec study | 181 images | BTC+PTC, [Jenadeleh 2504.06301](https://arxiv.org/abs/2504.06301) |
| [**AIC-HDR2025**](https://github.com/jpeg-aic/AIC-HDR2025) | HDR fine-grained IQA (not yet released) | 5 HDR sources, 100 compressed | BTC, [Jenadeleh 2506.12505](https://arxiv.org/abs/2506.12505) |
| [**KADID-10k**](https://database.mmsp-kn.de/kadid-10k-database.html) | Konstanz artificially-distorted | 81 refs × ~125 distortions | MOS via Figure-Eight crowdsourcing, [Lin 2019](https://database.mmsp-kn.de/kadid-10k-database.html) |
| [**TID2013**](https://www.ponomarenko.info/tid2013.htm) | Tampere — 524,340 pairwise comparisons | 25 refs × 24 distortions × 5 levels | Swiss-tournament pairwise → MOS, [Ponomarenko et al.](https://www.sciencedirect.com/science/article/abs/pii/S0923596514001490) |
| [**CSIQ**](https://s2.smu.edu/~eclarson/csiq.html) | Categorical SQ | 30 refs × 6 distortions × 4–5 levels | DMOS, [Larson & Chandler 2010](https://www.imageeval.org/) |
| [**KonIQ-10k**](https://database.mmsp-kn.de/koniq-10k-database.html) | In-the-wild authentic distortions | 10,073 images | Crowdsourced MOS, NR |
| [**SPAQ**](https://github.com/h4nwei/SPAQ) | Smartphone Photography Attribute & Quality | 11,125 photos | MOS + attributes, [Fang 2020](https://github.com/h4nwei/SPAQ) |
| [**PaQ-2-PiQ / FLIVE**](https://github.com/baidut/PaQ-2-PiQ) | Patch + global authentic distortions | ~40k images | NR MOS, [Ying 2020](https://arxiv.org/abs/1912.10088) |
| [**LPIPS / BAPPS**](https://github.com/richzhang/PerceptualSimilarity) | Berkeley-Adobe perceptual similarity | 484k human 2AFC judgments | 2AFC + JND, [Zhang 1801.03924](https://arxiv.org/abs/1801.03924) |
| [**PieAPP**](https://github.com/prashnani/PerceptualImageError) | Pairwise-preference learned error | 77,280 pairs over 180 refs | Pairwise preference, [Prashnani 1806.02067](https://arxiv.org/abs/1806.02067) |
| [**KonJND-1k**](https://database.mmsp-kn.de/konjnd-1k-database.html) | Crowdsourced just-noticeable-difference | 1,008 images | Flicker + slider, [Localization paper 2306.07678](https://arxiv.org/abs/2306.07678) |
| [**MCL-JCI**](http://mcl.usc.edu/mcl-jci-dataset/) | Lab JND staircase | 50 images × 30 subjects | Binary-search JND |
| [**UPIQ**](https://www.cl.cam.ac.uk/research/rainbow/projects/upiq/) | Unified Photometric IQ (HDR + SDR on one JOD scale) | 4,159 conditions | Pairwise + rating, [Mikhailiuk 2012.10758](https://arxiv.org/abs/2012.10758) |
| [**LIVE IQA**](https://live.ece.utexas.edu/research/quality/subjective.htm) | The OG, 2006 | 29 refs × ~25 distortions | DMOS with realignment, [Sheikh 2006](https://live.ece.utexas.edu/publications/2006/sheikh_qa_TIP06.pdf) |
| [**KoNViD-1k**](https://database.mmsp-kn.de/konvid-1k-database.html) | In-the-wild video | 1,200 clips | Crowdsourced video MOS |

Local copies of the human-tagged ones are under `/mnt/v/datasets/` per the
[zenpapers dataset inventory](https://github.com/imazen/zenpapers/blob/main/docs/iqa_dataset_sources_2026-05-27.md).

---

## 7. Software & tools

### Scale reconstruction & sampling

- [**pwcmp**](https://github.com/mantiuk/pwcmp) — Mantiuk's Matlab library for
  pairwise comparison scaling with Gaussian prior + bootstrap CIs. The canonical
  reference impl. Paper: [Pérez-Ortiz & Mantiuk
  2017](https://arxiv.org/abs/1712.03686).
- [**ASAP**](https://github.com/gfxdisp/asap) — Active Sampling for Pairwise
  comparisons. The active-sampling reference. Inspired squintly's `src/asap.rs`.
- [**Hybrid-MST**](https://github.com/jingnantes/hybrid-mst-python) — alternative
  active sampler.
- [**Netflix SUREAL**](https://github.com/Netflix/sureal) — Subjective Recovery From
  Erroneous Labels. Models per-subject bias and inconsistency.
  [Pinson 2004.02067](https://arxiv.org/abs/2004.02067).
- [**`choix`**](https://github.com/lucasmaystre/choix) — Python Bradley-Terry MLE,
  including expectation-propagation (used by squintly for posterior covariance).
- [**`crowd-kit`**](https://github.com/Toloka/crowd-kit) — Toloka's library of
  crowdsourcing aggregation methods.
- [**`evalica`**](https://github.com/dustalov/evalica) — Modern BT + Elo +
  Massey-style ratings.

### Metric ensembles

- [**Netflix VMAF**](https://github.com/Netflix/vmaf) — the de-facto video quality
  metric. [Paper](https://netflixtechblog.com/toward-a-practical-perceptual-video-quality-metric-653f208b9652).
- [**SSIMULACRA2** (Cloudinary)](https://github.com/cloudinary/ssimulacra2) — modern
  perceptual metric in XYB space. The fast Rust port:
  [fast-ssim2](https://github.com/imazen/fast-ssim2).
- [**Butteraugli (libjxl)**](https://github.com/libjxl/libjxl/tree/main/lib/jxl/butteraugli) —
  Google's perceptual metric, designed for JPEG XL.
- [**LPIPS**](https://github.com/richzhang/PerceptualSimilarity) — learned deep-feature
  distance. [Paper 1801.03924](https://arxiv.org/abs/1801.03924).
- [**DISTS**](https://github.com/dingkeyan93/DISTS) — structure + texture deep metric.
- [**ColorVideoVDP**](https://github.com/gfxdisp/ColorVideoVDP) — perceptual VDP with
  colour. [Paper 2401.11485](https://arxiv.org/abs/2401.11485).

### Crowdsourcing platforms

- [**Prolific**](https://prolific.com) — research-grade crowdsourcing with deep
  screeners (vision, colour, demographics). Better-quality workers than MTurk in our
  experience; UK-based. See [zenpapers Ch.9](https://github.com/imazen/zenpapers/blob/main/docs/iqa-methods/reference-book/ch9_prolific_self_serve.md).
- [**Amazon MTurk**](https://www.mturk.com/) — the classic; biggest pool, lowest
  screener depth.
- [**Toloka**](https://toloka.ai) — Yandex's platform; strong mobile presence; cheaper
  but lower fluency.
- [**Pavlovia**](https://pavlovia.org/) — psychology-experiment hosting for PsychoPy /
  jsPsych experiments.

### Experiment platforms

- [**jsPsych**](https://github.com/jspsych/jsPsych) — JS library for browser-based
  behavioural experiments.
- [**PsychoPy**](https://github.com/psychopy/psychopy) — Python experiment builder
  (desktop + web via Pavlovia).
- [**AVRate / avrateNG**](https://github.com/Telecommunication-Telemedia-Assessment/avrateNG) —
  domain-specific (audio/video quality) study UIs.
- [**WEST**](https://github.com/NTIA/WEST) — NTIA's video subjective testing tool.
- [**VQone**](https://github.com/mikkonuutinen/VQone) — Matlab tool for designed
  experiments.

### Encoder tooling (cloned to /mnt/v/repos/iqa-tools/)

47 referenced repos shallow-cloned per the [zenpapers Ch.7 software clone
registry](https://github.com/imazen/zenpapers/blob/main/docs/iqa-methods/reference-book/ch7_software_repo_clones.md).
The most directly used:

- [`mantiuk/pwcmp`](https://github.com/mantiuk/pwcmp)
- [`gfxdisp/asap`](https://github.com/gfxdisp/asap)
- [`gfxdisp/ColorVideoVDP`](https://github.com/gfxdisp/ColorVideoVDP)
- [`Netflix/sureal`](https://github.com/Netflix/sureal)
- [`Netflix/vmaf`](https://github.com/Netflix/vmaf)
- [`cloudinary/ssimulacra2`](https://github.com/cloudinary/ssimulacra2)
- [`lucasmaystre/choix`](https://github.com/lucasmaystre/choix)
- [`Toloka/crowd-kit`](https://github.com/Toloka/crowd-kit)

### Squintly's own stack

- [**Rust**](https://www.rust-lang.org/) — backend language.
- [**axum**](https://github.com/tokio-rs/axum) — web framework.
- [**sqlx**](https://github.com/launchbadge/sqlx) — async SQL + compile-time-checked
  queries.
- [**SQLite**](https://www.sqlite.org/) — the database.
- [**Vite**](https://vitejs.dev/) — frontend build tool.
- [**Playwright**](https://playwright.dev/) — e2e browser testing (including the
  Galaxy Z Fold 7 device profiles squintly ships).
- [**Cloudflare R2**](https://www.cloudflare.com/products/r2/) — object storage for
  curator inputs.
- [**Railway**](https://railway.app/) — deploy.
- [**Postmark**](https://postmarkapp.com/) — passwordless magic-link email.

---

## 8. Squintly-specific terms

### Coefficient store

The image store squintly consumes. Lives separately (see
[coefficient](https://github.com/imazen/coefficient) — the repo). Content-addressed
by sha256. Squintly is **read-only** against it (never writes back).

### Curator

The corpus-building workflow inside squintly: stream candidates from R2 or local
manifests, decide include/exclude + size bucket, set per-source threshold q, export
the curated manifest. See [`docs/CORPUS_CURATOR_SPEC.md`](CORPUS_CURATOR_SPEC.md).

### Suggestion

User-submitted candidate images. Goes through a researcher accept/reject workflow
before joining the curator pool. Migration `0008_suggestions.sql`,
[`src/suggestions.rs`](../src/suggestions.rs).

### Tier (engagement)

Squintly's account ladder:

| Tier | Code | What |
|---|---|---|
| Anon | 0 | UUID in localStorage, no account |
| Email | 1 | Verified email address, can do streaks |
| Passkey | 2 | WebAuthn passkey, more secure |
| Researcher | 3 | Internal, full access |

Migrations `0003_engagement.sql` + `0005_auth.sql`.

### Streak / freeze

[Engagement game-loop](motivation-and-compensation.md): consecutive-day participation
streaks. Observers get N freezes/year to skip a day without losing the streak.

### Trusted pool

Observers who have sustained a high `qualifier_score` and `golden_pass_rate` get
`trusted_pool = 1` (`migrations/0002_grading.sql`); their responses get higher weight
in scale reconstruction.

### Session weight A–F

Per [`participant-grading.md`](participant-grading.md): each session gets graded A–F
based on golden-pass rate + straight-line responses + honeypot fails. The grade maps
to a multiplier `∈ {0, 0.5, 1.0, 1.5}` applied at scale-reconstruction time. Implemented
in [`src/grading.rs`](../src/grading.rs).

### Honeypot / golden / trap

Three slightly different things, all about quality control:

- **Golden** — a trial with a known correct answer (typically reference vs
  obvious-distortion). If you get it wrong, that's a strike.
- **Honeypot** — same idea, inserted ~1/30 trials during the main session.
- **Trap** — strict-version golden where two trap-fails end the session.

Same family; the names come from different parts of the methodology literature.

### `intrinsic_to_device_ratio`

The headline-per-trial measurement: of the image actually rendered on screen, how
many intrinsic-image-pixels map to one device-pixel? `< 1` = browser is upscaling
(potentially blurring); `> 1` = downscaling (potentially aliasing); `1.0` =
pixel-perfect. Captured per trial via JS at render time.

### `q_notice` / `q_dislike` / `q_hate`

The three threshold quality values squintly fits, per (source, condition_bucket):

- `q_notice` — quality below which a typical observer notices distortion (79.4 %
  threshold).
- `q_dislike` — quality below which they're bothered (70.7 %).
- `q_hate` — quality below which they call it unacceptable (50 %).

The encoder picker wants `q_threshold(c, k)` — the *function* of viewing conditions
that gives these values for any deployed condition. Defined in
[`SPEC.md` §"Threshold model"](../SPEC.md) + tested in H2 of [`STUDY.md`](STUDY.md).

---

## 9. Where the broader reference lives

This glossary is intentionally **terse and accessible**. Where to go for depth:

- **Reproduction-grade methodology**: [zenpapers reference book](https://github.com/imazen/zenpapers/blob/main/docs/iqa-methods/reference-book/README.md)
  — 11 chapters, ~5,400 lines, every equation traced to a paper, all 47 referenced
  repos cloned to `/mnt/v/repos/iqa-tools/`.
- **Internal methods**: [`docs/methodology.md`](methodology.md) — squintly's own
  detailed methods (sampling, screening, scale-recon).
- **Participant policy**: [`docs/participant-grading.md`](participant-grading.md),
  [`docs/motivation-and-compensation.md`](motivation-and-compensation.md).
- **Curator workflow**: [`docs/CORPUS_CURATOR_SPEC.md`](CORPUS_CURATOR_SPEC.md).
- **Full design**: [`SPEC.md`](../SPEC.md).
- **Pre-registration**: [`docs/STUDY.md`](STUDY.md).
- **Status + amplifiers**: [`README.md`](../README.md).

External canonical references:

- ITU recommendations: [BT.500-15](https://www.itu.int/rec/R-REC-BT.500),
  [P.910](https://www.itu.int/rec/T-REC-P.910), [P.913](https://www.itu.int/rec/T-REC-P.913).
- Pre-registration framework: [Centre for Open Science](https://www.cos.io/initiatives/prereg).
- VQEG (Video Quality Experts Group) [tools](https://github.com/vqeg/software-tools)
  + [Number of Subjects](https://github.com/VQEG/number-of-subjects).
