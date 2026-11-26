-- The address of the photo of a person that its provider gave at the last sign-in or link: the
-- GitHub avatar, or the OIDC claim "picture". The server downloads it only for a new account and
-- for an import that the user asks for. A link that waits keeps it until the password comes.
ALTER TABLE external_identities ADD COLUMN picture_url text;

ALTER TABLE pending_links ADD COLUMN picture_url text;
