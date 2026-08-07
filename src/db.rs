use std::{fmt, num::NonZero, path::Path, str::FromStr};

use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, Local};
use rusqlite::{
    Connection, OpenFlags, OptionalExtension, Transaction, TransactionBehavior, params, types::Type,
};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use url::Url;

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
  flavours TEXT NOT NULL CHECK (json_valid(flavours)),
  archived INTEGER NOT NULL DEFAULT 0 CHECK(archived IN (0, 1))
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
  ('Filter', 'F', 0),
  ('Omni', 'O', 0);

CREATE INDEX Coffee_roaster_idx ON Coffee(roaster);
CREATE INDEX Coffee_label_order_idx ON Coffee(
  CASE substr(label, 1, 1) WHEN 'E' THEN 0 WHEN 'F' THEN 1 ELSE 2 END,
  CAST(substr(label, 2) AS INTEGER)
);
"#;

const LATEST_VERSION: i32 = 1;
const BREW_SCHEMA: &str = r#"
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

// First four bytes of SHA-256("coffeedb"), interpreted as a big-endian integer.
const APPLICATION_ID: i32 = 0x75af_f574;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoffeeElevation {
    Single(u16),
    Range(u16, u16),
}

impl FromStr for CoffeeElevation {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        fn parse_half(value: &str) -> Result<(u16, bool)> {
            let parts: Vec<_> = value.split_whitespace().collect();
            if parts.len() != 1 && parts.len() != 2 {
                bail!("invalid elevation");
            }
            if parts.get(1).is_some_and(|unit| *unit != "masl") {
                bail!("invalid elevation");
            }
            let number = parts[0]
                .parse::<u16>()
                .map_err(|_| anyhow!("invalid elevation: '{}' does not fit in a u16", parts[0]))?;
            Ok((number, parts.len() == 2))
        }

        let halves: Vec<_> = value.split('-').map(str::trim).collect();
        match halves.as_slice() {
            [single] => Ok(Self::Single(parse_half(single)?.0)),
            [start, end] => {
                let (start, start_has_unit) = parse_half(start)?;
                let (end, end_has_unit) = parse_half(end)?;
                if start_has_unit && !end_has_unit {
                    bail!("invalid elevation");
                }
                Ok(Self::Range(start, end))
            }
            _ => bail!("invalid elevation"),
        }
    }
}

impl fmt::Display for CoffeeElevation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Single(value) => write!(formatter, "{value} masl"),
            Self::Range(start, end) => write!(formatter, "{start} - {end} masl"),
        }
    }
}

