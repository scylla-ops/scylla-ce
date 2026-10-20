-- A bearer token is stored as the SHA-256 of its UTF-8 bytes, in lowercase hex.
-- A lookup hashes the presented token the same way, so a read of these tables
-- gives no credential that works.
ALTER TABLE app_tokens RENAME COLUMN token TO token_hash;
UPDATE app_tokens SET token_hash = encode(sha256(convert_to(token_hash, 'UTF8')), 'hex');
ALTER INDEX app_tokens_token_key RENAME TO app_tokens_token_hash_key;

ALTER TABLE sessions RENAME COLUMN token TO token_hash;
UPDATE sessions SET token_hash = encode(sha256(convert_to(token_hash, 'UTF8')), 'hex');
ALTER INDEX sessions_token_key RENAME TO sessions_token_hash_key;

ALTER TABLE organization_invites RENAME COLUMN token TO token_hash;
UPDATE organization_invites SET token_hash = encode(sha256(convert_to(token_hash, 'UTF8')), 'hex');
ALTER TABLE organization_invites
    RENAME CONSTRAINT organization_invites_token_key TO organization_invites_token_hash_key;
DROP INDEX organization_invites_token_idx;
