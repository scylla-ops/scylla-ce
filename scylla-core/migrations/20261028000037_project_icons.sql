-- The images that a person uploads: the server keeps a square PNG that it made from the upload,
-- never the file of the person. A resource that shows an image refers to its row.
CREATE TABLE images (
    id uuid PRIMARY KEY,
    created_by uuid NOT NULL REFERENCES actors (id),
    content_type text NOT NULL,
    data bytea NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX images_created_by_idx ON images (created_by);

-- The custom icon of a project: a glyph of the icon set of the web app with a color, or an
-- image. With no glyph and no image, the project shows its default icon.
ALTER TABLE projects
    ADD COLUMN icon_name text,
    ADD COLUMN icon_color text,
    ADD COLUMN icon_image_id uuid REFERENCES images (id) ON DELETE SET NULL;

CREATE INDEX projects_icon_image_id_idx ON projects (icon_image_id);