impl Serialize for CoffeeElevation {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for CoffeeElevation {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

impl rusqlite::types::ToSql for CoffeeElevation {
    fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
        Ok(self.to_string().into())
    }
}

impl rusqlite::types::FromSql for CoffeeElevation {
    fn column_result(value: rusqlite::types::ValueRef<'_>) -> rusqlite::types::FromSqlResult<Self> {
        value.as_str()?.parse().map_err(|error: anyhow::Error| {
            rusqlite::types::FromSqlError::Other(Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                error.to_string(),
            )))
        })
    }
}

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
        if version > LATEST_VERSION {
            bail!(
                "database schema version {version} is newer than supported version {LATEST_VERSION}"
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
            if version > LATEST_VERSION {
                bail!(
                    "database schema version {version} is newer than supported version {LATEST_VERSION}"
                );
            }
            if version == 0 {
                let transaction = self.connection.transaction()?;
                transaction.execute(
                    "ALTER TABLE Coffee ADD COLUMN archived INTEGER NOT NULL DEFAULT 0 CHECK(archived IN (0, 1))",
                    [],
                )?;
                transaction.execute_batch(BREW_SCHEMA)?;
                transaction.pragma_update(None, "user_version", LATEST_VERSION)?;
                transaction.commit()?;
            }
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
            transaction.pragma_update(None, "user_version", LATEST_VERSION)?;
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
        let row =
            self.connection
                .query_row(
                    "SELECT added, name, roast_date, roaster, origin, producer, farm, elevation,
                        variety, process, flavours, archived
                 FROM Coffee WHERE label = ?1",
                    [label],
                    |row| {
                        Ok((
                            row.get::<_, DateTime<Local>>(0)?,
                            CoffeeData {
                                name: row.get(1)?,
                                roast_date: row.get(2)?,
                                roaster: row.get(3)?,
                                origin: row.get(4)?,
                                producer: row.get(5)?,
                                farm: row.get(6)?,
                                elevation: row.get(7)?,
                                variety: row.get(8)?,
                                process: row.get(9)?,
                                flavours: serde_json::from_str(&row.get::<_, String>(10)?)
                                    .map_err(|error| {
                                        rusqlite::Error::FromSqlConversionFailure(
                                            10,
                                            Type::Text,
                                            Box::new(error),
                                        )
                                    })?,
                            },
                            row.get::<_, bool>(11)?,
                        ))
                    },
                )
                .optional()?;
        let Some((added, data, archived)) = row else {
            return Ok(None);
        };
        Ok(Some(Coffee {
            label,
            added,
            archived,
            data,
        }))
    }

    pub fn update(
        &mut self,
        label: Label,
        data: CoffeeData,
        new_roaster_url: Option<&Url>,
    ) -> Result<Coffee> {
        let data = data.validate()?;
        let transaction = self.connection.transaction()?;
        let added: DateTime<Local> = transaction
            .query_row(
                "SELECT added FROM Coffee WHERE label = ?1",
                [label],
                |row| row.get(0),
            )
            .optional()?
            .ok_or_else(|| anyhow!("coffee '{label}' does not exist"))?;
        ensure_roaster(&transaction, &data.roaster, new_roaster_url)?;
        let flavours = serde_json::to_string(&data.flavours)?;
        let changed = transaction.execute(
            "UPDATE Coffee SET name=?2, roast_date=?3, roaster=?4, origin=?5,
             producer=?6, farm=?7, elevation=?8, variety=?9, process=?10,
             flavours=?11 WHERE label=?1",
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
                flavours
            ],
        )?;
        debug_assert_eq!(changed, 1);
        register_flavours(&transaction, &data.flavours)?;
        let archived = transaction.query_row(
            "SELECT archived FROM Coffee WHERE label=?1",
            [label],
            |row| row.get(0),
        )?;
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
            .query_map([active_only], |row| {
                let flavours =
                    serde_json::from_str(&row.get::<_, String>(11)?).map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(11, Type::Text, Box::new(error))
                    })?;
                Ok(Coffee {
                    label: row.get(0)?,
                    added: row.get(1)?,
                    archived: row.get(12)?,
                    data: CoffeeData {
                        name: row.get(2)?,
                        roast_date: row.get(3)?,
                        roaster: row.get(4)?,
                        origin: row.get(5)?,
                        producer: row.get(6)?,
                        farm: row.get(7)?,
                        elevation: row.get(8)?,
                        variety: row.get(9)?,
                        process: row.get(10)?,
                        flavours,
                    },
                })
            })?
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
                "SELECT name, method FROM BrewDevices WHERE label=?1",
                [label.as_str()],
                |row| {
                    let method = row
                        .get::<_, String>(1)?
                        .parse()
                        .map_err(|e: anyhow::Error| sql_conversion(1, e))?;
                    Ok(BrewDevice {
                        label: label.clone(),
                        name: row.get(0)?,
                        method,
                    })
                },
            )
            .optional()?)
    }

    pub fn list_devices(&self) -> Result<Vec<BrewDevice>> {
        let mut statement = self
            .connection
            .prepare("SELECT label, name, method FROM BrewDevices ORDER BY label")?;
        Ok(statement
            .query_map([], |row| {
                let raw: String = row.get(0)?;
                let label = raw
                    .parse()
                    .map_err(|e: anyhow::Error| sql_conversion(0, e))?;
                let raw_method: String = row.get(2)?;
                let method = raw_method
                    .parse()
                    .map_err(|e: anyhow::Error| sql_conversion(2, e))?;
                Ok(BrewDevice {
                    label,
                    name: row.get(1)?,
                    method,
                })
            })?
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
                params![
                    device.label.as_str(),
                    device.name,
                    device.method.to_string()
                ],
            )?;
        }
        let device = tx
            .query_row(
                "SELECT method FROM BrewDevices WHERE label=?1",
                [data.device.as_str()],
                |r| r.get::<_, String>(0),
            )
            .optional()?
            .ok_or_else(|| anyhow!("device '{}' does not exist", data.device))?;
        let method: BrewMethod = device.parse()?;
        if method != data.metadata.method() {
            bail!("brew metadata does not match device method '{method}'");
        }
        if let Some(grinder) = new_grinder {
            if grinder.name.is_empty() {
                bail!("grinder name must not be empty");
            }
            tx.execute(
                "INSERT INTO Grinders(label,name) VALUES(?1,?2)",
                params![grinder.label.as_str(), grinder.name],
            )?;
        }
        let metadata = data.metadata.to_json()?;
        tx.execute("INSERT INTO Brews(coffee,device,grinder,method,occurred,temperature_c,coffee_grams,grind_setting,notes,metadata) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)", params![coffee, data.device.as_str(), data.grinder.as_str(), method.to_string(), data.occurred, data.temperature_c, data.coffee_grams, data.grind_setting, data.notes, metadata])?;
        let id = tx.last_insert_rowid();
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
        let brews = statement.query_map(params![coffee, limit], |row| {
            let method_raw: String = row.get(3)?;
            let method: BrewMethod = method_raw
                .parse()
                .map_err(|e: anyhow::Error| sql_conversion(3, e))?;
            let metadata_raw: String = row.get(10)?;
            let metadata = crate::brew::BrewMetadata::from_json(method, &metadata_raw)
                .map_err(|e| sql_conversion(10, e))?;
            let occurred = row.get(4)?;
            let device_raw: String = row.get(2)?;
            let device = device_raw
                .parse()
                .map_err(|e: anyhow::Error| sql_conversion(2, e))?;
            Ok(Brew {
                id: row.get(0)?,
                coffee: row.get(1)?,
                data: BrewData {
                    device,
                    occurred,
                    temperature_c: row.get(5)?,
                    coffee_grams: row.get(6)?,
                    grinder: row
                        .get::<_, String>(7)?
                        .parse()
                        .map_err(|e: anyhow::Error| sql_conversion(7, e))?,
                    grind_setting: row.get(8)?,
                    notes: row.get(9)?,
                    metadata,
                },
            })
        })?;
        for brew in brews {
            callback(brew?);
        }
        Ok(())
    }
}

fn sql_conversion(column: usize, error: anyhow::Error) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        column,
        Type::Text,
        Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            error.to_string(),
        )),
    )
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
    let flavours = serde_json::to_string(&data.flavours)?;
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
            flavours
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
}
