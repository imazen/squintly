-- Per-session native codec support, captured client-side via 1×1 decode probe.
-- See web/src/codec-probe.ts and docs for rationale: serving a JXL trial to a
-- Firefox observer means we'd have to transcode it, which would invalidate
-- the perceptual measurement. Filter at the sampler instead.

ALTER TABLE sessions ADD COLUMN supported_codecs TEXT;  -- comma-separated, e.g. "jpeg,png,webp,avif,jxl"
ALTER TABLE sessions ADD COLUMN codec_probe_cached INTEGER NOT NULL DEFAULT 0;  -- 0/1
