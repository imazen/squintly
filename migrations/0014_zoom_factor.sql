-- Magnification used when the response was given.
--
-- Display is a hard minimum of 1:1 device pixels; observers may zoom IN to
-- inspect artefacts. Magnification is restricted to integer factors and
-- nearest-neighbour so that one image pixel becomes an exact NxN block of
-- device pixels: no interpolation invents values, and no pixel is unevenly
-- sized. A non-integer factor would give some source pixels 2 device px and
-- others 3, which fabricates structure that isn't in the encode — the opposite
-- of what a magnifier is for.
--
-- Recorded per response because a judgement made at 4x is not the same
-- observation as one made at 1x: the visual angle subtended by an artefact
-- differs, which is the whole quantity this study conditions on.
-- 1.0 = viewed at native 1:1.

ALTER TABLE responses ADD COLUMN zoom_factor REAL NOT NULL DEFAULT 1.0;
