-- The options of the glyph avatar that the server drew as an image, so that the editor opens with
-- them. An uploaded image has none.
ALTER TABLE images ADD COLUMN glyph jsonb;
