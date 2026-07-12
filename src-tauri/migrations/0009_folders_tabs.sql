CREATE TABLE folders (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    position INTEGER NOT NULL,
    collapsed INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL
);

CREATE TABLE tabs (
    id TEXT PRIMARY KEY,
    folder_id TEXT REFERENCES folders(id) ON DELETE RESTRICT,
    name TEXT NOT NULL,
    position INTEGER NOT NULL,
    layout TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE INDEX idx_tabs_folder_position ON tabs(folder_id, position, created_at);
