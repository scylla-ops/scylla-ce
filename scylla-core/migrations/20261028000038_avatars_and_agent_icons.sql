-- The avatar of a user: an uploaded image, and the color of the initials. With no image and no
-- color, the user shows the initials on the default color.
ALTER TABLE users
    ADD COLUMN avatar_image_id uuid REFERENCES images (id) ON DELETE SET NULL,
    ADD COLUMN avatar_color text;

CREATE INDEX users_avatar_image_id_idx ON users (avatar_image_id);

-- The custom icon of an App: a glyph of the icon set of the web app with a color, or an image.
-- With no glyph and no image, the App shows its default icon.
ALTER TABLE apps
    ADD COLUMN icon_name text,
    ADD COLUMN icon_color text,
    ADD COLUMN icon_image_id uuid REFERENCES images (id) ON DELETE SET NULL;

CREATE INDEX apps_icon_image_id_idx ON apps (icon_image_id);
