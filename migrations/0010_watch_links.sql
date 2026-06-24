-- Watch-at-source deep links: per-service MOTN link, Plex per-item ratingKey,
-- and a kv table for the server-global Plex machineIdentifier.
ALTER TABLE title_services ADD COLUMN link TEXT;
ALTER TABLE titles ADD COLUMN plex_rating_key TEXT;

CREATE TABLE app_meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
