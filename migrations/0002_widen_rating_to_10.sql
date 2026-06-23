-- SQLite can't ALTER a CHECK constraint, so rebuild user_ratings with 1-10.
CREATE TABLE user_ratings_new (
    imdb_id   TEXT PRIMARY KEY,
    rating    INTEGER NOT NULL CHECK (rating BETWEEN 1 AND 10),
    rated_at  TEXT NOT NULL DEFAULT (datetime('now'))
);
INSERT INTO user_ratings_new (imdb_id, rating, rated_at)
    SELECT imdb_id, rating, rated_at FROM user_ratings;
DROP TABLE user_ratings;
ALTER TABLE user_ratings_new RENAME TO user_ratings;
