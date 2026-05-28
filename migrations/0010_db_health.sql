-- db_health: per-table row counts + DB-file size, refreshed hourly by
-- main.rs::spawn_db_health_task. Closes the data-sprawl exit gate from
-- README §7 ("per-table row counts in a db_health table updated hourly
-- so we notice when drift starts"). The hourly cadence is conservative
-- — for a write-heavy hour at squintly pilot scale (≤ 500 responses/hr)
-- the row counts are stable enough that polling more often would just
-- add noise.

CREATE TABLE db_health (
    refreshed_at   INTEGER NOT NULL,            -- ms since epoch
    table_name     TEXT    NOT NULL,
    row_count      INTEGER NOT NULL,
    PRIMARY KEY (refreshed_at, table_name)
);

CREATE INDEX idx_db_health_table ON db_health(table_name, refreshed_at DESC);

-- One row per refresh holds DB-file metadata (size in bytes, sqlite
-- page count and size, etc.). Keyed on refreshed_at alone since it's a
-- per-snapshot fact, not per-table.
CREATE TABLE db_health_meta (
    refreshed_at   INTEGER PRIMARY KEY,
    db_size_bytes  INTEGER NOT NULL,
    page_count     INTEGER NOT NULL,
    page_size      INTEGER NOT NULL,
    sqlite_version TEXT    NOT NULL
);
