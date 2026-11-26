-- The custom icon of a workspace: a glyph of the icon set of the web app with a color, or an
-- image. With no glyph and no image, the workspace shows its default icon.
ALTER TABLE workspaces
    ADD COLUMN icon_name text,
    ADD COLUMN icon_color text,
    ADD COLUMN icon_image_id uuid REFERENCES images (id) ON DELETE SET NULL;

CREATE INDEX workspaces_icon_image_id_idx ON workspaces (icon_image_id);

-- The color of the folder of a group of projects: the name of a color of the web app, or a hex
-- color. With no color, the group shows the default color.
ALTER TABLE project_groups
    ADD COLUMN color text;
