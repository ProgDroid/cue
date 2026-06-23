-- MOTN keeps its own showId-keyed snapshot here so it can rebuild its full
-- catalogue from /changes deltas instead of re-fetching everything each sync.
CREATE TABLE motn_catalog_cache (
    show_id    TEXT PRIMARY KEY,   -- MOTN internal show id (stable correlation key)
    payload    TEXT NOT NULL,      -- JSON: CachedTitle (title fields + attributed services)
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
