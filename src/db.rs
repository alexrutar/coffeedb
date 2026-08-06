use std::{num::NonZero, path::Path};

use anyhow::{Context, Result, anyhow, bail};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};

use crate::{
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
  name TEXT NOT NULL,
  roast_date TEXT,
  roaster TEXT NOT NULL REFERENCES Roasters(name),
  origin TEXT,
  producer TEXT,
  farm TEXT,
  elevation TEXT,
  variety TEXT,
  process TEXT
) STRICT;

CREATE TABLE Flavours (
  description TEXT NOT NULL PRIMARY KEY
) STRICT;

CREATE TABLE CoffeeFlavours (
  coffee TEXT NOT NULL REFERENCES Coffee(label) ON DELETE CASCADE,
  flavour TEXT NOT NULL REFERENCES Flavours(description),
  PRIMARY KEY (coffee, flavour)
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
CREATE INDEX CoffeeFlavours_flavour_idx ON CoffeeFlavours(flavour);
CREATE INDEX Coffee_label_order_idx ON Coffee(
  CASE substr(label, 1, 1) WHEN 'E' THEN 0 WHEN 'F' THEN 1 ELSE 2 END,
  CAST(substr(label, 2) AS INTEGER)
);
"#;

// First four bytes of SHA-256("coffeedb"), interpreted as a big-endian integer.
const APPLICATION_ID: i32 = 0x75af_f574;

pub struct Database {
    connection: Connection,
}

impl Database {
    pub fn open(path: &Path) -> Result<Self> {
        let is_new = !path
            .try_exists()
            .with_context(|| format!("failed to inspect database path '{}'", path.display()))?;
        let connection = Connection::open(path)
            .with_context(|| format!("failed to open database '{}'", path.display()))?;
        let mut database = Self { connection };
        database.initialize(is_new)?;
        database
            .connection
            .pragma_update(None, "foreign_keys", "ON")?;
        Ok(database)
    }

    #[cfg(test)]
    fn in_memory() -> Result<Self> {
        let connection = Connection::open_in_memory()?;
        let mut database = Self { connection };
        database.initialize(true)?;
        database
            .connection
            .pragma_update(None, "foreign_keys", "ON")?;
        Ok(database)
    }

    fn initialize(&mut self, is_new: bool) -> Result<()> {
        if is_new {
            let transaction = self.connection.transaction()?;
            transaction.execute_batch(SCHEMA)?;
            transaction.pragma_update(None, "application_id", APPLICATION_ID)?;
            transaction.commit()?;
            return Ok(());
        }

        let application_id: i32 =
            self.connection
                .pragma_query_value(None, "application_id", |row| row.get(0))?;
        match application_id {
            APPLICATION_ID => Ok(()),
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
        new_roaster_url: Option<&str>,
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
        insert_coffee(&transaction, label, &data)?;
        transaction.commit()?;
        Ok(Coffee { label, data })
    }

    pub fn get(&self, label: Label) -> Result<Option<Coffee>> {
        let data = self
            .connection
            .query_row(
                "SELECT name, roast_date, roaster, origin, producer, farm, elevation, variety, process
                 FROM Coffee WHERE label = ?1",
                [label],
                |row| {
                    Ok(CoffeeData {
                        name: row.get(0)?,
                        roast_date: row.get(1)?,
                        roaster: row.get(2)?,
                        origin: row.get(3)?,
                        producer: row.get(4)?,
                        farm: row.get(5)?,
                        elevation: row.get(6)?,
                        variety: row.get(7)?,
                        process: row.get(8)?,
                        flavours: Vec::new(),
                    })
                },
            )
            .optional()?;
        let Some(mut data) = data else {
            return Ok(None);
        };
        let mut statement = self
            .connection
            .prepare("SELECT flavour FROM CoffeeFlavours WHERE coffee = ?1 ORDER BY rowid")?;
        data.flavours = statement
            .query_map([label], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(Some(Coffee { label, data }))
    }

    pub fn update(
        &mut self,
        label: Label,
        data: CoffeeData,
        new_roaster_url: Option<&str>,
    ) -> Result<Coffee> {
        let data = data.validate()?;
        let transaction = self.connection.transaction()?;
        ensure_roaster(&transaction, &data.roaster, new_roaster_url)?;
        let changed = transaction.execute(
            "UPDATE Coffee SET name=?2, roast_date=?3, roaster=?4, origin=?5,
             producer=?6, farm=?7, elevation=?8, variety=?9, process=?10 WHERE label=?1",
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
                data.process
            ],
        )?;
        if changed == 0 {
            bail!("coffee '{label}' does not exist");
        }
        transaction.execute("DELETE FROM CoffeeFlavours WHERE coffee = ?1", [label])?;
        insert_flavours(&transaction, label, &data.flavours)?;
        transaction.commit()?;
        Ok(Coffee { label, data })
    }

    pub fn list(&self) -> Result<Vec<Coffee>> {
        let mut statement = self.connection.prepare(
            "SELECT label FROM Coffee
             ORDER BY CASE substr(label, 1, 1) WHEN 'E' THEN 0 WHEN 'F' THEN 1 ELSE 2 END,
                      CAST(substr(label, 2) AS INTEGER)",
        )?;
        let labels = statement
            .query_map([], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<Label>>>()?;
        labels
            .into_iter()
            .map(|label| {
                self.get(label)?
                    .ok_or_else(|| anyhow!("coffee '{label}' disappeared while listing"))
            })
            .collect()
    }

    pub fn list_flavours(&self) -> Result<Vec<String>> {
        let mut statement = self
            .connection
            .prepare("SELECT description FROM Flavours ORDER BY description")?;
        Ok(statement
            .query_map([], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?)
    }
}

fn ensure_roaster(transaction: &Transaction<'_>, name: &str, url: Option<&str>) -> Result<()> {
    transaction.execute(
        "INSERT OR IGNORE INTO Roasters (name, url) VALUES (?1, ?2)",
        params![name, url],
    )?;
    Ok(())
}

fn insert_coffee(transaction: &Transaction<'_>, label: Label, data: &CoffeeData) -> Result<()> {
    transaction.execute(
        "INSERT INTO Coffee
         (label, name, roast_date, roaster, origin, producer, farm, elevation, variety, process)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
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
            data.process
        ],
    )?;
    insert_flavours(transaction, label, &data.flavours)
}

fn insert_flavours(transaction: &Transaction<'_>, label: Label, flavours: &[String]) -> Result<()> {
    for flavour in flavours {
        transaction.execute(
            "INSERT OR IGNORE INTO Flavours (description) VALUES (?1)",
            [flavour],
        )?;
        transaction.execute(
            "INSERT INTO CoffeeFlavours (coffee, flavour) VALUES (?1, ?2)",
            params![label, flavour],
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
        let coffee = database.add(CoffeeType::Omni, data("Old"), Some("https://example.com"))?;
        let mut replacement = data("New");
        replacement.flavours = vec!["cocoa".into()];
        database.update(coffee.label, replacement.clone(), None)?;
        assert_eq!(database.get(coffee.label)?.unwrap().data, replacement);
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
        let mut database = Database { connection };
        let error = database.initialize(false).unwrap_err();
        assert!(error.to_string().contains("application_id is not set"));
        Ok(())
    }
}
