#!/usr/bin/env python3
"""Build a coefficient-shaped SplitStore from the imazen-26 corpus.

Squintly consumes an image store through `--coefficient-path` (see
`src/coefficient.rs::FsCoefficient`), which expects:

    <root>/meta/**/*.json          one object per source and per encoding
    <root>/blobs/sources/<sha>.png       source bytes (PNG)
    <root>/blobs/encodings/<id>.<ext>    encoded bytes

It produces exactly that, so the rating flow serves real trials instead of
409-ing on an empty manifest.

Two sources, `--source r2` (default) and `--source local`:

* **r2** — `codec-corpus/imazen-26-png-v3`, the canonical corpus. 21 numbered
  strata that separate what imazen/squintly#4 actually needs: plots, mobile vs
  web screenshots, AI clipart/illustrations/products, patent scans, manuscript
  text vs illustrations. Dimensions parse out of the filename for every key
  (2639/2639), so the selection is made from a key listing and only the chosen
  origins are downloaded, not 15.5 GiB. `nope/` is a reject bin and is skipped.
* **local** — the `/mnt/v/imazen-26` folders, which lump several of those
  categories together. Kept for offline work.

Selection rules this encodes (all from CLAUDE.md / squintly's own CLAUDE.md):

* **All four size buckets.** `export.rs::size_bucket` is S(<=256) / M(<=768) /
  L(<=2048) / XL(>2048) on the max dimension; a corpus that misses a bucket
  silently removes a whole stratum from the study.
* **Low-q weighted.** "Codec quality sweeps MUST cover q5-q60 with the same
  density as q60-q100" — web compression lives at aggressive settings, so the
  default ladder puts most of its rungs below q60.
* **Non-photo weighted.** imazen/squintly#4 needs screenshots, rendered text,
  line-art and charts, not just photographs; those categories are where
  SSIMULACRA2 is least validated.

Licensing for imazen-26 is settled and documented with the corpus itself
(`PROVENANCE.md` + per-folder files). This script only maps each stratum onto a
policy id in `src/licensing.rs` so the trial badge names the right terms.

Codec names are the real encoder, never a stand-in ("Never falsify
benchmark/codec names"). They also have to survive
`sampling.rs::codec_browser_family`, which classifies by substring — the names
below are both truthful and correctly classified.

Usage:
    python3 scripts/build_demo_corpus.py --out demo-corpus              # r2
    python3 scripts/build_demo_corpus.py --out demo-corpus --per-stratum 2
    python3 scripts/build_demo_corpus.py --out demo-corpus --source local
Then publish it: `just publish-corpus <version>` (scripts/publish_corpus_r2.py).
"""

from __future__ import annotations

import argparse
import collections
import os
import tempfile
import io
import csv
import hashlib
import importlib.util
import re
import json
import shutil
import subprocess
import sys
from dataclasses import dataclass, asdict
from pathlib import Path

try:
    from PIL import Image
except ImportError:
    sys.exit("Pillow is required: pip install Pillow")

IMAZEN26 = Path("/mnt/v/imazen-26")

# category -> (glob, license_id, is_photo). license_id must exist in
# src/licensing.rs or it degrades to "mixed-research" in the UI and exports.
CATEGORIES: dict[str, tuple[list[str], str, bool]] = {
    # --- non-photo (weighted: this is what imazen/squintly#4 is about) ---
    "office-documents": (["office-documents/**/*.png"], "imazen26-usgov-pd", False),
    "illustration-scans": (["internet-archive-scans/**/*.png"], "imazen26-public-domain", False),
    "icons": (["apancik-public-domain-icons.png"], "imazen26-public-domain", False),
    "screen-ui": (["screen/**/*.png"], "imazen26-screenshots", False),
    # --- photo / mixed ---
    "nasa": (["nasa/**/*.png"], "imazen26-usgov-pd", True),
    "noaa": (["noaa/**/*.png"], "imazen26-usgov-pd", True),
    "parks": (["national-park-service/**/*.png"], "imazen26-usgov-pd", True),
    # LoC ships JPEG + TIFF, no PNG — a single-extension glob silently yielded
    # nothing here, which is why every category takes a list of patterns.
    "loc": (
        [
            "library-of-congress-public-domain/**/*.jpg",
            "library-of-congress-public-domain/**/*.png",
        ],
        "imazen26-public-domain",
        True,
    ),
    "photo-own": (["lilith/**/*.jpg"], "imazen26-owned", True),
}

# Public-by-default set: public domain or the operator's own photographs.
# "screen-ui" is excluded by default — they are screenshots of third-party
# sites; fine for local research, a redistribution question in production.
DEFAULT_CATEGORIES = [
    "office-documents",
    "illustration-scans",
    "icons",
    "nasa",
    "noaa",
    "parks",
    "loc",
    "photo-own",
]

# (bucket name, target max dimension). One representative size per bucket so
# every stratum in export.rs::size_bucket is populated.
BUCKETS: list[tuple[str, int]] = [
    ("S", 240),
    ("M", 640),
    ("L", 1600),
    ("XL", 2400),
]

