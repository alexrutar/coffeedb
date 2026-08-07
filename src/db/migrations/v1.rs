use anyhow::Result;
use rusqlite::Connection;

/// Migrates a coffeedb database from schema version 1 to schema version 2.
pub(super) fn migrate(connection: &mut Connection) -> Result<()> {
    const TARGET_VERSION: i32 = 2;
    const SQL: &str = r#"
PRAGMA defer_foreign_keys = ON;

CREATE TABLE Coffee_v2 (
  label TEXT NOT NULL PRIMARY KEY,
  added TEXT NOT NULL,
  name TEXT NOT NULL,
  roast_date TEXT,
  roaster TEXT NOT NULL REFERENCES Roasters(name),
  origin TEXT,
  producer TEXT,
  farm TEXT,
  elevation TEXT,
  variety TEXT,
  process TEXT,
  flavours TEXT NOT NULL,
  archived INTEGER NOT NULL DEFAULT 0
) STRICT;
INSERT INTO Coffee_v2 (
  label, added, name, roast_date, roaster, origin, producer, farm, elevation,
  variety, process, flavours, archived
)
SELECT
  label, added, name, roast_date, roaster, origin, producer, farm, elevation,
  variety, process, flavours, archived
FROM Coffee;

CREATE TABLE Labels_v2 (
  type TEXT NOT NULL UNIQUE,
  short TEXT NOT NULL PRIMARY KEY,
  count INTEGER NOT NULL
) STRICT;
INSERT INTO Labels_v2 (type, short, count)
SELECT type, short, count FROM Labels;

CREATE TABLE Grinders_v2 (
  label TEXT NOT NULL PRIMARY KEY,
  name TEXT NOT NULL
) STRICT;
INSERT INTO Grinders_v2 (label, name)
SELECT label, name FROM Grinders;

CREATE TABLE BrewDevices_v2 (
  label TEXT NOT NULL PRIMARY KEY,
  name TEXT NOT NULL,
  method TEXT NOT NULL
) STRICT;
INSERT INTO BrewDevices_v2 (label, name, method)
SELECT label, name, method FROM BrewDevices;

CREATE TABLE Brews_v2 (
  id INTEGER PRIMARY KEY,
  coffee TEXT NOT NULL REFERENCES Coffee_v2(label),
  device TEXT NOT NULL REFERENCES BrewDevices_v2(label),
  grinder TEXT NOT NULL REFERENCES Grinders_v2(label),
  method TEXT NOT NULL,
  occurred TEXT NOT NULL,
  temperature_c REAL NOT NULL,
  coffee_grams REAL NOT NULL,
  grind_setting TEXT NOT NULL,
  notes TEXT,
  metadata TEXT NOT NULL
) STRICT;
INSERT INTO Brews_v2 (
  id, coffee, device, grinder, method, occurred, temperature_c, coffee_grams,
  grind_setting, notes, metadata
)
SELECT
  id, coffee, device, grinder, method, occurred, temperature_c, coffee_grams,
  grind_setting, notes, metadata
FROM Brews;

DROP TABLE Brews;
DROP TABLE Coffee;
DROP TABLE Labels;
DROP TABLE Grinders;
DROP TABLE BrewDevices;

ALTER TABLE Coffee_v2 RENAME TO Coffee;
ALTER TABLE Labels_v2 RENAME TO Labels;
ALTER TABLE Grinders_v2 RENAME TO Grinders;
ALTER TABLE BrewDevices_v2 RENAME TO BrewDevices;
ALTER TABLE Brews_v2 RENAME TO Brews;

CREATE INDEX Coffee_roaster_idx ON Coffee(roaster);
CREATE INDEX Coffee_label_order_idx ON Coffee(
  CASE substr(label, 1, 1) WHEN 'E' THEN 0 WHEN 'F' THEN 1 ELSE 2 END,
  CAST(substr(label, 2) AS INTEGER)
);
CREATE INDEX Brews_coffee_occurred_idx ON Brews(coffee, occurred DESC, id DESC);
CREATE INDEX Brews_device_idx ON Brews(device);
CREATE INDEX Brews_grinder_idx ON Brews(grinder);
"#;

    let transaction = connection.transaction()?;
    transaction.execute_batch(SQL)?;
    transaction.pragma_update(None, "user_version", TARGET_VERSION)?;
    transaction.commit()?;
    Ok(())
}
