ALTER TABLE titles RENAME COLUMN imdb_rating TO score;
ALTER TABLE titles ADD COLUMN anilist_id    INTEGER;
ALTER TABLE titles ADD COLUMN anilist_score REAL;