# Spaced by MEASURED perceptual distance, not by quality units.
#
# The old ladder was [15, 30, 45, 60, 80, 92] — even-ish in q, wildly uneven in
# what an observer sees. Measured on all 4032 encodings of imazen26-v5-test-noai
# (benchmarks/ssim2_imazen26-v5-test-noai_2026-08-06.meta), the median ssim2 gap
# between adjacent rungs was:
#
#     15->30  17.3     45->60   5.7     80->92   6.2
#     30->45   7.9     60->80   8.0
#
# And on 84 live human comparisons, agreement with ssim2's ordering hit 100% by
# a **5-point** gap. So every adjacent pair the old ladder could build was at or
# past the point where the answer stops being in doubt: the study was asking
# questions nobody could get wrong. That is what made rho/ceiling come out above
# 1 — see docs/OBSERVER-FEEDBACK.md §8.
#
# These 17 rungs were chosen by interpolating that measured q->ssim2 curve so no
# adjacent gap exceeds ~5 points, with the largest now 4.9 (was 17.3). Two
# further properties are deliberate:
#
#   - **8 rungs below q60 and 8 above.** The low-q half is where commercial web
#     compression actually operates, and the project rule is that a sweep covers
#     q5-q60 as densely as q60-q100. The old ladder's 15->30 jump was the single
#     worst offender in the whole grid.
#   - **It reaches q100.** The old ladder stopped at 92, where the pooled median
#     is ssim2 86.3 — so the corpus never entered the near-lossless band (90+)
#     at all, and any staircase searching for a notice threshold exited censored
#     at the ceiling with nothing recording that it had. That is
#     imazen/squintly#8 row 5. q100 lands around ssim2 90.6, which finally puts
#     the boundary inside the grid rather than beyond it.
#
# Cost: 17 x 4 codecs x 168 sources = 11,424 encodings, up from 4,032. Rung
# count does NOT worsen the ASAP cold-start problem, which counts observations
# per (source, codec) cell and so is unaffected by how many rungs a cell holds.
DEFAULT_QUALITIES = [15, 18, 22, 26, 30, 38, 45, 52, 60, 68, 75, 82, 88, 92, 95, 97, 100]

# ---------------------------------------------------------------------------
# Canonical corpus on R2 (`codec-corpus/imazen-26-png-v3`).
#
# Preferred over the local /mnt/v/imazen-26 folders: it is stratified into 21
# numbered categories that separate exactly what imazen/squintly#4 needs —
# plots, mobile vs web screenshots, AI-generated imagery, patent scans,
# manuscript text vs illustrations — which the local layout lumps together.
#
# Filenames carry the dimensions (`..._<W>x<H>.sdr.png`, 2639/2639 keys parse),
# so the selection is made from a key listing and only the chosen files are
# downloaded rather than the full 15.5 GiB.
# ---------------------------------------------------------------------------

R2_CORPUS = "r2:codec-corpus/imazen-26-png-v3"

# `nope/` is a reject bin, not a stratum.
# Strata kept out of the study corpus entirely.
#
# The `ai-*` strata are generated images. They are excluded because the study is
# measuring how compression artefacts look on real web content, and a diffusion
# model's output is not that: it is already smooth in ways a camera or a scan
# never is, carries no sensor noise or scan grain for a codec to spend bits on,
# and has its own synthesis artefacts that an observer can mistake for
# compression. A judgement about "which is closer to the original" is still
# well-defined on them — but the answer generalises to other generated images,
# not to the photographs, scans, screenshots and documents the metric will
# actually be pointed at.
#
# Removing them costs no coverage: ten non-photo strata remain (renders,
# brochures, EPA/NOAA documents, patent scans, manuscript illustrations and
# text, plots, mobile and web screenshots).
R2_EXCLUDE_STRATA = {
    "nope",
    "9000-lilith-ai-clipart",
    "9094-lilith-ai-illustrations",
    "9226-lilith-ai-products",
}

# stratum prefix -> (license_id, is_photo). Licensing for imazen-26 is settled
# and documented with the corpus itself (PROVENANCE.md); this only picks which
# policy id in src/licensing.rs the trial badge shows.
R2_STRATA: dict[str, tuple[str, bool]] = {
    "1000-lilith-photos-general": ("imazen26-owned", True),
    "1200-lilith-interiors": ("imazen26-owned", True),
    "1400-lilith-nature": ("imazen26-owned", True),
    "1600-lilith-food": ("imazen26-owned", True),
    "2000-unsplash-people": ("unsplash", True),
    "2200-unsplash-renders": ("unsplash", False),
    "2400-unsplash-textures": ("unsplash", True),
    "3000-art-institute-of-chicago-photos": ("imazen26-public-domain", True),
    "3300-met-museum-photos": ("imazen26-public-domain", True),
    "5000-national-park-service-brochures": ("imazen26-usgov-pd", False),
    "5200-epa-climate-impact-2021-report": ("imazen26-usgov-pd", False),
    "5300-noaa-hurricane-documents": ("imazen26-usgov-pd", False),
    "6000-lilith-scans-public-patents": ("imazen26-public-domain", False),
    "6600-ia-scans-manuscript-illustrations": ("imazen26-public-domain", False),
    "6800-ia-scans-manuscript-text": ("imazen26-public-domain", False),
    "7000-lilith-plots": ("imazen26-owned", False),
    "8000-lilith-mobile-screenshots": ("imazen26-screenshots", False),
    "8100-lilith-web-screenshots": ("imazen26-screenshots", False),
    "9000-lilith-ai-clipart": ("imazen26-owned", False),
    "9094-lilith-ai-illustrations": ("imazen26-owned", False),
    "9226-lilith-ai-products": ("imazen26-owned", False),
}

# Strata that get MORE origins than the global --per-stratum.
#
# Category-level inference needs n > 1. With one origin per stratum, "ssim2
# fails on screenshots" cannot be told apart from "ssim2 fails on THIS
# screenshot" — and catching a collapsed or inverted *category* is the whole
# point of imazen/squintly#4. Text-heavy content gets the extra origins because
# that is where a windowed SSIM-family metric is most likely to diverge from a
# human: glyph-edge ringing is highly salient and easily pooled away.
#
# Photographic strata stay at the global count. They are excluded from the
# non-photo study by `content_class` anyway, and quadrupling them would just
# multiply encode time and R2 storage for content the live study never serves.
TEXT_HEAVY_ORIGINS = 4
TEXT_HEAVY = {
    "5000-national-park-service-brochures",
    "5200-epa-climate-impact-2021-report",
    "5300-noaa-hurricane-documents",
    "6000-lilith-scans-public-patents",
    "6800-ia-scans-manuscript-text",
    "7000-lilith-plots",
    "8000-lilith-mobile-screenshots",
    "8100-lilith-web-screenshots",
}

DIMS_RE = re.compile(r"_(\d{2,5})x(\d{2,5})\.")

