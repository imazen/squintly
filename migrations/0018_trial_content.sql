-- What kind of content a trial actually showed.
--
-- `trials` recorded only `source_hash`, so nothing downstream could say which
-- stratum — or which content class — a response came from without joining
-- against a manifest that may since have changed. Two consequences, both real:
--
--  1. imazen/squintly#4 wants "per-reference and per-category human-vs-ssim2
--     SROCC". The category was simply not in the export, so that analysis could
--     not be run from `responses.tsv` at all.
--  2. A check for "did the non-photo study serve photographs" silently read a
--     column that did not exist and therefore always answered no. A vacuous
--     check is worse than a missing one — it reports reassurance.
--
-- Recorded at serve time rather than derived at export time on purpose: the
-- classification is a property of the trial as it was shown. Re-deriving later
-- would relabel history whenever the registry or the corpus changed — which is
-- exactly what happened when `9226-lilith-ai-products` was reclassified from
-- non-photo to photo after the live study reported photorealistic product
-- images ("there are product images like the baby clothing").
--
-- NULL on rows served before this migration: the corpus at that time is not
-- recoverable, and guessing it from today's manifest would be a fabrication.
ALTER TABLE trials ADD COLUMN source_corpus TEXT;

-- 'photo' | 'non_photo' | 'unknown', as classified when the trial was served.
ALTER TABLE trials ADD COLUMN content_class TEXT;
