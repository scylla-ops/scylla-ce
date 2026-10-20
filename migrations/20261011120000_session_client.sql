-- The client that opened a session, as the login call showed it. Both are
-- NULL on a session that the server opened before it recorded them, and when
-- the call did not show them.
ALTER TABLE sessions ADD COLUMN user_agent TEXT NULL;
ALTER TABLE sessions ADD COLUMN ip_address TEXT NULL;
