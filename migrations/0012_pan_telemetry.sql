-- Panning telemetry.
--
-- The stimulus is now displayed at a hard minimum of 1:1 device pixels
-- (web/src/trial.ts). Anything larger than the viewport is explored by panning
-- rather than shrunk to fit, because a display downscale means the observer is
-- judging resampled pixels instead of the encoded ones — which is not the
-- quantity this study is trying to measure.
--
-- That makes "what did they actually look at" a real question for the first
-- time: on a 304 CSS px phone an XL stimulus shows roughly a third of its
-- width at a time. These columns record it so analysis can tell a judgement
-- made after exploring the image from one made on whatever happened to be
-- under the initial centred crop.
--
-- `visible_*` is the intersection of stimulus and viewport at response time —
-- distinct from `image_displayed_w_css`, which is now the *full* stimulus size
-- and can exceed the screen.

ALTER TABLE responses ADD COLUMN pan_count INTEGER NOT NULL DEFAULT 0;
ALTER TABLE responses ADD COLUMN pan_distance_css REAL NOT NULL DEFAULT 0;
ALTER TABLE responses ADD COLUMN pannable_w_css REAL NOT NULL DEFAULT 0;
ALTER TABLE responses ADD COLUMN pannable_h_css REAL NOT NULL DEFAULT 0;
ALTER TABLE responses ADD COLUMN visible_w_css REAL NOT NULL DEFAULT 0;
ALTER TABLE responses ADD COLUMN visible_h_css REAL NOT NULL DEFAULT 0;
