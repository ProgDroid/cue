-- Make manual-watch idempotency a hard invariant: at most one 'manual' row per
-- title. `set_watched` already upholds this with an INSERT ... WHERE NOT EXISTS
-- guard; this partial index makes it structural rather than guard-only.
-- Plex (and any future) sources are outside the partial predicate, so their
-- rows (repeat views, multiple episodes) remain unconstrained.
CREATE UNIQUE INDEX idx_watch_history_manual_unique
    ON watch_history (imdb_id)
    WHERE source = 'manual';
