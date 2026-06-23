-- Speed up sync identity resolution. store::find_existing() looks a title up by
-- imdb_id first (already UNIQUE in 0001), then falls back to tmdb_id and
-- plex_guid; without these indexes each fallback is a full table scan on the
-- sync hot path (once per upserted title that misses on imdb_id).
--
-- Deliberately NON-UNIQUE: duplicate tmdb_id rows are tolerated by design
-- (see store.rs `upsert_tolerates_duplicate_tmdb_id`), so a UNIQUE index would
-- be wrong here.
CREATE INDEX IF NOT EXISTS idx_titles_tmdb_id ON titles(tmdb_id);
CREATE INDEX IF NOT EXISTS idx_titles_plex_guid ON titles(plex_guid);
