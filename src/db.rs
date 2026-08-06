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
"#;

pub struct Database {
    connection: Connection,
}

impl Database {
    pub fn open(path: &Path) -> Result<Self> {
        let connection = Connection::open(path)
            .with_context(|| format!("failed to open database '{}'", path.display()))?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        let database = Self { connection };
        database.initialize()?;
        Ok(database)
    }

    #[cfg(test)]
    fn in_memory() -> Result<Self> {
        let connection = Connection::open_in_memory()?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        let database = Self { connection };
        database.initialize()?;
        Ok(database)
    }

    fn initialize(&self) -> Result<()> {
        let has_tables: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table')",
            [],
            |row| row.get(0),
        )?;
        if !has_tables {
            self.connection.execute_batch(SCHEMA)?;
        }
        Ok(())
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
                "SELECT name, roast_date, roaster, origin, producer, farm, variety, process
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
                        variety: row.get(6)?,
                        process: row.get(7)?,
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
        let transaction = self.connection.transaction()?;
        ensure_roaster(&transaction, &data.roaster, new_roaster_url)?;
        let changed = transaction.execute(
            "UPDATE Coffee SET name=?2, roast_date=?3, roaster=?4, origin=?5,
             producer=?6, farm=?7, variety=?8, process=?9 WHERE label=?1",
            params![
                label,
                data.name,
                data.roast_date,
                data.roaster,
                data.origin,
                data.producer,
                data.farm,
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
         (label, name, roast_date, roaster, origin, producer, farm, variety, process)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            label,
            data.name,
            data.roast_date,
            data.roaster,
            data.origin,
            data.producer,
            data.farm,
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
        replacement.flavours = vec!["Cocoa".into()];
        database.update(coffee.label, replacement.clone(), None)?;
        assert_eq!(database.get(coffee.label)?.unwrap().data, replacement);
        Ok(())
    }
}
