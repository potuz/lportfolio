use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

#[derive(
    Clone,
    Copy,
    Debug,
    Eq,
    Hash,
    Ord,
    PartialEq,
    PartialOrd,
    Serialize,
    Deserialize,
    clap::ValueEnum,
)]
#[serde(rename_all = "lowercase")]
#[clap(rename_all = "lower")]
pub enum Chain {
    Mainnet,
    Arbitrum,
    Optimism,
    Base,
}

impl Chain {
    pub const ALL: &'static [Chain] = &[Self::Mainnet, Self::Arbitrum, Self::Optimism, Self::Base];

    pub fn id(self) -> u64 {
        match self {
            Self::Mainnet => 1,
            Self::Arbitrum => 42161,
            Self::Optimism => 10,
            Self::Base => 8453,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Mainnet => "mainnet",
            Self::Arbitrum => "arbitrum",
            Self::Optimism => "optimism",
            Self::Base => "base",
        }
    }

    pub fn rpc_env_key(self) -> &'static str {
        match self {
            Self::Mainnet => "LPORTFOLIO_RPC_MAINNET",
            Self::Arbitrum => "LPORTFOLIO_RPC_ARBITRUM",
            Self::Optimism => "LPORTFOLIO_RPC_OPTIMISM",
            Self::Base => "LPORTFOLIO_RPC_BASE",
        }
    }

    pub fn from_id(id: u64) -> Option<Self> {
        Self::ALL.iter().copied().find(|c| c.id() == id)
    }
}

impl fmt::Display for Chain {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl FromStr for Chain {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let lower = s.to_ascii_lowercase();
        for c in Self::ALL {
            if c.name() == lower {
                return Ok(*c);
            }
        }
        if let Ok(id) = lower.parse::<u64>()
            && let Some(c) = Self::from_id(id)
        {
            return Ok(c);
        }
        Err(format!("unknown chain: {s}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_unique_and_lowercase() {
        for c in Chain::ALL {
            assert_eq!(c.name(), c.name().to_ascii_lowercase());
        }
        let mut names: Vec<_> = Chain::ALL.iter().map(|c| c.name()).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), Chain::ALL.len());
    }

    #[test]
    fn from_str_accepts_name_and_id() {
        assert_eq!(Chain::from_str("mainnet").unwrap(), Chain::Mainnet);
        assert_eq!(Chain::from_str("MAINNET").unwrap(), Chain::Mainnet);
        assert_eq!(Chain::from_str("1").unwrap(), Chain::Mainnet);
        assert_eq!(Chain::from_str("42161").unwrap(), Chain::Arbitrum);
        assert!(Chain::from_str("polygon").is_err());
    }
}
