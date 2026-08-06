use std::{fmt, num::NonZero, str::FromStr};

use anyhow::{Result, bail};
use clap::ValueEnum;
use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ToSqlOutput, ValueRef};
use rusqlite::{ToSql, types::Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, ValueEnum)]
pub enum CoffeeType {
    #[value(alias = "e")]
    Espresso,
    #[value(alias = "f")]
    Filter,
    #[value(alias = "o")]
    Omni,
}

impl CoffeeType {
    pub const fn short(self) -> char {
        match self {
            Self::Espresso => 'E',
            Self::Filter => 'F',
            Self::Omni => 'O',
        }
    }

    fn from_short(short: u8) -> Option<Self> {
        match short {
            b'E' => Some(Self::Espresso),
            b'F' => Some(Self::Filter),
            b'O' => Some(Self::Omni),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Label {
    kind: CoffeeType,
    index: NonZero<u32>,
}

impl Label {
    pub const fn new(kind: CoffeeType, index: NonZero<u32>) -> Self {
        Self { kind, index }
    }

    #[cfg(test)]
    pub const fn kind(self) -> CoffeeType {
        self.kind
    }

    #[cfg(test)]
    pub const fn index(self) -> NonZero<u32> {
        self.index
    }
}

impl fmt::Display for Label {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", self.kind.short(), self.index)
    }
}

impl FromStr for Label {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        let bytes = value.as_bytes();
        if bytes.len() < 2 {
            bail!("label must have the form E1, F1, or O1");
        }
        let Some(kind) = CoffeeType::from_short(bytes[0]) else {
            bail!("label prefix must be E, F, or O");
        };
        let digits = &value[1..];
        if digits.starts_with('0') || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
            bail!("label index must be a positive decimal integer without leading zeroes");
        }
        let index = digits.parse::<NonZero<u32>>()?;
        Ok(Self { kind, index })
    }
}

impl ToSql for Label {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::Owned(Value::Text(self.to_string())))
    }
}

impl FromSql for Label {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        value
            .as_str()?
            .parse()
            .map_err(|error: anyhow::Error| FromSqlError::Other(error.into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_canonical_labels() {
        let label: Label = "F23".parse().unwrap();
        assert_eq!(label.kind(), CoffeeType::Filter);
        assert_eq!(label.index().get(), 23);
        assert_eq!(label.to_string(), "F23");
    }

    #[test]
    fn rejects_noncanonical_labels() {
        for label in ["", "E", "X1", "e1", "E0", "E01", "E-1", "E1x"] {
            assert!(label.parse::<Label>().is_err(), "accepted {label}");
        }
    }
}
