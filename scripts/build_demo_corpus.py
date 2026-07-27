#!/usr/bin/env python3
"""Build a coefficient-shaped SplitStore from the imazen-26 corpus.

Squintly consumes an image store through `--coefficient-path` (see
`src/coefficient.rs::FsCoefficient`), which expects:

    <root>/meta/**/*.json          one object per source and per encoding
    <root>/blobs/sources/<sha>.png       source bytes (PNG)
    <root>/blobs/encodings/<id>.<ext>    encoded bytes

This script produces exactly that from `/mnt/v/imazen-26`, so the rating flow
can serve real trials instead of 409-ing on an empty manifest.

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

Licensing: only folders that are public domain or the user's own work are
selected by default, because a public deployment redistributes these bytes.
`--include` can widen that, but read `/mnt/v/imazen-26/PROVENANCE.md` first.

Codec names are the real encoder, never a stand-in ("Never falsify
benchmark/codec names"). They also have to survive
`sampling.rs::codec_browser_family`, which classifies by substring — the names
below are both truthful and correctly classified.

Usage:
    python3 scripts/build_demo_corpus.py --out ~/tmp/squintly-corpus
    python3 scripts/build_demo_corpus.py --out /data/corpus --per-bucket 4
"""

from __future__ import annotations

import argparse
import hashlib
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

# Low-q weighted: 5 rungs at/below 60, 2 above. See module docstring.
DEFAULT_QUALITIES = [15, 30, 45, 60, 80, 92]


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
    try:
        im = Image.open(path)
        im.load()
    except Exception:
        return None
    if im.mode in ("RGBA", "LA", "P"):
        im = im.convert("RGBA")
        bg = Image.new("RGBA", im.size, (255, 255, 255, 255))
        im = Image.alpha_composite(bg, im).convert("RGB")
    elif im.mode != "RGB":
        im = im.convert("RGB")
    return im


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
    args = ap.parse_args()

    if not IMAZEN26.exists():
        return print(f"corpus not found at {IMAZEN26}", file=sys.stderr) or 1

    root: Path = args.out.expanduser()
    if args.clean and root.exists():
        shutil.rmtree(root)
    meta_dir = root / "meta"
    src_dir = root / "blobs" / "sources"
    enc_dir = root / "blobs" / "encodings"
    for d in (meta_dir, src_dir, enc_dir):
        d.mkdir(parents=True, exist_ok=True)

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
                    origin_path=str(path.relative_to(IMAZEN26)),
                )
            )
            encodings.extend(encode_variants(final, im, enc_dir, sha, args.qualities))
            print(f"  {cat:20s} {bucket_name:2s} {im.width:5d}x{im.height:<5d} {sha[:12]} "
                  f"(+{len(args.qualities)*3}~4 encodings)")

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
                "origin": str(IMAZEN26),
                "provenance": str(IMAZEN26 / "PROVENANCE.md"),
                "categories": args.include,
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
