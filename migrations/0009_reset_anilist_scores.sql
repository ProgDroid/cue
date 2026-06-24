-- One-time reset. AniList enrichment is populate-once (it skips rows that
-- already have a score), so the matching-bug fixes — tmdb movie/tv namespacing
-- and dropping ambiguous keys — only take effect on rows re-enriched from
-- scratch. Null the AniList columns so the next sync re-resolves every title
-- with the corrected map (false matches like "Dude, Where's My Car?" disappear;
-- ambiguous franchises like Dominion fall back to their generic score).
-- Harmless on a fresh DB (no rows); the scores are re-derivable from AniList.
UPDATE titles SET anilist_id = NULL, anilist_score = NULL;
