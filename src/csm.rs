use alloy::primitives::{Address, U256, address};
use alloy::providers::{DynProvider, Provider, ProviderBuilder};
use alloy::sol;
use anyhow::{Context, Result};

const CS_ACCOUNTING: Address = address!("0x4d72BFF1BeaC69925F8Bd12526a39BAAb069e5Da");

sol! {
    #[sol(rpc)]
    interface ICSAccounting {
        /// Current bond amount for a node operator, denominated in stETH (wei).
        function getBond(uint256 nodeOperatorId) external view returns (uint256);
    }
}

#[derive(Debug, Clone)]
pub struct CsmBond {
    pub operator_id: u64,
    pub bond_steth_wei: U256,
}

pub struct CsmReader {
    provider: DynProvider,
}

impl CsmReader {
    pub fn connect(mainnet_rpc_url: &str) -> Result<Self> {
        let url = mainnet_rpc_url
            .parse()
            .with_context(|| format!("invalid mainnet RPC URL: {mainnet_rpc_url}"))?;
        let provider = ProviderBuilder::new().connect_http(url).erased();
        Ok(Self { provider })
    }

    pub async fn read_bond(&self, operator_id: u64) -> Result<CsmBond> {
        let contract = ICSAccounting::new(CS_ACCOUNTING, &self.provider);
        let bond_steth_wei = contract
            .getBond(U256::from(operator_id))
            .call()
            .await
            .with_context(|| format!("CSAccounting.getBond({operator_id}) failed"))?;
        Ok(CsmBond {
            operator_id,
            bond_steth_wei,
        })
    }

    pub async fn read_bonds(&self, operator_ids: &[u64]) -> Result<Vec<CsmBond>> {
        let mut out = Vec::with_capacity(operator_ids.len());
        for &id in operator_ids {
            out.push(self.read_bond(id).await?);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cs_accounting_address_constant() {
        // Sanity-check that the constant decodes as an address (catch typos at compile time).
        assert_eq!(CS_ACCOUNTING.0.0.len(), 20);
    }
}
