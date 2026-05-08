use std::time::Duration;

use anyhow::{Context, Result, bail};
use reqwest::Client;
use serde::{Deserialize, Deserializer, de::Error as DeError};

use crate::chain::Chain;

const ENDPOINT: &str = "https://api.etherscan.io/v2/api";
const PAGE_SIZE: u32 = 10_000;

pub struct Explorer {
    api_key: String,
    http: Client,
}

#[derive(Debug, Clone, Deserialize)]
pub struct EtherscanTx {
    #[serde(rename = "blockNumber", deserialize_with = "de_u64_str")]
    pub block_number: u64,
    #[serde(rename = "timeStamp", deserialize_with = "de_u64_str")]
    pub timestamp: u64,
    pub hash: String,
    pub from: String,
    #[serde(default)]
    pub to: String,
    pub value: String,
    #[serde(default)]
    pub input: String,
    #[serde(rename = "txreceipt_status", default)]
    pub receipt_status: String,
    #[serde(rename = "isError", default)]
    pub is_error: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct EtherscanTokenTx {
    #[serde(rename = "blockNumber", deserialize_with = "de_u64_str")]
    pub block_number: u64,
    #[serde(rename = "timeStamp", deserialize_with = "de_u64_str")]
    pub timestamp: u64,
    pub hash: String,
    #[serde(rename = "contractAddress")]
    pub contract_address: String,
    pub from: String,
    pub to: String,
    pub value: String,
    #[serde(rename = "tokenSymbol", default)]
    pub token_symbol: String,
    #[serde(rename = "tokenDecimal", default, deserialize_with = "de_u64_str_opt")]
    pub token_decimal: u64,
}

trait HasBlockNumber {
    fn block_number(&self) -> u64;
}

impl HasBlockNumber for EtherscanTx {
    fn block_number(&self) -> u64 {
        self.block_number
    }
}

impl HasBlockNumber for EtherscanTokenTx {
    fn block_number(&self) -> u64 {
        self.block_number
    }
}

#[derive(Deserialize)]
struct EtherscanResp {
    status: String,
    message: String,
    result: serde_json::Value,
}

impl Explorer {
    pub fn new(api_key: String) -> Result<Self> {
        if api_key.trim().is_empty() {
            bail!("LPORTFOLIO_ETHERSCAN_API_KEY is not set");
        }
        let http = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .context("building http client")?;
        Ok(Self { api_key, http })
    }

    pub async fn txlist(
        &self,
        chain: Chain,
        address: &str,
        from_block: u64,
    ) -> Result<Vec<EtherscanTx>> {
        self.fetch_account("txlist", chain, address, from_block)
            .await
    }

    pub async fn tokentx(
        &self,
        chain: Chain,
        address: &str,
        from_block: u64,
    ) -> Result<Vec<EtherscanTokenTx>> {
        self.fetch_account("tokentx", chain, address, from_block)
            .await
    }

    async fn fetch_account<T>(
        &self,
        action: &str,
        chain: Chain,
        address: &str,
        from_block: u64,
    ) -> Result<Vec<T>>
    where
        T: serde::de::DeserializeOwned + HasBlockNumber,
    {
        let mut all: Vec<T> = Vec::new();
        let mut start = from_block;

        loop {
            let resp: EtherscanResp = self
                .http
                .get(ENDPOINT)
                .query(&[
                    ("chainid", chain.id().to_string()),
                    ("module", "account".to_string()),
                    ("action", action.to_string()),
                    ("address", address.to_string()),
                    ("startblock", start.to_string()),
                    ("endblock", "99999999".to_string()),
                    ("page", "1".to_string()),
                    ("offset", PAGE_SIZE.to_string()),
                    ("sort", "asc".to_string()),
                    ("apikey", self.api_key.clone()),
                ])
                .send()
                .await
                .context("etherscan request failed")?
                .error_for_status()
                .context("etherscan returned http error")?
                .json()
                .await
                .context("parsing etherscan response")?;

            if resp.status == "0" && resp.message == "No transactions found" {
                break;
            }
            if resp.status != "1" {
                let msg = match &resp.result {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                bail!("etherscan {action} error ({}): {msg}", resp.message);
            }

            let batch: Vec<T> = serde_json::from_value(resp.result)
                .with_context(|| format!("etherscan {action} schema mismatch"))?;
            let n = batch.len();
            if n == 0 {
                break;
            }
            let max_block = batch
                .iter()
                .map(HasBlockNumber::block_number)
                .max()
                .unwrap_or(start);

            all.extend(batch);

            if n < PAGE_SIZE as usize {
                break;
            }
            // advance: next page resumes at max_block; duplicates are deduped on insert
            start = max_block;
        }
        Ok(all)
    }
}

fn de_u64_str<'de, D: Deserializer<'de>>(de: D) -> Result<u64, D::Error> {
    let s: String = String::deserialize(de)?;
    s.parse::<u64>().map_err(D::Error::custom)
}

fn de_u64_str_opt<'de, D: Deserializer<'de>>(de: D) -> Result<u64, D::Error> {
    let s: String = String::deserialize(de)?;
    if s.is_empty() {
        return Ok(0);
    }
    s.parse::<u64>().map_err(D::Error::custom)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tx_response_shape() {
        let body = r#"{
            "status": "1",
            "message": "OK",
            "result": [{
                "blockNumber": "12345",
                "timeStamp": "1700000000",
                "hash": "0xabc",
                "from": "0xfrom",
                "to": "0xto",
                "value": "1000000000000000000",
                "input": "0x",
                "txreceipt_status": "1",
                "isError": "0"
            }]
        }"#;
        let resp: EtherscanResp = serde_json::from_str(body).unwrap();
        assert_eq!(resp.status, "1");
        let txs: Vec<EtherscanTx> = serde_json::from_value(resp.result).unwrap();
        assert_eq!(txs.len(), 1);
        assert_eq!(txs[0].block_number, 12345);
        assert_eq!(txs[0].timestamp, 1_700_000_000);
        assert_eq!(txs[0].value, "1000000000000000000");
    }

    #[test]
    fn parses_tokentx_response_shape() {
        let body = r#"{
            "status": "1",
            "message": "OK",
            "result": [{
                "blockNumber": "20000000",
                "timeStamp": "1710000000",
                "hash": "0xdef",
                "contractAddress": "0xtoken",
                "from": "0xfrom",
                "to": "0xto",
                "value": "1000000",
                "tokenSymbol": "USDC",
                "tokenDecimal": "6"
            }]
        }"#;
        let resp: EtherscanResp = serde_json::from_str(body).unwrap();
        let txs: Vec<EtherscanTokenTx> = serde_json::from_value(resp.result).unwrap();
        assert_eq!(txs.len(), 1);
        assert_eq!(txs[0].token_symbol, "USDC");
        assert_eq!(txs[0].token_decimal, 6);
        assert_eq!(txs[0].contract_address, "0xtoken");
    }

    #[test]
    fn rejects_missing_api_key() {
        assert!(Explorer::new(String::new()).is_err());
        assert!(Explorer::new("   ".to_string()).is_err());
    }
}
