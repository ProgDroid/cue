CREATE TABLE titles (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    imdb_id      TEXT UNIQUE,
    tmdb_id      TEXT,
    plex_guid    TEXT,
    title        TEXT NOT NULL,
    year         INTEGER NOT NULL,
    type         TEXT NOT NULL CHECK (type IN ('movie','series')),
    imdb_rating  REAL,
    length       TEXT NOT NULL DEFAULT '',
    description  TEXT NOT NULL DEFAULT '',
    added_at     TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at   TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE title_services (
    title_id  INTEGER NOT NULL REFERENCES titles(id) ON DELETE CASCADE,
    service   TEXT NOT NULL CHECK (service IN ('plex','disney','crunchyroll')),
    PRIMARY KEY (title_id, service)
);

CREATE TABLE title_genres (
    title_id  INTEGER NOT NULL REFERENCES titles(id) ON DELETE CASCADE,
    genre     TEXT NOT NULL,
    PRIMARY KEY (title_id, genre)
);

CREATE TABLE title_cast (
    title_id  INTEGER NOT NULL REFERENCES titles(id) ON DELETE CASCADE,
    person    TEXT NOT NULL,
    ord       INTEGER NOT NULL,
    PRIMARY KEY (title_id, ord)
);

CREATE TABLE title_embeddings (
    title_id  INTEGER PRIMARY KEY REFERENCES titles(id) ON DELETE CASCADE,
    vector    BLOB NOT NULL,
    model     TEXT NOT NULL,
    dims      INTEGER NOT NULL
);

CREATE TABLE user_ratings (
    imdb_id   TEXT PRIMARY KEY,
    rating    INTEGER NOT NULL CHECK (rating BETWEEN 1 AND 5),
    rated_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE watch_history (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    imdb_id     TEXT NOT NULL,
    watched_at  TEXT NOT NULL DEFAULT (datetime('now')),
    source      TEXT NOT NULL CHECK (source IN ('manual','plex')),
    season      INTEGER,
    episode     INTEGER
);

CREATE TABLE sync_runs (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    source       TEXT NOT NULL,
    started_at   TEXT NOT NULL DEFAULT (datetime('now')),
    finished_at  TEXT,
    status       TEXT NOT NULL,
    item_count   INTEGER NOT NULL DEFAULT 0,
    error        TEXT
);

CREATE INDEX idx_title_services_service ON title_services(service);
CREATE INDEX idx_title_genres_genre ON title_genres(genre);
CREATE INDEX idx_watch_history_imdb ON watch_history(imdb_id);
