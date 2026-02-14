use super::{
    error::{AppError, Error},
    types::{BoardConf, ChannelConf, ComType, FrontRadio, RadioType, RxRFConf, TxGain},
};
use serde::{Deserialize, Serialize};
use std::{convert::TryFrom, ffi::CString};
use toml;

static DEFAULT_CFG_TOML: &str = include_str!("./default_config_sx1302.toml");

/// Represents top-level configuration document.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    pub board: Board,
    pub radios: Option<Vec<Radio>>,
    pub multirate_channels: Option<Vec<MultirateLoraChannel>>,
    pub tx_gains: Option<Vec<ConfTxGain>>,
}

impl Config {
    pub fn from_str_or_default(cfg: Option<&str>) -> Result<Self, Error> {
        Self::from_str(cfg.unwrap_or(DEFAULT_CFG_TOML))
    }

    pub fn from_str(cfg: &str) -> Result<Self, Error> {
        Ok(toml::from_str(cfg)?)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Board {
    pub lorawan_public: bool,
    pub clksrc: u32,
    pub spidev_path: CString,
    pub com_type: ComType,
}

impl TryFrom<Board> for BoardConf {
    type Error = AppError;
    fn try_from(other: Board) -> Result<BoardConf, Self::Error> {
        Ok(Self {
            lorawan_public: other.lorawan_public,
            clksrc: FrontRadio::try_from(other.clksrc)?,
            spidev_path: other.spidev_path,
            com_type: other.com_type,
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Radio {
    // TODO: Make id an enum? { Radio1, Radio2 } to avoid trys
    pub id: u32,
    pub freq: u32,
    pub rssi_offset: f32,
    #[serde(rename(serialize = "type", deserialize = "type"))]
    pub type_: String,
    pub tx_enable: bool,
}

impl TryFrom<Radio> for RxRFConf {
    type Error = AppError;
    fn try_from(other: Radio) -> Result<Self, Self::Error> {
        Ok(RxRFConf {
            radio: FrontRadio::try_from(other.id)?,
            enable: true,
            freq: other.freq,
            rssi_offset: other.rssi_offset,
            type_: RadioType::try_from(other.type_.as_ref())?,
            tx_enable: other.tx_enable,
            tx_notch_freq: 0,
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct MultirateLoraChannel {
    radio: u32,
    #[serde(rename(serialize = "if", deserialize = "if"))]
    if_: i32,
}

impl TryFrom<&MultirateLoraChannel> for ChannelConf {
    type Error = AppError;
    fn try_from(other: &MultirateLoraChannel) -> Result<ChannelConf, Self::Error> {
        Ok(ChannelConf::Multirate {
            radio: FrontRadio::try_from(other.radio)?,
            freq: other.if_,
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ConfTxGain {
    #[serde(rename(serialize = "dbm", deserialize = "dbm"))]
    pub rf_power: i8,
    #[serde(rename(serialize = "dig", deserialize = "dig"))]
    pub dig_gain: u8,
    #[serde(rename(serialize = "pa", deserialize = "pa"))]
    pub pa_gain: u8,
    #[serde(rename(serialize = "mix", deserialize = "mix"))]
    pub mix_gain: u8,
}
impl From<ConfTxGain> for TxGain {
    fn from(conf: ConfTxGain) -> Self {
        TxGain {
            rf_power: conf.rf_power,
            pa_gain: conf.pa_gain,
            mix_gain: conf.mix_gain,
            dig_gain: conf.dig_gain,

            // Initialize other fields that might exist in TxGain but not in TOML
            // For example, if using SX1250, you might need pwr_idx, or dac_gain:
            dac_gain: 3, // Default value (example)
            pwr_id: 0,   // Default value (example)
            offset_i: 0,
            offset_q: 0,
        }
    }
}
