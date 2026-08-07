use std::{borrow::Cow, fmt, str::FromStr};

use anyhow::{Result, anyhow, bail};
use chrono::{DateTime, Local, NaiveDate, Utc};
use rusqlite::{
    Row, ToSql,
    types::{FromSql, FromSqlError, FromSqlResult, ToSqlOutput, Value, ValueRef},
};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de::DeserializeOwned};

use crate::{
    brew::{Brew, BrewData, BrewDevice, BrewMetadata, BrewMethod, DeviceLabel, GrinderLabel},
    label::Label,
    record::{Coffee, CoffeeData},
};

/// A decoded row from the `Coffee` table.
pub(super) struct CoffeeRow {
    pub label: Label,
    pub added: DateTime<Local>,
    pub name: String,
    pub roast_date: Option<NaiveDate>,
    pub roaster: String,
    pub origin: Option<String>,
    pub producer: Option<String>,
    pub farm: Option<String>,
    pub elevation: Option<CoffeeElevation>,
    pub variety: Option<String>,
    pub process: Option<String>,
    pub flavours: Vec<String>,
    pub archived: bool,
}

impl CoffeeRow {
    pub fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            label: row.get(0)?,
            added: row.get(1)?,
            name: row.get(2)?,
            roast_date: row.get(3)?,
            roaster: row.get(4)?,
            origin: row.get(5)?,
            producer: row.get(6)?,
            farm: row.get(7)?,
            elevation: row.get(8)?,
            variety: row.get(9)?,
            process: row.get(10)?,
            flavours: row.get::<_, Json<Vec<String>>>(11)?.0,
            archived: row.get(12)?,
        })
    }
}

impl From<CoffeeRow> for Coffee {
    fn from(row: CoffeeRow) -> Self {
        Self {
            label: row.label,
            added: row.added,
            archived: row.archived,
            data: CoffeeData {
                name: row.name,
                roast_date: row.roast_date,
                roaster: row.roaster,
                origin: row.origin,
                producer: row.producer,
                farm: row.farm,
                elevation: row.elevation,
                variety: row.variety,
                process: row.process,
                flavours: row.flavours,
            },
        }
    }
}

/// A decoded row from the `BrewDevices` table.
pub(super) struct BrewDeviceRow(pub BrewDevice);

impl BrewDeviceRow {
    pub fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self(BrewDevice {
            label: row.get(0)?,
            name: row.get(1)?,
            method: row.get(2)?,
        }))
    }
}

/// A decoded row from the `Brews` table.
pub(super) struct BrewRow(pub Brew);

impl BrewRow {
    pub fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        let method = row.get(3)?;
        let metadata = BrewMetadata::from_json(method, row.get_ref(10)?.as_str()?)
            .map_err(|error| conversion_error(10, error))?;
        Ok(Self(Brew {
            id: row.get(0)?,
            coffee: row.get(1)?,
            data: BrewData {
                device: row.get(2)?,
                occurred: row.get::<_, DateTime<Utc>>(4)?,
                temperature_c: row.get(5)?,
                coffee_grams: row.get(6)?,
                grinder: row.get(7)?,
                grind_setting: row.get(8)?,
                notes: row.get(9)?,
                metadata,
            },
        }))
    }
}

fn conversion_error(column: usize, error: anyhow::Error) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        column,
        rusqlite::types::Type::Text,
        Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            error.to_string(),
        )),
    )
}

/// An elevation stored in the `Coffee.elevation` text column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoffeeElevation {
    Single(u16),
    Range(u16, u16),
}

impl FromStr for CoffeeElevation {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        fn parse_half(value: &str) -> Result<(u16, bool)> {
            let mut parts = value.split_whitespace();
            let number = parts.next().ok_or_else(|| anyhow!("invalid elevation"))?;
            let unit = parts.next();
            if parts.next().is_some() || unit.is_some_and(|unit| unit != "masl") {
                bail!("invalid elevation");
            }
            let number = number
                .parse::<u16>()
                .map_err(|_| anyhow!("invalid elevation: '{number}' does not fit in a u16"))?;
            Ok((number, unit.is_some()))
        }

        let mut halves = value.split('-').map(str::trim);
        let first = halves.next().ok_or_else(|| anyhow!("invalid elevation"))?;
        match (halves.next(), halves.next()) {
            (None, None) => Ok(Self::Single(parse_half(first)?.0)),
            (Some(second), None) => {
                let (start, start_has_unit) = parse_half(first)?;
                let (end, end_has_unit) = parse_half(second)?;
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
        Cow::<'de, str>::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

impl ToSql for CoffeeElevation {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(self.to_string().into())
    }
}

impl FromSql for CoffeeElevation {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        parse_text(value)
    }
}

// Labels already own their strings, so their SQLite representation can be borrowed.
impl ToSql for DeviceLabel {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(self.as_str().into())
    }
}

impl FromSql for DeviceLabel {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        parse_text(value)
    }
}

impl ToSql for GrinderLabel {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(self.as_str().into())
    }
}

impl FromSql for GrinderLabel {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        parse_text(value)
    }
}

impl ToSql for BrewMethod {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        let text = match self {
            Self::Pourover => "pourover",
            Self::Immersion => "immersion",
            Self::Espresso => "espresso",
        };
        Ok(text.into())
    }
}

impl FromSql for BrewMethod {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        parse_text(value)
    }
}

impl ToSql for BrewMetadata {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        let json = match self {
            BrewMetadata::Pourover(value) => serde_json::to_string(value),
            BrewMetadata::Immersion(value) => serde_json::to_string(value),
            BrewMetadata::Espresso(value) => serde_json::to_string(value),
        };
        json.map(Value::Text)
            .map(ToSqlOutput::Owned)
            .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))
    }
}

fn parse_text<T>(value: ValueRef<'_>) -> FromSqlResult<T>
where
    T: FromStr,
    T::Err: fmt::Display,
{
    value.as_str()?.parse::<T>().map_err(|error| {
        FromSqlError::Other(Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            error.to_string(),
        )))
    })
}

/// A serde value stored as JSON text in a SQLite column.
pub(super) struct Json<T>(pub T);

impl<T: DeserializeOwned> FromSql for Json<T> {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        serde_json::from_str(value.as_str()?)
            .map(Self)
            .map_err(|error| FromSqlError::Other(Box::new(error)))
    }
}

/// A borrowed serde value written as JSON text to a SQLite column.
pub(super) struct JsonRef<'a, T: ?Sized>(pub &'a T);

impl<'a, T: Serialize + ?Sized> JsonRef<'a, T> {
    pub fn new(value: &'a T) -> Self {
        Self(value)
    }
}

impl<T: Serialize + ?Sized> ToSql for JsonRef<'_, T> {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        serde_json::to_string(self.0)
            .map(Value::Text)
            .map(ToSqlOutput::Owned)
            .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))
    }
}
