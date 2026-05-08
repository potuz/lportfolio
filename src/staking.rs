use std::time::Duration;

use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Deserializer, de::Error as DeError};

#[derive(Debug, Clone)]
pub struct StakingSummary {
    pub validator_count: u64,
    pub total_balance_gwei: u64,
}

pub struct BeaconNodeClient {
    base_url: String,
    http: Client,
}

#[derive(Debug, Deserialize)]
struct ValidatorsEnvelope {
    data: Vec<ValidatorEntry>,
}

#[derive(Debug, Deserialize)]
struct ValidatorEntry {
    #[serde(deserialize_with = "de_u64_str")]
    balance: u64,
}

impl BeaconNodeClient {
    pub fn new(base_url: String) -> Result<Self> {
        let http = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .context("building http client")?;
        Ok(Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            http,
        })
    }

    pub async fn validator_balances(&self, indices: &[u64]) -> Result<StakingSummary> {
        if indices.is_empty() {
            return Ok(StakingSummary {
                validator_count: 0,
                total_balance_gwei: 0,
            });
        }
        let id_param = indices
            .iter()
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let url = format!(
            "{}/eth/v1/beacon/states/head/validators?id={}",
            self.base_url, id_param
        );
        let resp: ValidatorsEnvelope = self
            .http
            .get(&url)
            .send()
            .await
            .with_context(|| format!("beacon node GET {url} failed"))?
            .error_for_status()
            .with_context(|| format!("beacon node {url} returned non-2xx"))?
            .json()
            .await
            .with_context(|| format!("parsing beacon node response from {url}"))?;

        let total: u64 = resp.data.iter().map(|v| v.balance).sum();
        Ok(StakingSummary {
            validator_count: resp.data.len() as u64,
            total_balance_gwei: total,
        })
    }
}

fn de_u64_str<'de, D: Deserializer<'de>>(de: D) -> Result<u64, D::Error> {
    let s: String = String::deserialize(de)?;
    s.parse::<u64>().map_err(D::Error::custom)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_validators_envelope() {
        let body = r#"{
            "execution_optimistic": false,
            "finalized": true,
            "data": [
                { "index": "7654",  "balance": "32500000000", "status": "active_ongoing" },
                { "index": "43867", "balance": "32100000000", "status": "active_ongoing" }
            ]
        }"#;
        let envelope: ValidatorsEnvelope = serde_json::from_str(body).unwrap();
        assert_eq!(envelope.data.len(), 2);
        let total: u64 = envelope.data.iter().map(|v| v.balance).sum();
        assert_eq!(total, 64_600_000_000);
    }

    #[test]
    fn parses_empty_envelope() {
        let body = r#"{ "data": [] }"#;
        let envelope: ValidatorsEnvelope = serde_json::from_str(body).unwrap();
        assert!(envelope.data.is_empty());
    }
}