# ---- train / val / test -----------------------------------------------------
#
# imazen-26 has a canonical split and this builder ignored it entirely, so the
# study served whatever the pixel ranking happened to surface: measured
# 2026-08-04 on imazen-26-png-v3, the picked set was 60% TRAIN / 29% val / 11%
# test. Collecting human judgements on training images and then scoring a metric
# against them is exactly the leak the split exists to prevent — the metric has
# already seen those pictures.
#
# The rule is by ORIGIN (last digit of the leading numeric stem) and every
# rendition inherits its origin's bucket, so a size ladder built from one origin
# cannot straddle the split.
#
# IMPORTED, never re-implemented: `zensim/docs/DATA_SPLITS.md` §2a names
# `origin_split.py::split_of` the single source of truth. A vendored copy would
# drift silently, and a drifted split is indistinguishable from a correct one
# until the results are already wrong — so a missing canonical file is a hard
# failure here, not a fallback.
SPLIT_MODULE = Path.home() / "work/zen/zenmetrics/scripts/picker/origin_split.py"

# Recorded split labels, one row per origin: `stem  split  ...  original_path`.
# The rule is derivable, but a derivation that silently disagrees with the
# labels the rest of the pipeline was built against is worse than no rule at
# all — every downstream number would be filed under the wrong bucket and
# nothing would say so. So each pick is cross-checked against its recorded
# label, and a single disagreement stops the build.
#
# Verified 2026-08-05: `split_of` reproduces all 2,157 labels exactly
# (1082 train / 657 val / 418 test).
SPLIT_LABELS = Path("/mnt/v/output/imazen-26-features/imazen26_split_evenodd.tsv")


def load_split_labels() -> dict[str, str]:
    """origin stem -> recorded split. Empty when the label file is not present."""
    if not SPLIT_LABELS.exists():
        return {}
    out: dict[str, str] = {}
    with SPLIT_LABELS.open(encoding="utf-8") as fh:
        rd = csv.DictReader(fh, delimiter="\t")
        for row in rd:
            stem = (row.get("stem") or "").strip()
            split = (row.get("split") or "").strip()
            if stem and split:
                out[stem] = split
    return out


def load_split_of():
    if not SPLIT_MODULE.exists():
        sys.exit(
            f"canonical split rule not found at {SPLIT_MODULE}.\n"
            "It is the source of truth (zensim/docs/DATA_SPLITS.md 2a) and must not "
            "be re-implemented here. Check out zenmetrics, or pass --split any to "
            "build a deliberately unsplit corpus."
        )
    spec = importlib.util.spec_from_file_location("origin_split", SPLIT_MODULE)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod.split_of


def r2_list_keys(remote: str) -> list[str]:
    r = subprocess.run(
        ["rclone", "lsf", remote, "--files-only", "-R"],
        capture_output=True,
        text=True,
    )
    if r.returncode != 0:
        sys.exit(f"rclone lsf {remote} failed:\n{r.stderr[-1500:]}")
    return [l.strip() for l in r.stdout.splitlines() if l.strip()]


