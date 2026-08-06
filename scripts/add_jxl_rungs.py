"""Add high-quality JXL rungs to a built SplitStore, to reach ssim2 90-100.

    python3 scripts/add_jxl_rungs.py demo-corpus 88,92,95,97,99

# Why JXL specifically

[measured 2026-08-06, correcting an earlier claim in this docstring]

With q100 in the ladder the near-lossless band is ALREADY reachable: jpegli
medians ssim2 93.4 at q100, and 863 of 11,424 encodings score >=90. The earlier
claim that the builder codecs "top out around 86-90 even at q100" was
extrapolated from the old q92-capped grid and is wrong.

So JXL is an ENRICHMENT rather than a rescue: a fifth codec, and extra density
in the 90-100 band where the other four thin out (only jpegli clears 93). That
band is where compression product decisions live and is zensim's documented weak
zone, so more of it is worth having — but the band is no longer empty without
JXL, and that changes how urgent these rungs are.

# The caveat an operator has to know

**Browser JXL support is not universal.** Chromium ships it behind a flag;
Safari has it natively. Squintly's codec probe records what each browser can
decode and the sampler only serves encodings a session declared support for, so
these rungs degrade safely — an observer who cannot decode JXL simply never sees
one, rather than meeting a broken image. But it does mean that on a panel with
no JXL-capable browser these encodings are scored and never judged. The welcome
screen's `jxlEnableHint` exists to nudge Chromium users toward the flag.

Runs AFTER build_demo_corpus.py, against its output directory. Idempotent: an
existing blob is reused, so a re-run only fills gaps.
"""
import json, pathlib, subprocess, sys, concurrent.futures as cf

store = pathlib.Path(sys.argv[1])
QUALITIES = [int(x) for x in sys.argv[2].split(',')]
blobs, meta = store/'blobs', store/'meta'
sources = sorted({p.stem for p in blobs.glob('*.png') if len(p.stem) == 64})
print(f"{len(sources)} sources x {len(QUALITIES)} jxl rungs = {len(sources)*len(QUALITIES)}")

def one(sha):
    out = []
    src = blobs/f'{sha}.png'
    for q in QUALITIES:
        eid = f'{sha[:16]}__jxl__q{q}'
        dst = blobs/f'{eid}.jxl'
        if not dst.exists():
            r = subprocess.run(['cjxl-rs', '-q', str(q), str(src), str(dst)],
                               capture_output=True)
            if r.returncode != 0 or not dst.exists():
                print(f"  FAIL {eid}: {r.stderr[-160:]!r}"); continue
        (meta/f'encoding_{eid}.json').write_text(json.dumps({
            "id": eid, "source_hash": sha, "codec_name": "jxl",
            "quality": float(q), "encoded_size": dst.stat().st_size}, indent=1))
        out.append(eid)
    return out

done = 0
with cf.ThreadPoolExecutor(max_workers=10) as ex:
    for res in ex.map(one, sources):
        done += 1
        if done % 20 == 0: print(f"  {done}/{len(sources)}")
print("added", sum(1 for _ in meta.glob('encoding_*__jxl__*.json')), "jxl encodings")
