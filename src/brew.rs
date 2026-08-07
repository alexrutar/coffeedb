use std::{fmt, str::FromStr};

use anyhow::{Result, bail};
use chrono::{DateTime, FixedOffset, Local};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};

use crate::label::Label;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DeviceLabel(String);

impl DeviceLabel {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for DeviceLabel {
    type Err = anyhow::Error;
    fn from_str(value: &str) -> Result<Self> {
        let valid = !value.is_empty()
            && value.split('-').all(|part| {
                !part.is_empty()
                    && part
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
            });
        if !valid {
            bail!("device label must match [a-z0-9]+(?:-[a-z0-9]+)*");
        }
        Ok(Self(value.into()))
    }
}

impl fmt::Display for DeviceLabel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum BrewMethod {
    Pourover,
    Immersion,
    Espresso,
}

impl fmt::Display for BrewMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Pourover => "pourover",
            Self::Immersion => "immersion",
            Self::Espresso => "espresso",
        })
    }
}

impl FromStr for BrewMethod {
    type Err = anyhow::Error;
    fn from_str(value: &str) -> Result<Self> {
        match value {
            "pourover" => Ok(Self::Pourover),
            "immersion" => Ok(Self::Immersion),
            "espresso" => Ok(Self::Espresso),
            _ => bail!("unknown brew method '{value}'"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PouroverMetadata {
    pub pours: Vec<(f64, f64)>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImmersionMetadata {
    pub volume_ml: f64,
    pub immersion_seconds: f64,
    pub drawdown_seconds: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EspressoMetadata {
    pub beverage_grams: f64,
    pub duration_seconds: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BrewMetadata {
    Pourover(PouroverMetadata),
    Immersion(ImmersionMetadata),
    Espresso(EspressoMetadata),
}

impl BrewMetadata {
    pub fn method(&self) -> BrewMethod {
        match self {
            Self::Pourover(_) => BrewMethod::Pourover,
            Self::Immersion(_) => BrewMethod::Immersion,
            Self::Espresso(_) => BrewMethod::Espresso,
        }
    }
    pub fn from_json(method: BrewMethod, json: &str) -> Result<Self> {
        Ok(match method {
            BrewMethod::Pourover => Self::Pourover(serde_json::from_str(json)?),
            BrewMethod::Immersion => Self::Immersion(serde_json::from_str(json)?),
            BrewMethod::Espresso => Self::Espresso(serde_json::from_str(json)?),
        })
    }
    pub fn to_json(&self) -> Result<String> {
        Ok(match self {
            Self::Pourover(v) => serde_json::to_string(v)?,
            Self::Immersion(v) => serde_json::to_string(v)?,
            Self::Espresso(v) => serde_json::to_string(v)?,
        })
    }
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn total_volume_ml(&self) -> Option<f64> {
        match self {
            Self::Pourover(v) => Some(v.pours.iter().map(|(_, ml)| ml).sum()),
            _ => None,
        }
    }
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn brew_ratio(&self, dose: f64) -> Option<f64> {
        match self {
            Self::Espresso(v) => Some(v.beverage_grams / dose),
            _ => None,
        }
    }
    fn validate(&self) -> Result<()> {
        match self {
            Self::Pourover(v) => {
                if v.pours.is_empty() {
                    bail!("'pours' must contain at least one pour");
                }
                let mut previous = None;
                for &(elapsed, volume) in &v.pours {
                    if !elapsed.is_finite() || elapsed < 0.0 {
                        bail!("pour elapsed times must be finite and nonnegative");
                    }
                    if previous.is_some_and(|p| elapsed <= p) {
                        bail!("pour elapsed times must be strictly increasing");
                    }
                    positive("incremental pour volumes", volume)?;
                    previous = Some(elapsed);
                }
            }
            Self::Immersion(v) => {
                positive("volume_ml", v.volume_ml)?;
                positive("immersion_seconds", v.immersion_seconds)?;
                positive("drawdown_seconds", v.drawdown_seconds)?;
            }
            Self::Espresso(v) => {
                positive("beverage_grams", v.beverage_grams)?;
                positive("duration_seconds", v.duration_seconds)?;
            }
        }
        Ok(())
    }
}

fn positive(name: &str, value: f64) -> Result<()> {
    if !value.is_finite() || value <= 0.0 {
        bail!("'{name}' must be finite and positive");
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq)]
pub struct BrewData {
    pub device: DeviceLabel,
    pub occurred: DateTime<FixedOffset>,
    pub temperature_c: f64,
    pub coffee_grams: f64,
    pub grinder: String,
    pub grind_setting: String,
    pub notes: Option<String>,
    pub metadata: BrewMetadata,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Common<T> {
    device: DeviceLabel,
    occurred: DateTime<FixedOffset>,
    temperature_c: f64,
    coffee_grams: f64,
    grinder: String,
    grind_setting: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    notes: Option<String>,
    #[serde(flatten)]
    metadata: T,
}

impl BrewData {
    pub fn parse_for_method(text: &str, method: BrewMethod) -> Result<Self> {
        macro_rules! parse {
            ($ty:ty, $variant:ident) => {{
                let v: Common<$ty> = toml::from_str(text)?;
                Self {
                    device: v.device,
                    occurred: v.occurred,
                    temperature_c: v.temperature_c,
                    coffee_grams: v.coffee_grams,
                    grinder: v.grinder,
                    grind_setting: v.grind_setting,
                    notes: v.notes,
                    metadata: BrewMetadata::$variant(v.metadata),
                }
            }};
        }
        let value = match method {
            BrewMethod::Pourover => parse!(PouroverMetadata, Pourover),
            BrewMethod::Immersion => parse!(ImmersionMetadata, Immersion),
            BrewMethod::Espresso => parse!(EspressoMetadata, Espresso),
        };
        value.validate()
    }
    pub fn validate(self) -> Result<Self> {
        if !self.temperature_c.is_finite()
            || self.temperature_c <= 0.0
            || self.temperature_c > 100.0
        {
            bail!("'temperature_c' must be finite and in (0, 100]");
        }
        positive("coffee_grams", self.coffee_grams)?;
        for (name, value) in [
            ("grinder", self.grinder.as_str()),
            ("grind_setting", self.grind_setting.as_str()),
        ] {
            if value.is_empty() {
                bail!("'{name}' must not be empty");
            }
        }
        if self.notes.as_deref() == Some("") {
            bail!("'notes' is empty; delete it when unused");
        }
        self.metadata.validate()?;
        Ok(self)
    }
    pub fn draft(device: &DeviceLabel, method: BrewMethod) -> String {
        let occurred = Local::now().fixed_offset().to_rfc3339();
        let metadata = match method {
            BrewMethod::Pourover => "pours = [[0, 0]]\n",
            BrewMethod::Immersion => "volume_ml = 0\nimmersion_seconds = 0\ndrawdown_seconds = 0\n",
            BrewMethod::Espresso => "beverage_grams = 0\nduration_seconds = 0\n",
        };
        format!(
            "# Fill in all required fields. Delete notes if unused.\ndevice = \"{device}\"\noccurred = \"{occurred}\"\ntemperature_c = 0\ncoffee_grams = 0\ngrinder = \"\"\ngrind_setting = \"\"\nnotes = \"\"\n{metadata}"
        )
    }
    pub fn to_toml(&self) -> Result<String> {
        macro_rules! ser {
            ($v:expr) => {
                toml::to_string_pretty(&Common {
                    device: self.device.clone(),
                    occurred: self.occurred,
                    temperature_c: self.temperature_c,
                    coffee_grams: self.coffee_grams,
                    grinder: self.grinder.clone(),
                    grind_setting: self.grind_setting.clone(),
                    notes: self.notes.clone(),
                    metadata: $v.clone(),
                })?
            };
        }
        Ok(match &self.metadata {
            BrewMetadata::Pourover(v) => ser!(v),
            BrewMetadata::Immersion(v) => ser!(v),
            BrewMetadata::Espresso(v) => ser!(v),
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Brew {
    pub id: i64,
    pub coffee: Label,
    pub data: BrewData,
}

impl Brew {
    pub fn to_toml(&self) -> Result<String> {
        let mut table: toml::Table = toml::from_str(&self.data.to_toml()?)?;
        table.insert("id".into(), self.id.into());
        table.insert("coffee".into(), self.coffee.to_string().into());
        table.insert(
            "method".into(),
            self.data.metadata.method().to_string().into(),
        );
        Ok(toml::to_string_pretty(&table)?)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrewDevice {
    pub label: DeviceLabel,
    pub name: String,
    pub method: BrewMethod,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn device_labels() {
        for good in ["aeropress", "flair-58", "origami-m", "1"] {
            assert!(good.parse::<DeviceLabel>().is_ok());
        }
        for bad in ["", "AeroPress", "-v60", "v60-", "v_60", "two--words"] {
            assert!(bad.parse::<DeviceLabel>().is_err(), "{bad}");
        }
    }
    #[test]
    fn parses_and_derives() {
        let d = BrewData::parse_for_method("device='v60'\noccurred='2026-01-01T12:00:00+01:00'\ntemperature_c=95\ncoffee_grams=15\ngrinder='K'\ngrind_setting='5'\npours=[[0,40],[30,60]]", BrewMethod::Pourover).unwrap();
        assert_eq!(d.metadata.total_volume_ml(), Some(100.0));
        assert!(
            BrewMetadata::from_json(BrewMethod::Pourover, &d.metadata.to_json().unwrap()).unwrap()
                == d.metadata
        );
        let e = BrewMetadata::Espresso(EspressoMetadata {
            beverage_grams: 36.0,
            duration_seconds: 28.0,
        });
        assert_eq!(e.brew_ratio(18.0), Some(2.0));
    }
}
