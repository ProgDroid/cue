-- sync_runs grows one row per source per sync and is queried by source in
-- latest_per_source / last_ok_unix / motn_recent_ok / any_sync_ok. Index source
-- so those don't full-scan as history accumulates (the orchestrator also prunes
-- to the newest N rows per source — see sync_runs::prune_old_runs).
CREATE INDEX IF NOT EXISTS idx_sync_runs_source ON sync_runs(source);
