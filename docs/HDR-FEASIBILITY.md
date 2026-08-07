# HDR in squintly — what exists, what is missing, what it would cost

Assessed 2026-08-06. Nothing here is built; this is the honest inventory before
anybody starts.

> **[note 2026-08-06] The HDR path below assumes PQ 10-bit AVIF/JXL. That is the
> LAB answer, not the web one, and it should be reconsidered before anybody
> builds it.**
>
> `zencodecs` supports **UltraHDR gain maps** — `jpeg-ultrahdr`, decode AND
> encode, via zenjpeg — and the `heic` codec reads "base SDR + gain-map HDR".
> UltraHDR is what HDR images on the web actually are, and it is structurally
> better for this study than PQ for one specific reason: **a gain-map JPEG is a
> valid SDR JPEG.** An SDR display sees the SDR base with no tone-mapping, so
> the "hard-gate on `dynamic_range_high` or measure Chrome's tone mapper"
> problem in §1 below largely dissolves — the fallback is defined by the format
> rather than improvised by the browser.
>
> That also means the corpus does not need a separate HDR source set encoded to
> a separate ladder; it needs gain-map variants of images we already serve, and
> `dynamic_range_high` (already recorded per session, 73% of live responses)
> selects who can see the lift.
>
> Reassess §1–§5 against UltraHDR before spending anything on the PQ path.

## Verdict

**Feasible, and squintly is further along than it looks — but it is a new study
arm, not a corpus swap.** Four of the five pieces exist. The missing one is the
display path, and it is the one that decides whether the measurement means
anything.

## What already exists

| piece | state | evidence |
|---|---|---|
| HDR source material | **76 PQ-encoded `*.hdr.png` sources**, across 22 strata | `/mnt/v/output/imazen-26-hdr-2026-06-14`, named in `imazen26_hdr_2026-06-23_linearlight_MANIFEST.json` |
| HDR-capable display detection | **already captured, every session** | `conditions.ts` records `dynamic_range_high` from `matchMedia('(dynamic-range: high)')`, and it is a column in `responses.tsv` |
| An HDR-aware quality metric | **available** | `fast-ssim2` has an `hdr-pu` feature (PU21-style encoding) — the crate we already score with |
| HDR-capable codecs | **two of our four** | AVIF and JXL carry PQ/HLG at 10-bit. JPEG and WebP are 8-bit SDR and cannot participate |
| Per-session codec gating | **already works** | the codec probe + `supported_codecs` means a session is only served encodings it can decode — the same machinery that makes JXL degrade safely |

## What is missing

**1. The display path is unverified, and it is the load-bearing one.**

Squintly's whole presentation discipline is that the browser must not resample
or reinterpret the stimulus — hard 1:1 device pixels, integer-only magnification,
nearest-neighbour. HDR adds a second axis of the same concern: an HDR image on an
SDR display is *tone-mapped by the browser*, and an observer would then be
judging the browser's tone-mapping operator rather than the encode. That is
exactly the class of defect the 1:1 rule exists to prevent.

So HDR trials must be **hard-gated on `dynamic_range_high`**, not merely
recorded. Serving one to an SDR display would produce data that looks valid and
measures the wrong thing — the worst possible failure mode, and one no
downstream filter can undo.

Unresolved beyond that: whether the two `<img>` layers switch between HDR and
SDR arms without a visible adaptation flash, and whether the neutral-grey
letterbox surround is still neutral in an HDR compositing path. Both need
measuring on real hardware, not reasoning about.

**2. The corpus builder is SDR-only.** `R2_CORPUS` points at
`imazen-26-png-v3` (8-bit PNG) and every encoder call passes 8-bit paths. HDR
means a second source corpus, a second encode path (AVIF/JXL at 10-bit PQ), and
a `bit_depth` / `transfer` field on `EncodingMeta` — which does not exist, and
whose absence would be exactly the silent-confound the missing subsampling field
already is (see CLAUDE.md's chroma note).

**3. Scoring needs the `hdr-pu` feature and a linear-light path.** The existing
`squintly-score` decodes to 8-bit RGB and would clip an HDR encode to SDR before
measuring, producing a confidently wrong number. The zensim HDR features were
themselves rebuilt "linear-light corrected", which suggests this is easy to get
subtly wrong.

**4. Nothing in the schema says a trial was HDR.** `dynamic_range_high`
describes the *display*, not the stimulus. Without a stimulus-side flag an
analyst cannot separate "HDR content on an HDR screen" from "SDR content on an
HDR screen", and those are different measurements.

## Cost, and the recommended shape

The honest smallest useful version is **a separate study arm**, not a widened
corpus:

1. A new study in `src/studies.rs` with `ContentFilter`-style gating on
   `dynamic_range_high` — studies are pre-registration here, so this is the
   right place for the constraint to live.
2. 76 sources × AVIF + JXL × the 17-rung ladder ≈ 2,600 encodings. Small.
3. `EncodingMeta` gains `bit_depth` and `transfer`; the export gains them too.
4. `squintly-score` builds with `hdr-pu` and keeps a linear-light path.
5. Display verification on real HDR hardware **before** any observer sees one.

Step 5 is the gate. Everything else is mechanical; that one decides whether the
arm measures compression or measures Chrome's tone mapper.

## Why this is worth doing anyway

The panel is two people. If either has an HDR display, an HDR arm is one of the
few things squintly could measure that essentially nobody has measured on real
consumer hardware — the literature's HDR work (AIC-HDR2025 and friends) is lab
displays. And `dynamic_range_high` has been recorded on every session since the
beginning, so the answer to "do our observers even have HDR screens?" was
already in `responses.tsv`. Checked 2026-08-06:

- **166 of 229 responses (73%) came from an HDR-capable display.**
- 5 of 8 observer ids report `dynamic_range_high = 1`.
- One observer reports **both** across their responses — they switch devices,
  which is precisely why the gate has to be per-SESSION (conditions are
  re-captured on each visit) and not per-observer.

So the panel is not a barrier. The gate is, and it is cheap: `sessions` already
carries the flag, so a study can filter on it the same way `ContentFilter`
filters on content class.
