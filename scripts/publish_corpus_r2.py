#!/usr/bin/env python3
"""Publish a coefficient SplitStore to a public R2 bucket as a static HTTP store.

`HttpCoefficient` (src/coefficient.rs) only ever issues three GETs:

    GET <base>/api/manifest              -> {"sources": [...], "encodings": [...]}
    GET <base>/api/sources/<hash>/image
    GET <base>/api/encodings/<id>/image

Object keys can contain slashes, so a plain bucket can answer all three with no
server at all — upload objects at exactly those keys and point
`SQUINTLY_COEFFICIENT_HTTP` at the bucket's public domain. That removes the
~121 MB build-context upload and 119 MB image layer that baking the corpus
costs on every deploy.

The store lives under a **versioned prefix** (`--prefix`), never the bucket
root: these buckets are shared, and a version in the path means publishing a new
corpus can't mutate what a running study is reading. Roll forward by publishing
a new prefix and changing one env var; roll back by changing it back.

Content types are set explicitly per object. R2 serves `application/octet-stream`
for anything it wasn't told about, and the key ends in `/image` with no
extension, so nothing can be inferred from the name.

Usage:
    python3 scripts/publish_corpus_r2.py \\
        --store demo-corpus \\
        --bucket codec-corpus \\
        --prefix squintly/demo-corpus/imazen26-v1 \\
        --public-base https://codec-corpus.r2.imazen.org
"""

from __future__ import annotations

import argparse
import json
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

# Extension -> content type for encoding blobs. Keep in sync with the encoders
# in build_demo_corpus.py.
CONTENT_TYPES = {
    ".png": "image/png",
    ".jpg": "image/jpeg",
    ".jpeg": "image/jpeg",
    ".webp": "image/webp",
    ".avif": "image/avif",
    ".jxl": "image/jxl",
}


def load_store(store: Path) -> tuple[list[dict], list[dict]]:
    """Read the SplitStore's per-object meta files into manifest arrays."""
    sources, encodings = [], []
    meta = store / "meta"
    if not meta.is_dir():
        sys.exit(f"no meta/ under {store} — is this a SplitStore?")
    for f in sorted(meta.glob("*.json")):
        obj = json.loads(f.read_text())
        if "source_hash" in obj:
            encodings.append(obj)
        else:
            sources.append(obj)
    return sources, encodings


def rclone(args: list[str]) -> None:
    r = subprocess.run(["rclone", *args], capture_output=True, text=True)
    if r.returncode != 0:
        sys.exit(f"rclone {' '.join(args[:3])}… failed:\n{r.stderr[-2000:]}")


def main() -> int:
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    ap.add_argument("--store", required=True, type=Path, help="built SplitStore root")
    ap.add_argument("--bucket", required=True, help="R2 bucket name (rclone remote 'r2')")
    ap.add_argument("--prefix", required=True, help="versioned key prefix inside the bucket")
    ap.add_argument("--public-base", required=True, help="public domain serving the bucket")
    ap.add_argument("--remote", default="r2", help="rclone remote name (default: r2)")
    ap.add_argument("--dry-run", action="store_true")
    args = ap.parse_args()

    store: Path = args.store.expanduser()
    sources, encodings = load_store(store)
    if not sources or not encodings:
        sys.exit(f"store looks empty: {len(sources)} sources, {len(encodings)} encodings")

    # The manifest coefficient's HTTP API would return. parse_manifest_json in
    # src/coefficient.rs accepts `codec_name`/`encoded_size` (what the SplitStore
    # meta files already use), so the objects pass through unchanged.
    manifest = {"sources": sources, "encodings": encodings}

    prefix = args.prefix.strip("/")
    dest_root = f"{args.remote}:{args.bucket}/{prefix}"
    print(f"publishing {len(sources)} sources + {len(encodings)} encodings")
    print(f"  -> {dest_root}")
    print(f"  -> served at {args.public_base.rstrip('/')}/{prefix}")

    if args.dry_run:
        print("(dry run; nothing uploaded)")
        return 0

    # Stage the exact key layout locally, then upload in content-type batches.
    # Staging by hardlink keeps this cheap even for a multi-GB store.
    stage = Path(tempfile.mkdtemp(prefix="squintly-r2-"))
    try:
        api = stage / "api"
        (api).mkdir(parents=True)
        (api / "manifest").write_text(json.dumps(manifest))

        # group -> list of (staged_path). Each group uploads with one content type.
        groups: dict[str, list[Path]] = {}

        for s in sources:
            src = store / "blobs" / "sources" / f"{s['hash']}.png"
            if not src.exists():
                sys.exit(f"missing source blob for {s['hash']}")
            d = api / "sources" / s["hash"]
            d.mkdir(parents=True, exist_ok=True)
            link = d / "image"
            try:
                link.hardlink_to(src)
            except OSError:
                shutil.copy2(src, link)
            groups.setdefault("image/png", []).append(link)

        enc_dir = store / "blobs" / "encodings"
        for e in encodings:
            matches = list(enc_dir.glob(f"{e['id']}.*"))
            if not matches:
                sys.exit(f"missing encoding blob for {e['id']}")
            src = matches[0]
            ctype = CONTENT_TYPES.get(src.suffix.lower())
            if ctype is None:
                sys.exit(f"unknown extension {src.suffix} for {e['id']}")
            d = api / "encodings" / e["id"]
            d.mkdir(parents=True, exist_ok=True)
            link = d / "image"
            try:
                link.hardlink_to(src)
            except OSError:
                shutil.copy2(src, link)
            groups.setdefault(ctype, []).append(link)

        # application/json for the manifest itself.
        rclone([
            "copyto", str(api / "manifest"), f"{dest_root}/api/manifest",
            "--header-upload", "Content-Type: application/json",
            "--s3-no-check-bucket",
        ])
        print("  uploaded api/manifest")

        for ctype, files in sorted(groups.items()):
            listing = stage / f"files-{ctype.replace('/', '-')}.txt"
            listing.write_text("\n".join(str(p.relative_to(stage)) for p in files) + "\n")
            rclone([
                "copy", str(stage), dest_root,
                "--files-from", str(listing),
                "--header-upload", f"Content-Type: {ctype}",
                "--transfers", "16",
                "--s3-no-check-bucket",
            ])
            print(f"  uploaded {len(files):4d} objects as {ctype}")
    finally:
        shutil.rmtree(stage, ignore_errors=True)

    base = f"{args.public_base.rstrip('/')}/{prefix}"
    print("\nverify:")
    print(f"  curl -s {base}/api/manifest | head -c 200")
    print("point the deployment at it:")
    print(f'  railway variables --set "SQUINTLY_COEFFICIENT_HTTP={base}"')
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
