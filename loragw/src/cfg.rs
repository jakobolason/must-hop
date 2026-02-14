use super::{
    error::{AppError, Error},
    types::{BoardConf, ChannelConf, FrontRadio, RadioType, RxRFConf, TxGain},
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
    pub tx_gains: Option<Vec<TxGain>>,
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
}

impl TryFrom<Board> for BoardConf {
    type Error = AppError;
    fn try_from(other: Board) -> Result<BoardConf, Self::Error> {
        Ok(Self {
            lorawan_public: other.lorawan_public,
            clksrc: FrontRadio::try_from(other.clksrc)?,
            spidev_path: other.spidev_path,
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

// #[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
// pub struct TxGain {
//     #[serde(rename(serialize = "dbm", deserialize = "dbm"))]
//     pub rf_power: i8,
//     #[serde(rename(serialize = "dig", deserialize = "dig"))]
//     pub dig_gain: u8,
//     #[serde(rename(serialize = "pa", deserialize = "pa"))]
//     pub pa_gain: u8,
//     #[serde(rename(serialize = "mix", deserialize = "mix"))]
//     pub mix_gain: u8,
// }