def r2_pick(
    remote: str, per_stratum: int, cache: Path, split: str = "test"
) -> list[tuple[str, Path, str, bool]]:
    """Select per stratum from the key listing, then download only those keys.

    Prefers the *largest* source in each stratum: every size bucket is produced
    by downscaling, so starting from the most pixels keeps the L/XL rungs true
    downsamples rather than upscales of something smaller.
    """
    keys = r2_list_keys(remote)
    print(f"  listed {len(keys)} keys from {remote}")
    split_of = None if split == "any" else load_split_of()
    by_stratum: dict[str, list[tuple[int, str]]] = {}
    unparsed = 0
    wrong_split = 0
    unsplittable = 0
    for k in keys:
        stratum = k.split("/")[0]
        if stratum in R2_EXCLUDE_STRATA:
            continue
        if stratum not in R2_STRATA:
            continue
        if split_of is not None:
            got = split_of(k.rsplit("/", 1)[-1])
            if got is None:
                # No leading numeric stem means the rule cannot place it. Serving
                # it anyway would be assuming it is held out, which is the very
                # thing being checked.
                unsplittable += 1
                continue
            if got != split:
                wrong_split += 1
                continue
        m = DIMS_RE.search(k)
        if not m:
            unparsed += 1
            continue
        pixels = int(m.group(1)) * int(m.group(2))
        by_stratum.setdefault(stratum, []).append((pixels, k))
    if unparsed:
        print(f"  ! {unparsed} keys had no parseable dimensions; skipped")
    if split_of is not None:
        print(
            f"  split={split}: kept {sum(len(v) for v in by_stratum.values())}, "
            f"skipped {wrong_split} in other splits and {unsplittable} with no origin stem"
        )

    # Cross-check the derived split against the recorded label for each row.
    if split_of is not None:
        labels = load_split_labels()
        if not labels:
            print(
                f"  ! no recorded split labels at {SPLIT_LABELS} — proceeding on the "
                f"rule alone. The labels are the check that the rule still agrees "
                f"with what the rest of the pipeline was built against."
            )
        else:
            disagreements = []
            checked = 0
            for keys in by_stratum.values():
                for _, k in keys:
                    name = k.rsplit("/", 1)[-1]
                    stem = re.match(r"^(\d+)", name)
                    if not stem:
                        continue
                    recorded = labels.get(stem.group(1))
                    if recorded is None:
                        continue
                    checked += 1
                    if recorded != split:
                        disagreements.append((name, split, recorded))
            if disagreements:
                sample = "\n  ".join(
                    f"{n}: rule says {g}, label says {w}" for n, g, w in disagreements[:10]
                )
                raise SystemExit(
                    f"{len(disagreements)} picks disagree with their recorded split "
                    f"label:\n  {sample}\nThe labels are authoritative. Do NOT ship a "
                    f"corpus whose split cannot be reproduced from them."
                )
            print(f"  split labels agree on all {checked} checked keys")

    # A stratum that is deliberately excluded is not a silent omission — that
    # distinction is the whole point of the check below, so it has to know
    # about it. (The guard fired correctly the first time the ai-* strata were
    # excluded while still listed here; this is the fix, not a relaxation.)
    missing = sorted(set(R2_STRATA) - set(by_stratum) - R2_EXCLUDE_STRATA)
    if missing:
        raise SystemExit(
            f"strata matched nothing in {remote}: {missing}. Fix R2_STRATA rather "
            f"than shipping a corpus that silently omits them."
        )

    picks: list[tuple[str, Path, str, bool]] = []
    cache.mkdir(parents=True, exist_ok=True)
    for stratum in sorted(by_stratum):
        license_id, is_photo = R2_STRATA[stratum]
        want = TEXT_HEAVY_ORIGINS if stratum in TEXT_HEAVY else max(1, per_stratum)
        # Sort by pixels desc, then key for determinism.
        ranked = sorted(by_stratum[stratum], key=lambda t: (-t[0], t[1]))
        # The comment here used to say "spread the picks" while the code took
        # the top N — fine at N=1, but at N=4 the four largest files in a
        # stratum are often near-duplicates (consecutive pages of one document,
        # frames of one capture), which would buy breadth in name only.
        #
        # Spread evenly across the largest quarter instead: every pick is still
        # big enough that the XL rung is a true downsample rather than an
        # upscale, and the content actually varies.
        if len(ranked) < want:
            # Quietly serving fewer would make the corpus silently unbalanced,
            # and the shortfall would be invisible in every downstream number.
            raise SystemExit(
                f"stratum {stratum} has only {len(ranked)} origins in split "
                f"'{split}' but {want} are wanted. Lower --per-stratum, widen "
                f"the split, or accept the gap explicitly — do not ship a corpus "
                f"that silently under-fills a stratum."
            )
        pool = ranked[: max(want, len(ranked) // 4)]
        if want >= len(pool):
            chosen = pool[:want]
        else:
            step = len(pool) / want
            chosen = [pool[int(i * step)] for i in range(want)]
        for _, key in chosen:
            local = cache / key
            if not local.exists():
                local.parent.mkdir(parents=True, exist_ok=True)
                r = subprocess.run(
                    ["rclone", "copyto", f"{remote}/{key}", str(local)],
                    capture_output=True,
                    text=True,
                )
                if r.returncode != 0:
                    print(f"  ! download failed for {key}: {r.stderr[-200:]}")
                    continue
            picks.append((stratum, local, license_id, is_photo))
        print(f"  {stratum:44s} {len(chosen)} picked of {len(ranked)}")
    return picks


@dataclass
class SourceMeta:
    hash: str
    width: int
    height: int
    size_bytes: int
    corpus: str
    filename: str
    license_id: str
    size_bucket: str
    is_photo: bool
    origin_path: str


@dataclass
class EncodingMeta:
    id: str
    source_hash: str
    codec_name: str
    quality: float
    encoded_size: int


def origin_label(path: Path) -> str:
    """Provenance string that works for both source modes."""
    for root in (IMAZEN26,):
        try:
            return str(path.relative_to(root))
        except ValueError:
            pass
    # R2 cache: report the object key, which is the real provenance.
    parts = path.parts
    if "imazen-26-png-v3" in parts:
        i = parts.index("imazen-26-png-v3")
        return "codec-corpus/" + "/".join(parts[i:])
    return str(path)


def sha256_bytes(b: bytes) -> str:
    return hashlib.sha256(b).hexdigest()


def bucket_of(max_dim: int) -> str:
    if max_dim <= 256:
        return "S"
    if max_dim <= 768:
        return "M"
    if max_dim <= 2048:
        return "L"
    return "XL"


def load_rgb(path: Path) -> Image.Image | None:
    """Decode to 8-bit sRGB, converting through any embedded ICC profile.

    # Why this converts rather than passing the profile through

    Measured 2026-08-06 on the previous build: 16 of 168 sources carried an ICC
    profile, and the encoders DISAGREED about what to do with it — jpegli and
    libavif preserved it, libjpeg-turbo and libwebp dropped it, 272 encodings
    each way. A browser then renders the source through its profile and a
    stripped JPEG as sRGB, so on a wide-gamut source the observer sees a colour
    shift that is not compression and attributes it to the codec. That is a
    pixels bug by this project's rules, not a cosmetic one.

    Two ways to make it consistent: preserve everywhere, or convert once and
    strip. Converting is chosen because it is the one that cannot rot — it
    depends on Pillow alone, not on four encoders each continuing to agree about
    metadata they treat as optional. After this, the source and every encoding
    of it are sRGB with no profile, so they are colorimetrically identical in
    interpretation and any difference an observer sees is compression.

    It also makes the metric honest: neither `squintly-score` nor
    `zenmetrics batch` applies ICC (verified 2026-08-06 — they agree to 0.3
    ssim2 points on an ICC-carrying source, i.e. both read raw samples). With
    profiles converted at build time, "raw samples" and "what the observer sees"
    finally mean the same thing.
    """
    try:
        im = Image.open(path)
        im.load()
    except Exception:
        return None

    icc = im.info.get("icc_profile")
    if icc:
        try:
            from PIL import ImageCms
            src_prof = ImageCms.ImageCmsProfile(io.BytesIO(icc))
            dst_prof = ImageCms.createProfile("sRGB")
            # Perceptual rendering: these are pictures being judged by eye, not
            # colorimetric proofs, and out-of-gamut clipping (the relative-
            # colorimetric default) would itself be a visible artefact that no
            # codec caused.
            im = ImageCms.profileToProfile(
                im,
                src_prof,
                dst_prof,
                renderingIntent=ImageCms.Intent.PERCEPTUAL,
                outputMode="RGBA" if im.mode in ("RGBA", "LA", "P") else "RGB",
            )
        except Exception as e:
            # A broken profile must not silently become "assume sRGB" — that is
            # the same confound with a different cause. Skip the source instead;
            # a missing image is visible in the stratum counts, a mis-coloured
            # one is not.
            print(f"  ICC conversion failed for {path.name}: {e}; skipping source")
            return None
        # profileToProfile ATTACHES the destination profile. Leaving it on would
        # reintroduce the split it just closed — jpegli/libavif would carry an
        # sRGB profile and libjpeg-turbo/libwebp would not. Untagged sRGB is
        # what every browser assumes by default, so stripping is what makes all
        # five files agree.
        im.info.pop("icc_profile", None)

    if im.mode in ("RGBA", "LA", "P"):
        im = im.convert("RGBA")
        bg = Image.new("RGBA", im.size, (255, 255, 255, 255))
        im = Image.alpha_composite(bg, im).convert("RGB")
    elif im.mode != "RGB":
        im = im.convert("RGB")
    return im


# The zencodecs transcoder, built from the zenpipe workspace:
#     cd ~/work/zen/zenpipe && cargo build --release -p zencodecs-cli
# It is a BINARY we invoke, not a crate to link — the same way this script
# already uses `cjpegli` and the way squintly uses `zenmetrics`.
ZENCODECS_BIN = Path("~/work/zen/zenpipe/target/release/zencodecs").expanduser()

# zencodecs' encoders are the ZEN implementations, so its output must be named
# for them. Calling zenjpeg output "libjpeg-turbo" would falsify a codec name,
# which is banned outright — and it would silently change what every published
# number in this study refers to.
ZENCODECS_CODECS: dict[str, tuple[str, str]] = {
    # study codec name -> (zencodecs --format, file extension)
    "zenjpeg": ("jpeg", "jpg"),
    "zenwebp": ("webp", "webp"),
    "zenavif": ("avif", "avif"),
    "zenjxl": ("jxl", "jxl"),
}


def encode_with_zencodecs(
    src_png: Path, out_dir: Path, sha: str, qualities: list[int]
) -> list[EncodingMeta]:
    """Encode via the zencodecs CLI — the zen codec family, consistent metadata.

    Chosen over the Pillow path for two properties Pillow cannot give:

    - **One ICC policy across every codec.** `--metadata color` keeps colour and
      rotation and drops the rest, for all four formats. Verified 2026-08-06 on a
      Display P3 source: zencodecs emits the profile in JPEG, WebP AND AVIF
      (`acsp` present; AVIF `colr` box of type `prof`), where Pillow silently
      dropped it from JPEG and WebP. That per-encoder disagreement is the bug
      `load_rgb` currently has to pre-empt by converting to sRGB and stripping.
    - **JXL**, which Pillow cannot write at all, so the near-lossless band stops
      depending on a separately-installed `cjxl`.

    NOTE this changes WHICH CODECS the study measures — zenjpeg/zenwebp/zenavif
    are not libjpeg-turbo/libwebp/libavif. That is a change to the experiment,
    not a refactor, which is why it is opt-in via `--encoder zencodecs` rather
    than a silent replacement.
    """
    if not ZENCODECS_BIN.exists():
        sys.exit(
            f"--encoder zencodecs needs {ZENCODECS_BIN}\n"
            "Build it:  cd ~/work/zen/zenpipe && cargo build --release -p zencodecs-cli"
        )
    out: list[EncodingMeta] = []
    out_dir.mkdir(parents=True, exist_ok=True)
    for q in qualities:
        for codec, (fmt, ext) in ZENCODECS_CODECS.items():
            eid = f"{sha[:16]}__{codec}__q{q}"
            p = out_dir / f"{eid}.{ext}"
            r = subprocess.run(
                [
                    str(ZENCODECS_BIN), "convert", str(src_png), str(p),
                    "--format", fmt, "--quality", str(q),
                    # One policy, every codec. See the docstring.
                    "--metadata", "color",
                ],
                capture_output=True,
            )
            if r.returncode == 0 and p.exists():
                out.append(EncodingMeta(eid, sha, codec, float(q), p.stat().st_size))
            else:
                print(f"    ! {codec} q{q} failed rc={r.returncode}: {r.stderr[-160:]!r}")
    return out


# ---------------------------------------------------------------------------
# The canonical path: hand the selected sources to `zenmetrics sweep`.
#
# squintly's real job is stratified SELECTION — which origins, which split,
# which content classes, which size buckets. Encoding and scoring are
# `zenmetrics sweep`'s job and it does them better than this script ever did:
# one pass produces the encoded bytes AND the metric scores, subsampling and
# every other codec knob become an explicit swept axis rather than an accident
# of per-encoder defaults, and the orchestrator brings OOM-safe GPU->CPU
# fallback and fleet distribution.
#
# See CLAUDE.md "ROOT CAUSE: squintly should not build or score corpora at all".
# ---------------------------------------------------------------------------

ZENMETRICS_BIN = Path("~/work/zen/zenmetrics/target/release/zenmetrics").expanduser()

# study codec name -> zenmetrics --codec value. Named for the ACTUAL encoder,
# because calling zenjpeg output "libjpeg-turbo" would falsify a codec name.
SWEEP_CODECS: dict[str, str] = {
    "zenjpeg": "zenjpeg",
    "zenwebp": "zenwebp",
    "zenavif": "zenavif",
    "zenjxl": "zenjxl",
}


def encode_via_sweep(
    src_dir: Path,
    enc_dir: Path,
    qualities: list[int],
    metrics: list[str],
    knob_grid: str | None,
    hdr: bool,
    jobs: int,
) -> tuple[list[EncodingMeta], dict[str, dict[str, float]]]:
    """Run one sweep per codec over the whole source directory.

    Returns `(encodings, scores)` where `scores` maps encoding id -> {metric:
    value}, ready to POST at `/api/admin/metrics` — the scores come free with
    the encode rather than needing a second pass over every blob.

    Sweep works on a DIRECTORY in batch, not per-source, which is why this runs
    after the selection loop rather than inside it.
    """
    def _require_zenmetrics() -> None:
        """Re-checked before EVERY codec, not once at the start.

        A full sweep takes over an hour, and the binary lives in another repo's
        `target/` — exactly what a disk-cleanup sweep deletes. On 2026-08-08 that
        happened mid-run: zenjpeg, zenwebp and zenavif each completed 2856/2856
        cells, then zenjxl died with `unrecognized subcommand 'sweep'` because
        the binary had been replaced underneath us. A start-of-run check cannot
        catch that; the failure has to be loud at the point it happens, or a
        75-minute run quietly yields three codecs out of four.
        """
        if not ZENMETRICS_BIN.exists():
            sys.exit(
                f"\nFATAL: {ZENMETRICS_BIN} is gone.\n"
                "It lives in another repo's target/ and disk-cleanup sweeps delete it.\n"
                "Rebuild:  cd ~/work/zen/zenmetrics && cargo build --release --features sweep"
            )
        probe = subprocess.run(
            [str(ZENMETRICS_BIN), "sweep", "--help"], capture_output=True, text=True
        )
        if probe.returncode != 0 and "unrecognized subcommand" in (probe.stderr or ""):
            sys.exit(
                f"\nFATAL: {ZENMETRICS_BIN} exists but was built WITHOUT `--features sweep`.\n"
                "Rebuild:  cd ~/work/zen/zenmetrics && cargo build --release --features sweep"
            )

    _require_zenmetrics()
    enc_dir.mkdir(parents=True, exist_ok=True)
    out: list[EncodingMeta] = []
    scores: dict[str, dict[str, float]] = {}

    for study_codec, sweep_codec in SWEEP_CODECS.items():
        _require_zenmetrics()  # the binary can vanish between codecs — see above
        # dir= is NOT optional: `/tmp` is banned in this workspace (it is a
        # small tmpfs that gets wiped, and a whole-corpus sweep writes hundreds
        # of MB of encoded cells here). Filling it takes the machine's shell
        # down with ENOSPC, which happened on 2026-08-06.
        scratch = Path("~/tmp").expanduser()
        scratch.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(dir=scratch) as td:
            pareto = Path(td) / "pareto.tsv"
            raw = Path(td) / "enc"
            cmd = [
                str(ZENMETRICS_BIN), "sweep",
                "--codec", sweep_codec,
                "--sources", str(src_dir),
                "--q-grid", ",".join(str(q) for q in qualities),
                "--encoded-out-dir", str(raw),
                "--output", str(pareto),
            ]
            for m in metrics:
                cmd += ["--metric", m]
            # Cap the thread budget. Sweep's default is one rayon thread per
            # logical core, and a whole-corpus AVIF sweep was measured at 17
            # concurrent workers x ~550 MB = ~9 GB with every core saturated —
            # on a box the workspace rules call "frequently shared by concurrent
            # agents", where a runaway job takes the machine down for everyone.
            cmd += ["--jobs", str(jobs)]
            if knob_grid:
                cmd += ["--knob-grid", knob_grid]
            if hdr:
                cmd += ["--hdr"]
            print(f"  sweep {study_codec}: {len(qualities)} q x sources in {src_dir.name}", flush=True)
            r = subprocess.run(cmd, capture_output=True, text=True)
            if r.returncode != 0:
                print(f"    ! sweep {study_codec} rc={r.returncode}: {r.stderr[-400:]}", flush=True)
                continue
            print(f"    {r.stderr.strip().splitlines()[-1] if r.stderr.strip() else 'ok'}", flush=True)

            with pareto.open() as fh:
                for row in csv.DictReader(fh, delimiter="\t"):
                    sha = Path(row["image_path"]).stem
                    q = int(float(row["q"]))
                    eid = f"{sha[:16]}__{study_codec}__q{q}"
                    produced = raw / row["encoded_filename"]
                    if not produced.exists():
                        print(f"    ! {eid}: sweep row has no file {row['encoded_filename']}")
                        continue
                    dst = enc_dir / f"{eid}{produced.suffix}"
                    shutil.copyfile(produced, dst)
                    out.append(
                        EncodingMeta(eid, sha, study_codec, float(q), dst.stat().st_size)
                    )
                    # Every score_* column becomes an ingestable metric. Free:
                    # the sweep already decoded each cell to score it, so this
                    # costs nothing beyond reading the column.
                    got = {
                        k[len("score_"):]: float(v)
                        for k, v in row.items()
                        if k.startswith("score_") and v not in (None, "", "NaN")
                    }
                    if got:
                        scores[eid] = got

    produced = collections.Counter(e.codec_name for e in out)
    missing = [c for c in SWEEP_CODECS if not produced.get(c)]
    if missing:
        sys.exit(
            f"\nFATAL: {', '.join(missing)} produced NO encodings.\n"
            f"Got: {dict(produced)}\n"
            "A corpus missing a whole codec is not publishable — every pair the "
            "sampler builds from it would silently exclude that codec. Fix the "
            "cause and re-run rather than shipping a partial build."
        )
    return out, scores


def encode_variants(
    src_png: Path, im: Image.Image, out_dir: Path, sha: str, qualities: list[int]
) -> list[EncodingMeta]:
    """Encode one source across every codec x quality. Real encoders only."""
    out: list[EncodingMeta] = []
    out_dir.mkdir(parents=True, exist_ok=True)

    for q in qualities:
        # libjpeg-turbo, via Pillow.
        eid = f"{sha[:16]}__libjpeg-turbo__q{q}"
        p = out_dir / f"{eid}.jpg"
        im.save(p, "JPEG", quality=q, optimize=True, progressive=True)
        out.append(EncodingMeta(eid, sha, "libjpeg-turbo", float(q), p.stat().st_size))

        # libwebp, via Pillow.
        eid = f"{sha[:16]}__libwebp__q{q}"
        p = out_dir / f"{eid}.webp"
        im.save(p, "WEBP", quality=q, method=4)
        out.append(EncodingMeta(eid, sha, "libwebp", float(q), p.stat().st_size))

        # libavif, via Pillow (Pillow >= 11.3 ships AVIF support).
        try:
            eid = f"{sha[:16]}__libavif__q{q}"
            p = out_dir / f"{eid}.avif"
            im.save(p, "AVIF", quality=q)
            out.append(EncodingMeta(eid, sha, "libavif", float(q), p.stat().st_size))
        except Exception as e:  # noqa: BLE001
            print(f"    ! libavif q{q} failed ({e}); skipping AVIF for this source")

        # jpegli, via the cjpegli CLI when present. Distinct from
        # libjpeg-turbo: same container, materially different rate-distortion.
        if shutil.which("cjpegli"):
            eid = f"{sha[:16]}__jpegli__q{q}"
            p = out_dir / f"{eid}.jpg"
            r = subprocess.run(
                ["cjpegli", str(src_png), str(p), "-q", str(q)],
                capture_output=True,
            )
            if r.returncode == 0 and p.exists():
                out.append(EncodingMeta(eid, sha, "jpegli", float(q), p.stat().st_size))
            else:
                print(f"    ! cjpegli q{q} failed rc={r.returncode}")
    return out


def pick_sources(categories: list[str], per_bucket: int) -> list[tuple[str, Path, str, bool]]:
    """Deterministically pick files, spreading each category across buckets.

    Returns (category, path, license_id, is_photo). Selection is sorted-order,
    not random, so a rebuild reproduces the same corpus.
    """
    picks: list[tuple[str, Path, str, bool]] = []
    missing: list[str] = []
    for cat in categories:
        globs, license_id, is_photo = CATEGORIES[cat]
        files = sorted({f for g in globs for f in IMAZEN26.glob(g)})
        if not files:
            # Loud, and fatal at the end: a silently-empty category drops a
            # whole content type out of the study without anyone noticing.
            print(f"  !! {cat}: NO FILES matched {globs} — category will be absent")
            missing.append(cat)
            continue
        # Spread picks across the category rather than taking a contiguous run;
        # adjacent files in these folders are often near-duplicates (same
        # document, consecutive pages / frames).
        want = min(per_bucket, len(files))
        step = max(1, len(files) // want)
        chosen = [files[i * step] for i in range(want) if i * step < len(files)]
        for f in chosen:
            picks.append((cat, f, license_id, is_photo))
    if missing:
        raise SystemExit(
            f"categories matched nothing: {missing}. Fix the globs (or drop them "
            f"from --include) rather than shipping a corpus that quietly omits them."
        )
    return picks


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--out", required=True, type=Path, help="SplitStore root to write")
    ap.add_argument(
        "--per-bucket",
        type=int,
        default=1,
        help="sources per (category, size bucket); total = categories x buckets x this",
    )
    ap.add_argument(
        "--include",
        nargs="*",
        default=DEFAULT_CATEGORIES,
        choices=list(CATEGORIES),
        help="categories to draw from (default: public-domain / owned only)",
    )
    ap.add_argument(
        "--qualities",
        type=int,
        nargs="*",
        default=DEFAULT_QUALITIES,
        help="quality ladder; keep it low-q weighted",
    )
    ap.add_argument("--clean", action="store_true", help="wipe --out first")
    ap.add_argument(
        "--encoder",
        choices=["pillow", "zencodecs", "sweep"],
        default="pillow",
        help=(
            "pillow (default): libjpeg-turbo / libwebp / libavif + cjpegli — the "
            "reference implementations the live study measures. zencodecs: the zen "
            "family (zenjpeg/zenwebp/zenavif/zenjxl) with ONE ICC policy across all "
            "of them and JXL support. NOT interchangeable — it changes which codecs "
            "the study is about, so the codec names change with it. sweep: THE "
            "CANONICAL PATH — hand the selected sources to `zenmetrics sweep`, "
            "which encodes AND scores in one pass, makes codec knobs an explicit "
            "swept axis, and supports HDR. See CLAUDE.md's ROOT CAUSE entry."
        ),
    )
    ap.add_argument(
        "--metrics",
        nargs="*",
        default=["ssim2"],
        help="metrics for --encoder sweep to compute per cell (ssim2, butteraugli, dssim, "
             "iwssim-gpu, zensim, cvvdp, ...). They land in <out>/metrics.tsv ready to ingest.",
    )
    ap.add_argument(
        "--knob-grid",
        default=None,
        help=(
            "JSON '{axis: [values]}' passed to `zenmetrics sweep --knob-grid`. This is how "
            "chroma subsampling stops being an accident of per-encoder defaults and becomes a "
            "recorded, swept axis — the confound EncodingMeta cannot currently express."
        ),
    )
    ap.add_argument(
        "--jobs",
        type=int,
        default=max(1, (os.cpu_count() or 8) - 4),
        help=(
            "CPU thread budget for `zenmetrics sweep`. Defaults to nproc-4, leaving room for "
            "other agents — this box is shared and sweep's own default saturates every core "
            "(measured: 17 workers x ~550 MB on an AVIF sweep). 0 = sweep's auto-detect."
        ),
    )
    ap.add_argument(
        "--hdr",
        action="store_true",
        help=(
            "HDR sweep: sources must be 16-bit PQ PNGs (cICP transfer 16). zenjxl only today. "
            "Do NOT use the zencodecs/pillow encoders for HDR — they silently tone-map to SDR."
        ),
    )
    ap.add_argument(
        "--source",
        choices=["r2", "local"],
        default="r2",
        help="r2 (default): the canonical stratified imazen-26-png-v3 on "
             "codec-corpus. local: the /mnt/v/imazen-26 folders.",
    )
    ap.add_argument("--r2-remote", default=R2_CORPUS)
    ap.add_argument(
        "--split",
        default="test",
        choices=["test", "val", "train", "any"],
        help=(
            "Which canonical imazen-26 bucket to draw from. Defaults to TEST: "
            "human judgements collected on training images cannot be used to "
            "score a metric that was fitted on them. 'any' reproduces the old "
            "split-blind behaviour and should be used only deliberately."
        ),
    )
    ap.add_argument(
        "--r2-cache",
        type=Path,
        default=Path("~/tmp/imazen26-v3-cache"),
        help="where downloaded origin files are kept between runs",
    )
    ap.add_argument(
        "--per-stratum",
        type=int,
        default=1,
        help="r2 mode: origin files per stratum (each yields 4 size buckets)",
    )
    args = ap.parse_args()

    if args.source == "local" and not IMAZEN26.exists():
        return print(f"corpus not found at {IMAZEN26}", file=sys.stderr) or 1

    root: Path = args.out.expanduser()
    if args.clean and root.exists():
        shutil.rmtree(root)
    meta_dir = root / "meta"
    src_dir = root / "blobs" / "sources"
    enc_dir = root / "blobs" / "encodings"
    for d in (meta_dir, src_dir, enc_dir):
        d.mkdir(parents=True, exist_ok=True)

    if args.source == "r2":
        picks = r2_pick(args.r2_remote, args.per_stratum, args.r2_cache.expanduser(), args.split)
        print(f"selected {len(picks)} origin files across {len(R2_STRATA)} strata")
    else:
        picks = pick_sources(args.include, args.per_bucket)
        print(f"selected {len(picks)} origin files across {len(args.include)} categories")

    sources: list[SourceMeta] = []
    encodings: list[EncodingMeta] = []
    seen: set[str] = set()

    for cat, path, license_id, is_photo in picks:
        im0 = load_rgb(path)
        if im0 is None:
            print(f"  ! unreadable, skipping: {path}")
            continue
        for bucket_name, target in BUCKETS:
            im = im0.copy()
            longest = max(im.size)
            if longest != target:
                scale = target / longest
                new = (max(1, round(im.width * scale)), max(1, round(im.height * scale)))
                im = im.resize(new, Image.LANCZOS)
            actual_bucket = bucket_of(max(im.size))
            if actual_bucket != bucket_name:
                # Resizing lands on the intended bucket by construction; guard
                # against an off-by-one at a boundary rather than silently
                # mislabelling a stratum.
                print(f"  ! {path.name} -> {im.size} landed in {actual_bucket}, wanted {bucket_name}")

            buf = src_dir / "tmp.png"
            im.save(buf, "PNG", optimize=True)
            data = buf.read_bytes()
            sha = sha256_bytes(data)
            if sha in seen:
                buf.unlink(missing_ok=True)
                continue
            seen.add(sha)
            final = src_dir / f"{sha}.png"
            buf.rename(final)

            sources.append(
                SourceMeta(
                    hash=sha,
                    width=im.width,
                    height=im.height,
                    size_bytes=len(data),
                    corpus=f"imazen26-{cat}",
                    filename=f"{path.stem}__{bucket_name}.png",
                    license_id=license_id,
                    size_bucket=actual_bucket,
                    is_photo=is_photo,
                    origin_path=origin_label(path),
                )
            )
            if args.encoder == "sweep":
                pass  # batched after the loop — sweep takes a directory
            elif args.encoder == "zencodecs":
                encodings.extend(
                    encode_with_zencodecs(final, enc_dir, sha, args.qualities)
                )
            else:
                encodings.extend(
                    encode_variants(final, im, enc_dir, sha, args.qualities)
                )
            print(f"  {cat:20s} {bucket_name:2s} {im.width:5d}x{im.height:<5d} {sha[:12]} "
                  f"(+{len(args.qualities)*3}~4 encodings)")

    if args.encoder == "sweep":
        encs, scores = encode_via_sweep(
            src_dir, enc_dir, args.qualities, args.metrics,
            args.knob_grid, args.hdr, args.jobs,
        )
        encodings.extend(encs)
        if scores:
            # Emitted beside the store rather than posted from here: squintly
            # ingests scores through an admin endpoint, and a corpus builder
            # holding an admin token would be a worse thing than a second step.
            mp = root / "metrics.tsv"
            names = sorted({m for v in scores.values() for m in v})
            with mp.open("w") as fh:
                fh.write("encoding_id\t" + "\t".join(names) + "\n")
                for eid, v in sorted(scores.items()):
                    fh.write(eid + "\t" + "\t".join(
                        ("" if n not in v else f"{v[n]:.6f}") for n in names) + "\n")
            print(f"wrote {len(scores)} scored encodings x {len(names)} metric(s) -> {mp}")
            print(f"  ingest:  curl -X POST '<host>/api/admin/metrics?format=tsv"
                  f"&source=sweep-{'-'.join(names)}&admin_token=...' --data-binary @{mp}")

    # One meta file per object keeps FsCoefficient's walk simple and lets a
    # partial rebuild replace individual entries.
    for s in sources:
        (meta_dir / f"source_{s.hash}.json").write_text(json.dumps(asdict(s), indent=1))
    for e in encodings:
        (meta_dir / f"encoding_{e.id}.json").write_text(json.dumps(asdict(e), indent=1))

    # A manifest sidecar for humans + provenance, mirroring the discipline the
    # export TSVs follow. Not read by FsCoefficient.
    by_bucket: dict[str, int] = {}
    by_corpus: dict[str, int] = {}
    for s in sources:
        by_bucket[s.size_bucket] = by_bucket.get(s.size_bucket, 0) + 1
        by_corpus[s.corpus] = by_corpus.get(s.corpus, 0) + 1
    (root / "_CORPUS.json").write_text(
        json.dumps(
            {
                "source_mode": args.source,
                "origin": args.r2_remote if args.source == "r2" else str(IMAZEN26),
                "provenance": str(IMAZEN26 / "PROVENANCE.md"),
                "categories": sorted(R2_STRATA) if args.source == "r2" else args.include,
                "qualities": args.qualities,
                "sources": len(sources),
                "encodings": len(encodings),
                "by_size_bucket": by_bucket,
                "by_corpus": by_corpus,
                "codecs": sorted({e.codec_name for e in encodings}),
            },
            indent=1,
        )
    )

    total = sum(f.stat().st_size for f in root.rglob("*") if f.is_file())
    print(
        f"\nwrote {len(sources)} sources + {len(encodings)} encodings "
        f"({total/1e6:.1f} MB) -> {root}"
    )
    print(f"size buckets: {by_bucket}")
    missing = [b for b, _ in BUCKETS if b not in by_bucket]
    if missing:
        print(f"WARNING: size buckets with no sources: {missing}")
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

# Note on AVIF: chrome-headless-shell (what Playwright runs) cannot decode
# AVIF, so `libavif` encodings are correctly withheld from e2e sessions by the
# codec probe. That is the browser, not a bug in the probe or the corpus —
# real Chrome/Safari/mobile browsers decode it and will be served AVIF trials.
