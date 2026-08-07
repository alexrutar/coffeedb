use anyhow::Result;
use rusqlite::Connection;

/// Migrates a coffeedb database from schema version 0 to schema version 1.
pub(super) fn migrate(connection: &mut Connection) -> Result<()> {
    const TARGET_VERSION: i32 = 1;
    const SQL: &str = r#"
ALTER TABLE Coffee ADD COLUMN archived INTEGER NOT NULL DEFAULT 0 CHECK(archived IN (0, 1));

CREATE TABLE Grinders (
  label TEXT NOT NULL PRIMARY KEY CHECK(label GLOB '[a-z0-9]*' AND label NOT GLOB '*[^a-z0-9-]*' AND label NOT LIKE '-%' AND label NOT LIKE '%-' AND label NOT LIKE '%--%'),
  name TEXT NOT NULL CHECK(length(name) > 0)
) STRICT;
CREATE TABLE BrewDevices (
  label TEXT NOT NULL PRIMARY KEY CHECK(label GLOB '[a-z0-9]*' AND label NOT GLOB '*[^a-z0-9-]*' AND label NOT LIKE '-%' AND label NOT LIKE '%-' AND label NOT LIKE '%--%'),
  name TEXT NOT NULL CHECK(length(name) > 0),
  method TEXT NOT NULL CHECK(method IN ('pourover', 'immersion', 'espresso'))
) STRICT;
CREATE TABLE Brews (
  id INTEGER PRIMARY KEY,
  coffee TEXT NOT NULL REFERENCES Coffee(label),
  device TEXT NOT NULL REFERENCES BrewDevices(label),
  grinder TEXT NOT NULL REFERENCES Grinders(label),
  method TEXT NOT NULL CHECK(method IN ('pourover', 'immersion', 'espresso')),
  occurred TEXT NOT NULL CHECK(substr(occurred, -1) = 'Z' OR substr(occurred, -6) = '+00:00'),
  temperature_c REAL NOT NULL CHECK(temperature_c > 0 AND temperature_c <= 100),
  coffee_grams REAL NOT NULL CHECK(coffee_grams > 0 AND coffee_grams < 1.0e308),
  grind_setting TEXT NOT NULL CHECK(length(grind_setting) > 0),
  notes TEXT CHECK(notes IS NULL OR length(notes) > 0),
  metadata TEXT NOT NULL CHECK(json_valid(metadata))
) STRICT;
CREATE TRIGGER Brews_device_method_insert BEFORE INSERT ON Brews
BEGIN SELECT CASE WHEN (SELECT method FROM BrewDevices WHERE label=NEW.device) IS NOT NEW.method THEN RAISE(ABORT, 'brew method does not match device') END; END;
CREATE TRIGGER Brews_device_method_update BEFORE UPDATE OF device, method ON Brews
BEGIN SELECT CASE WHEN (SELECT method FROM BrewDevices WHERE label=NEW.device) IS NOT NEW.method THEN RAISE(ABORT, 'brew method does not match device') END; END;
CREATE TRIGGER BrewDevices_method_immutable BEFORE UPDATE OF method ON BrewDevices
WHEN OLD.method IS NOT NEW.method BEGIN SELECT RAISE(ABORT, 'device method is immutable'); END;
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
