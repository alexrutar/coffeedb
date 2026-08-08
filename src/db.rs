use std::{num::NonZero, path::Path};

use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, Local};
use rusqlite::{
    Connection, OpenFlags, OptionalExtension, Transaction, TransactionBehavior, params,
};
use url::Url;

mod types;
pub use types::CoffeeElevation;
use types::{BrewDeviceRow, BrewRow, CoffeeRow, Json};

mod migrations;

use crate::{
    brew::{Brew, BrewData, BrewDevice, BrewMethod, DeviceLabel, Grinder, GrinderLabel},
    label::{CoffeeType, Label},
    record::{Coffee, CoffeeData},
};

const SCHEMA: &str = r#"
CREATE TABLE Roasters (
  name TEXT NOT NULL PRIMARY KEY,
  url TEXT
) STRICT;

CREATE TABLE Coffee (
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

CREATE TABLE Flavours (
  description TEXT NOT NULL PRIMARY KEY
) STRICT;

CREATE TABLE Labels (
  type TEXT NOT NULL UNIQUE,
  short TEXT NOT NULL PRIMARY KEY,
  count INTEGER NOT NULL
) STRICT;

INSERT INTO Labels (type, short, count) VALUES
  ('Espresso', 'E', 0),
  ('Filter', 'F', 0),
  ('Omni', 'O', 0);

CREATE INDEX Coffee_roaster_idx ON Coffee(roaster);
CREATE INDEX Coffee_label_order_idx ON Coffee(
  CASE substr(label, 1, 1) WHEN 'E' THEN 0 WHEN 'F' THEN 1 ELSE 2 END,
  CAST(substr(label, 2) AS INTEGER)
);
"#;

const BREW_SCHEMA: &str = r#"
CREATE TABLE Grinders (
  label TEXT NOT NULL PRIMARY KEY,
  name TEXT NOT NULL
) STRICT;
CREATE TABLE BrewDevices (
  label TEXT NOT NULL PRIMARY KEY,
  name TEXT NOT NULL,
  method TEXT NOT NULL
) STRICT;
CREATE TABLE Brews (
  id INTEGER PRIMARY KEY,
  coffee TEXT NOT NULL REFERENCES Coffee(label),
  device TEXT NOT NULL REFERENCES BrewDevices(label),
  grinder TEXT NOT NULL REFERENCES Grinders(label),
  method TEXT NOT NULL,
  occurred TEXT NOT NULL,
  temperature_c REAL NOT NULL,
  coffee_grams REAL NOT NULL,
  grind_setting TEXT NOT NULL,
  notes TEXT,
  metadata TEXT NOT NULL
) STRICT;
CREATE INDEX Brews_coffee_occurred_idx ON Brews(coffee, occurred DESC, id DESC);
CREATE INDEX Brews_device_idx ON Brews(device);
CREATE INDEX Brews_grinder_idx ON Brews(grinder);
"#;

// First four bytes of SHA-256("coffeedb"), interpreted as a big-endian integer.
const APPLICATION_ID: i32 = 0x75af_f574;

pub struct Database {
    connection: Connection,
}

impl Database {
    pub fn open(path: &Path) -> Result<Self> {
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let connection = Connection::open_with_flags(path, flags)
            .with_context(|| format!("failed to open database '{}'", path.display()))?;
        let mut database = Self { connection };
        database
            .connection
            .pragma_update(None, "foreign_keys", "ON")?;
        database.initialize()?;
        Ok(database)
    }

    pub fn open_read_only(path: &Path) -> Result<Self> {
        let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let connection = Connection::open_with_flags(path, flags)
            .with_context(|| format!("failed to open database '{}'", path.display()))?;
        let application_id: i32 =
            connection.pragma_query_value(None, "application_id", |row| row.get(0))?;
        if application_id != APPLICATION_ID {
            bail!("database is not coffeedb-compatible");
        }
        let version: i32 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version > migrations::LATEST_VERSION {
            bail!(
                "database schema version {version} is newer than supported version {}",
                migrations::LATEST_VERSION
            );
        }
        Ok(Self { connection })
    }

    #[cfg(test)]
    fn in_memory() -> Result<Self> {
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let connection = Connection::open_in_memory_with_flags(flags)?;
        let mut database = Self { connection };
        database
            .connection
            .pragma_update(None, "foreign_keys", "ON")?;
        database.initialize()?;
        Ok(database)
    }

    fn initialize(&mut self) -> Result<()> {
        let application_id: i32 =
            self.connection
                .pragma_query_value(None, "application_id", |row| row.get(0))?;

        if application_id == APPLICATION_ID {
            let version: i32 = self
                .connection
                .pragma_query_value(None, "user_version", |row| row.get(0))?;
            if version > migrations::LATEST_VERSION {
                bail!(
                    "database schema version {version} is newer than supported version {}",
                    migrations::LATEST_VERSION
                );
            }
            migrations::migrate(&mut self.connection, version)?;
            return Ok(());
        }

        // Fall back to checking sqlite_master only when the application ID did not match.
        let is_empty = {
            let mut statement = self.connection.prepare("SELECT name FROM sqlite_master")?;
            let mut rows = statement.query([])?;
            rows.next()?.is_none()
        };
        if is_empty && application_id == 0 {
            let transaction = self.connection.transaction()?;
            transaction.pragma_update(None, "application_id", APPLICATION_ID)?;
            transaction.execute_batch(SCHEMA)?;
            transaction.execute_batch(BREW_SCHEMA)?;
            transaction.pragma_update(None, "user_version", migrations::LATEST_VERSION)?;
            transaction.commit()?;
            return Ok(());
        }

        match application_id {
            0 => bail!("database is not coffeedb-compatible: SQLite application_id is not set"),
            found => bail!(
                "database is not coffeedb-compatible: expected SQLite application_id \
                 {APPLICATION_ID}, found {found}"
            ),
        }
    }

    pub fn roaster_url(&self, name: &str) -> Result<Option<Option<String>>> {
        Ok(self
            .connection
            .query_row("SELECT url FROM Roasters WHERE name = ?1", [name], |row| {
                row.get(0)
            })
            .optional()?)
    }

    pub fn add(
        &mut self,
        kind: CoffeeType,
        data: CoffeeData,
        new_roaster_url: Option<&Url>,
    ) -> Result<Coffee> {
        let data = data.validate()?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        ensure_roaster(&transaction, &data.roaster, new_roaster_url)?;
        let next: i64 = transaction.query_row(
            "UPDATE Labels SET count = count + 1 WHERE short = ?1 RETURNING count",
            [kind.short().to_string()],
            |row| row.get(0),
        )?;
        let index = u32::try_from(next)
            .ok()
            .and_then(NonZero::new)
            .ok_or_else(|| anyhow!("label counter is outside the supported range"))?;
        let label = Label::new(kind, index);
        let added = Local::now();
        insert_coffee(&transaction, label, &added, &data)?;
        transaction.commit()?;
        Ok(Coffee {
            label,
            added,
            archived: false,
            data,
        })
    }

    pub fn get(&self, label: Label) -> Result<Option<Coffee>> {
        Ok(self
            .connection
            .query_row(
                "SELECT label, added, name, roast_date, roaster, origin, producer, farm,
                        elevation, variety, process, flavours, archived
                 FROM Coffee WHERE label = ?1",
                [label],
                CoffeeRow::from_row,
            )
            .optional()?
            .map(|row| row.0))
    }

    pub fn update(
        &mut self,
        label: Label,
        data: CoffeeData,
        new_roaster_url: Option<&Url>,
    ) -> Result<Coffee> {
        let data = data.validate()?;
        let transaction = self.connection.transaction()?;
        ensure_roaster(&transaction, &data.roaster, new_roaster_url)?;
        let (added, archived): (DateTime<Local>, bool) = transaction
            .query_row(
                "UPDATE Coffee SET name=?2, roast_date=?3, roaster=?4, origin=?5,
             producer=?6, farm=?7, elevation=?8, variety=?9, process=?10,
             flavours=?11 WHERE label=?1 RETURNING added, archived",
                params![
                    label,
                    data.name,
                    data.roast_date,
                    data.roaster,
                    data.origin,
                    data.producer,
                    data.farm,
                    data.elevation,
                    data.variety,
                    data.process,
                    Json(&data.flavours)
                ],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?
            .ok_or_else(|| anyhow!("coffee '{label}' does not exist"))?;
        register_flavours(&transaction, &data.flavours)?;
        transaction.commit()?;
        Ok(Coffee {
            label,
            added,
            archived,
            data,
        })
    }

    pub fn list(&self) -> Result<Vec<Coffee>> {
        self.list_coffees(false)
    }

    pub fn list_active(&self) -> Result<Vec<Coffee>> {
        self.list_coffees(true)
    }

    fn list_coffees(&self, active_only: bool) -> Result<Vec<Coffee>> {
        let mut statement = self.connection.prepare(
            "SELECT label, added, name, roast_date, roaster, origin, producer, farm,
                    elevation, variety, process, flavours, archived
             FROM Coffee
             WHERE NOT ?1 OR archived=0
             ORDER BY CASE substr(label, 1, 1) WHEN 'E' THEN 0 WHEN 'F' THEN 1 ELSE 2 END,
                      CAST(substr(label, 2) AS INTEGER)",
        )?;
        Ok(statement
            .query_map([active_only], CoffeeRow::from_row)?
            .map(|row| row.map(|row| row.0))
            .collect::<rusqlite::Result<_>>()?)
    }

    pub fn set_archived(&mut self, label: Label, archived: bool) -> Result<()> {
        let changed = self.connection.execute(
            "UPDATE Coffee SET archived=?2 WHERE label=?1",
            params![label, archived],
        )?;
        if changed == 0 {
            bail!("coffee '{label}' does not exist");
        }
        Ok(())
    }

    pub fn list_flavours(&self) -> Result<Vec<String>> {
        let mut statement = self
            .connection
            .prepare("SELECT description FROM Flavours ORDER BY description")?;
        Ok(statement
            .query_map([], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?)
    }

    pub fn device(&self, label: &DeviceLabel) -> Result<Option<BrewDevice>> {
        Ok(self
            .connection
            .query_row(
                "SELECT label, name, method FROM BrewDevices WHERE label=?1",
                [label.as_str()],
                BrewDeviceRow::from_row,
            )
            .optional()?
            .map(|row| row.0))
    }

    pub fn list_devices(&self) -> Result<Vec<BrewDevice>> {
        let mut statement = self
            .connection
            .prepare("SELECT label, name, method FROM BrewDevices ORDER BY label")?;
        Ok(statement
            .query_map([], BrewDeviceRow::from_row)?
            .map(|row| row.map(|row| row.0))
            .collect::<rusqlite::Result<_>>()?)
    }

    pub fn grinder(&self, label: &GrinderLabel) -> Result<Option<Grinder>> {
        Ok(self
            .connection
            .query_row(
                "SELECT name FROM Grinders WHERE label=?1",
                [label.as_str()],
                |row| {
                    Ok(Grinder {
                        label: label.clone(),
                        name: row.get(0)?,
                    })
                },
            )
            .optional()?)
    }

    pub fn brew(&self, id: i64) -> Result<Option<Brew>> {
        Ok(self
            .connection
            .query_row(
                "SELECT id,coffee,device,method,occurred,temperature_c,coffee_grams,grinder,grind_setting,notes,metadata FROM Brews WHERE id=?1",
                [id],
                BrewRow::from_row,
            )
            .optional()?
            .map(|row| row.0))
    }

    pub fn add_brew(
        &mut self,
        coffee: Label,
        data: BrewData,
        new_device: Option<&BrewDevice>,
        new_grinder: Option<&Grinder>,
    ) -> Result<Brew> {
        let data = data.validate()?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let exists = tx
            .query_row("SELECT 1 FROM Coffee WHERE label=?1", [coffee], |_| Ok(()))
            .optional()?
            .is_some();
        if !exists {
            bail!("coffee '{coffee}' does not exist");
        }
        if let Some(device) = new_device {
            if device.name.is_empty() {
                bail!("device name must not be empty");
            }
            tx.execute(
                "INSERT INTO BrewDevices(label,name,method) VALUES(?1,?2,?3)",
                params![device.label, device.name, device.method],
            )?;
        }
        let device = tx
            .query_row(
                "SELECT method FROM BrewDevices WHERE label=?1",
                [&data.device],
                |r| r.get::<_, BrewMethod>(0),
            )
            .optional()?
            .ok_or_else(|| anyhow!("device '{}' does not exist", data.device))?;
        if device != data.metadata.method() {
            bail!("brew metadata does not match device method '{device}'");
        }
        if let Some(grinder) = new_grinder {
            if grinder.name.is_empty() {
                bail!("grinder name must not be empty");
            }
            tx.execute(
                "INSERT INTO Grinders(label,name) VALUES(?1,?2)",
                params![grinder.label, grinder.name],
            )?;
        }
        tx.execute("INSERT INTO Brews(coffee,device,grinder,method,occurred,temperature_c,coffee_grams,grind_setting,notes,metadata) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)", params![coffee, data.device, data.grinder, device, data.occurred, data.temperature_c, data.coffee_grams, data.grind_setting, data.notes, Json(&data.metadata)])?;
        let id = tx.last_insert_rowid();
        tx.commit()?;
        Ok(Brew { id, coffee, data })
    }

    pub fn update_brew(&mut self, id: i64, data: BrewData) -> Result<Brew> {
        let data = data.validate()?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let coffee = tx
            .query_row("SELECT coffee FROM Brews WHERE id=?1", [id], |row| {
                row.get(0)
            })
            .optional()?
            .ok_or_else(|| anyhow!("brew '{id}' does not exist"))?;
        let method = tx
            .query_row(
                "SELECT method FROM BrewDevices WHERE label=?1",
                [&data.device],
                |row| row.get::<_, BrewMethod>(0),
            )
            .optional()?
            .ok_or_else(|| anyhow!("device '{}' does not exist", data.device))?;
        if method != data.metadata.method() {
            bail!("brew metadata does not match device method '{method}'");
        }
        let grinder_exists = tx
            .query_row(
                "SELECT 1 FROM Grinders WHERE label=?1",
                [&data.grinder],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if !grinder_exists {
            bail!("grinder '{}' does not exist", data.grinder);
        }
        tx.execute(
            "UPDATE Brews SET device=?1,grinder=?2,method=?3,occurred=?4,temperature_c=?5,coffee_grams=?6,grind_setting=?7,notes=?8,metadata=?9 WHERE id=?10",
            params![data.device, data.grinder, method, data.occurred, data.temperature_c, data.coffee_grams, data.grind_setting, data.notes, Json(&data.metadata), id],
        )?;
        tx.commit()?;
        Ok(Brew { id, coffee, data })
    }

    pub fn map_brews<F>(&self, coffee: Option<Label>, limit: i64, mut callback: F) -> Result<()>
    where
        F: FnMut(Brew),
    {
        if let Some(coffee) = coffee
            && self.get(coffee)?.is_none()
        {
            bail!("coffee '{coffee}' does not exist");
        }
        let mut statement = self.connection.prepare("SELECT id,coffee,device,method,occurred,temperature_c,coffee_grams,grinder,grind_setting,notes,metadata FROM Brews WHERE ?1 IS NULL OR coffee=?1 ORDER BY occurred DESC, id DESC LIMIT ?2")?;
        let brews = statement.query_map(params![coffee, limit], BrewRow::from_row)?;
        for brew in brews {
            callback(brew?.0);
        }
        Ok(())
    }
}

fn ensure_roaster(transaction: &Transaction<'_>, name: &str, url: Option<&Url>) -> Result<()> {
    transaction.execute(
        "INSERT OR IGNORE INTO Roasters (name, url) VALUES (?1, ?2)",
        params![name, url.map(Url::as_str)],
    )?;
    Ok(())
}

fn insert_coffee(
    transaction: &Transaction<'_>,
    label: Label,
    added: &DateTime<Local>,
    data: &CoffeeData,
) -> Result<()> {
    transaction.execute(
        "INSERT INTO Coffee
         (label, added, name, roast_date, roaster, origin, producer, farm, elevation, variety,
          process, flavours)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            label,
            added,
            data.name,
            data.roast_date,
            data.roaster,
            data.origin,
            data.producer,
            data.farm,
            data.elevation,
            data.variety,
            data.process,
            Json(&data.flavours)
        ],
    )?;
    register_flavours(transaction, &data.flavours)
}

fn register_flavours(transaction: &Transaction<'_>, flavours: &[String]) -> Result<()> {
    for flavour in flavours {
        transaction.execute(
            "INSERT OR IGNORE INTO Flavours (description) VALUES (?1)",
            [flavour],
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data(name: &str) -> CoffeeData {
        CoffeeData {
            name: name.into(),
            roast_date: None,
            roaster: "Test Roaster".into(),
            origin: None,
            producer: None,
            farm: None,
            elevation: None,
            variety: None,
            process: None,
            flavours: vec!["Apple".into()],
        }
    }

    #[test]
    fn parses_supported_elevation_formats() {
        for (value, expected) in [
            ("1000", CoffeeElevation::Single(1000)),
            ("1000 masl", CoffeeElevation::Single(1000)),
            ("1000 - 1200", CoffeeElevation::Range(1000, 1200)),
            ("1000 masl - 1200 masl", CoffeeElevation::Range(1000, 1200)),
            ("1000 - 1200 masl", CoffeeElevation::Range(1000, 1200)),
        ] {
            assert_eq!(value.parse::<CoffeeElevation>().unwrap(), expected);
        }
    }

    #[test]
    fn rejects_unsupported_elevation_formats() {
        for value in [
            "",
            "1000 m",
            "1000masl",
            "1000 masl - 1200",
            "1000 - 1200 meters",
            "1000 - 1200 - 1300",
            "65536",
        ] {
            assert!(
                value.parse::<CoffeeElevation>().is_err(),
                "accepted {value}"
            );
        }
    }

    #[test]
    fn allocates_independent_sequential_labels() -> Result<()> {
        let mut database = Database::in_memory()?;
        assert_eq!(
            database
                .add(CoffeeType::Filter, data("One"), None)?
                .label
                .to_string(),
            "F1"
        );
        assert_eq!(
            database
                .add(CoffeeType::Espresso, data("Two"), None)?
                .label
                .to_string(),
            "E1"
        );
        assert_eq!(
            database
                .add(CoffeeType::Filter, data("Three"), None)?
                .label
                .to_string(),
            "F2"
        );
        Ok(())
    }

    #[test]
    fn updates_data_and_flavours() -> Result<()> {
        let mut database = Database::in_memory()?;
        let url = Url::parse("https://example.com")?;
        let coffee = database.add(CoffeeType::Omni, data("Old"), Some(&url))?;
        assert_eq!(
            database.roaster_url("Test Roaster")?,
            Some(Some("https://example.com/".into()))
        );
        let added = coffee.added;
        let mut replacement = data("New");
        replacement.flavours = vec!["cocoa".into()];
        database.update(coffee.label, replacement.clone(), None)?;
        let updated = database.get(coffee.label)?.unwrap();
        assert_eq!(updated.data, replacement);
        assert_eq!(updated.added, added);
        Ok(())
    }

    #[test]
    fn gets_and_updates_brews_without_changing_their_identity() -> Result<()> {
        let mut database = Database::in_memory()?;
        let coffee = database.add(CoffeeType::Filter, data("Coffee"), None)?;
        let device = BrewDevice {
            label: "v60".parse()?,
            name: "V60".into(),
            method: BrewMethod::Pourover,
        };
        let grinder = Grinder {
            label: "grinder".parse()?,
            name: "Grinder".into(),
        };
        let original = BrewData::parse_for_method(
            r#"device = "v60"
occurred = 2026-08-08T08:00:00Z
temperature_c = 95
coffee_grams = 15
grinder = "grinder"
grind_setting = "20"
pours = [[0, 50], [30, 200]]
"#,
            BrewMethod::Pourover,
        )?;
        let brew = database.add_brew(coffee.label, original, Some(&device), Some(&grinder))?;

        let mut replacement = brew.data.clone();
        replacement.temperature_c = 90.0;
        replacement.notes = Some("Sweeter".into());
        let updated = database.update_brew(brew.id, replacement.clone())?;

        assert_eq!(updated.id, brew.id);
        assert_eq!(updated.coffee, coffee.label);
        assert_eq!(database.brew(brew.id)?.unwrap().data, replacement);
        Ok(())
    }

    #[test]
    fn stores_added_with_datetime_to_sql() -> Result<()> {
        let mut database = Database::in_memory()?;
        let coffee = database.add(CoffeeType::Filter, data("Timestamped"), None)?;
        let stored: String = database.connection.query_row(
            "SELECT added FROM Coffee WHERE label = ?1",
            [coffee.label],
            |row| row.get(0),
        )?;

        assert!(stored.ends_with("+00:00"));
        assert_eq!(database.get(coffee.label)?.unwrap().added, coffee.added);
        Ok(())
    }

    #[test]
    fn lists_sorted_unique_flavours() -> Result<()> {
        let mut database = Database::in_memory()?;
        let mut first = data("First");
        first.flavours = vec!["Lemon".into(), "Apple".into()];
        database.add(CoffeeType::Filter, first, None)?;
        let mut second = data("Second");
        second.flavours = vec!["Apple".into(), "Cocoa".into()];
        database.add(CoffeeType::Omni, second, None)?;

        assert_eq!(database.list_flavours()?, ["apple", "cocoa", "lemon"]);
        let coffees = database.list()?;
        assert_eq!(coffees[0].data.flavours, ["lemon", "apple"]);
        assert_eq!(coffees[1].data.flavours, ["apple", "cocoa"]);
        let (storage_type, json): (String, String) = database.connection.query_row(
            "SELECT typeof(flavours), json(flavours) FROM Coffee WHERE label = 'F1'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        assert_eq!(storage_type, "text");
        assert_eq!(json, r#"["lemon","apple"]"#);
        Ok(())
    }

    #[test]
    fn stamps_new_database_with_application_id() -> Result<()> {
        let database = Database::in_memory()?;
        let application_id: i32 =
            database
                .connection
                .pragma_query_value(None, "application_id", |row| row.get(0))?;
        assert_eq!(application_id, APPLICATION_ID);
        Ok(())
    }

    #[test]
    fn rejects_existing_database_without_application_id() -> Result<()> {
        let connection = Connection::open_in_memory()?;
        connection.execute("CREATE TABLE Other (value TEXT)", [])?;
        let mut database = Database { connection };
        let error = database.initialize().unwrap_err();
        assert!(error.to_string().contains("application_id is not set"));
        Ok(())
    }

    #[test]
    fn migrates_v0_to_check_free_latest_schema() -> Result<()> {
        const V0_SCHEMA: &str = r#"
CREATE TABLE Roasters (
  name TEXT NOT NULL PRIMARY KEY,
  url TEXT
) STRICT;
CREATE TABLE Coffee (
  label TEXT NOT NULL PRIMARY KEY,
  name TEXT NOT NULL,
  roast_date TEXT,
  roaster TEXT NOT NULL REFERENCES Roasters(name),
  origin TEXT,
  producer TEXT,
  farm TEXT,
  variety TEXT,
  process TEXT,
  elevation TEXT,
  added TEXT NOT NULL DEFAULT '2026-08-06 12:30:00+00:00',
  flavours TEXT NOT NULL CHECK (json_valid(flavours))
) STRICT;
CREATE TABLE Flavours (
  description TEXT NOT NULL PRIMARY KEY
) STRICT;
CREATE TABLE Labels (
  type TEXT NOT NULL UNIQUE,
  short TEXT NOT NULL PRIMARY KEY,
  count INTEGER NOT NULL CHECK (count >= 0)
) STRICT;
INSERT INTO Labels (type, short, count) VALUES
  ('Espresso', 'E', 0),
  ('Filter', 'F', 1),
  ('Omni', 'O', 0);
INSERT INTO Roasters(name, url) VALUES ('Legacy Roaster', NULL);
INSERT INTO Coffee(
  label, added, name, roast_date, roaster, origin, producer, farm, elevation,
  variety, process, flavours
) VALUES (
  'F1', '2026-01-01T00:00:00+00:00', 'Legacy Coffee', NULL, 'Legacy Roaster',
  NULL, NULL, NULL, NULL, NULL, NULL, '["apple"]'
);
CREATE INDEX Coffee_roaster_idx ON Coffee(roaster);
CREATE INDEX Coffee_label_order_idx ON Coffee(
  CASE substr(label, 1, 1) WHEN 'E' THEN 0 WHEN 'F' THEN 1 ELSE 2 END,
  CAST(substr(label, 2) AS INTEGER)
);
"#;

        let connection = Connection::open_in_memory()?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        connection.pragma_update(None, "application_id", APPLICATION_ID)?;
        connection.execute_batch(V0_SCHEMA)?;
        let mut database = Database { connection };
        database.initialize()?;

        let version: i32 = database
            .connection
            .pragma_query_value(None, "user_version", |row| row.get(0))?;
        assert_eq!(version, migrations::LATEST_VERSION);
        assert_eq!(
            database.get("F1".parse()?)?.unwrap().data.name,
            "Legacy Coffee"
        );

        let schema: String = database.connection.query_row(
            "SELECT group_concat(sql, ' ') FROM sqlite_master WHERE sql IS NOT NULL",
            [],
            |row| row.get(0),
        )?;
        assert!(!schema.to_ascii_uppercase().contains("CHECK"));
        let triggers: i64 = database.connection.query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='trigger'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(triggers, 0);

        database.connection.execute_batch(
            "UPDATE Coffee SET flavours='not json', archived=7 WHERE label='F1';
             UPDATE Labels SET count=-1 WHERE short='F';
             INSERT INTO Grinders(label, name) VALUES ('', '');
             INSERT INTO BrewDevices(label, name, method) VALUES ('', '', 'unknown');
             INSERT INTO Brews(
               coffee, device, grinder, method, occurred, temperature_c, coffee_grams,
               grind_setting, notes, metadata
             ) VALUES ('F1', '', '', 'other', 'not utc', -1, -1, '', '', 'not json');",
        )?;
        Ok(())
    }
}
