use std::fmt;

use super::*;
use asset_contract::AssetContract;
use bitcoin::{
    opcodes,
    script::{self, Instruction, PushBytes},
    OutPoint, ScriptBuf, Transaction,
};
use bitcoincore_rpc::jsonrpc::serde_json::{self, Deserializer};
use constants::OP_RETURN_MAGIC_PREFIX;
use flaw::Flaw;

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub enum ContractType {
    Asset(AssetContract),
}

#[serde_with::skip_serializing_none]
#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct MintOption {
    pub pointer: u32,
    pub oracle_message: Option<OracleMessageSigned>,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub enum CallType {
    Mint(MintOption),
    Burn,
    Swap,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct OracleMessageSigned {
    pub signature: Vec<u8>,
    pub message: OracleMessage,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct OracleMessage {
    /// the input_outpoint dictates which UTXO is being evaluated by the Oracle
    pub input_outpoint: OutPoint,
    /// min_in_value represents what the input valued at (minimum because btc value could differ (-fee))
    pub min_in_value: u128,
    /// out_value represents the oracle's valuation of the input e.g. for 1 btc == 72000 wusd, 72000 is the out_value
    pub out_value: u128,
    /// rune's BlockTx if rune
    pub asset_id: Option<String>,
    // the bitcoin block height when the message is signed
    pub block_height: u64,
}

/// Transfer
/// Asset: This is a block:tx reference to the contract where the asset was created
/// Output index of output to receive asset
/// Amount: value assigning shares of the transfer to the appropriate UTXO output
#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub struct TxTypeTransfer {
    pub asset: BlockTxTuple,
    pub output: u32,
    pub amount: U128,
}

// TxTypes: Transfer, ContractCreation, ContractCall
#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub struct Transfer {
    pub transfers: Vec<TxTypeTransfer>,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub struct ContractCreation {
    pub contract_type: ContractType,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub struct ContractCall {
    pub contract: BlockTxTuple,
    pub call_type: CallType,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde_with::skip_serializing_none]
pub struct OpReturnMessage {
    pub transfer: Option<Transfer>,
    pub contract_creation: Option<ContractCreation>,
    pub contract_call: Option<ContractCall>,
}

impl CallType {
    pub fn validate(&self) -> Option<Flaw> {
        None
    }
}

impl OpReturnMessage {
    pub fn parse_tx(tx: &Transaction) -> Result<OpReturnMessage, Flaw> {
        let mut payload = Vec::new();

        for output in tx.output.iter() {
            let mut instructions = output.script_pubkey.instructions();

            if instructions.next() != Some(Ok(Instruction::Op(opcodes::all::OP_RETURN))) {
                continue;
            }

            let signature = instructions.next();
            if let Some(Ok(Instruction::PushBytes(glittr_message))) = signature {
                if glittr_message.as_bytes() != OP_RETURN_MAGIC_PREFIX.as_bytes() {
                    continue;
                }
            } else {
                continue;
            }

            for result in instructions {
                match result {
                    Ok(Instruction::PushBytes(push)) => {
                        payload.extend_from_slice(push.as_bytes());
                    }
                    Ok(Instruction::Op(op)) => {
                        return Err(Flaw::InvalidInstruction(op.to_string()));
                    }
                    Err(_) => {
                        return Err(Flaw::InvalidScript);
                    }
                }
            }
            break;
        }

        if payload.is_empty() {
            return Err(Flaw::NonGlittrMessage);
        }

        let message =
            OpReturnMessage::deserialize(&mut Deserializer::from_slice(payload.as_slice()));

        match message {
            Ok(message) => Ok(message),
            Err(_) => Err(Flaw::FailedDeserialization),
        }
    }

    pub fn validate(&self) -> Option<Flaw> {
        if let Some(contract_creation) = &self.contract_creation {
            match &contract_creation.contract_type {
                ContractType::Asset(asset_contract) => {
                    return asset_contract.validate();
                }
            }
        }

        if let Some(contract_call) = &self.contract_call {
            return contract_call.call_type.validate();
        }

        None
    }

    pub fn into_script(&self) -> ScriptBuf {
        let mut builder = script::Builder::new().push_opcode(opcodes::all::OP_RETURN);
        let magic_prefix: &PushBytes = OP_RETURN_MAGIC_PREFIX.as_bytes().try_into().unwrap();
        let binding = self.to_string();
        let script_bytes: &PushBytes = binding.as_bytes().try_into().unwrap();

        builder = builder.push_slice(magic_prefix);
        builder = builder.push_slice(script_bytes);

        builder.into_script()
    }
}

impl fmt::Display for OpReturnMessage {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", serde_json::to_string(&self).unwrap())
    }
}

#[cfg(test)]
mod test {
    use bitcoin::consensus::deserialize;
    use bitcoin::{locktime, transaction::Version, Amount, Transaction, TxOut};

    use crate::asset_contract::DistributionSchemes;
    use crate::asset_contract::FreeMint;
    use crate::asset_contract::SimpleAsset;
    use crate::transaction::asset_contract::AssetContract;
    use crate::transaction::message::ContractType;
    use crate::U128;

    use super::{ContractCreation, OpReturnMessage};

    #[test]
    pub fn parse_op_return_message_success() {
        let dummy_message = OpReturnMessage {
            transfer: None,
            contract_creation: Some(ContractCreation {
                contract_type: ContractType::Asset(AssetContract {
                    asset: SimpleAsset {
                        supply_cap: Some(U128(1000)),
                        divisibility: 18,
                        live_time: 0,
                    },
                    distribution_schemes: DistributionSchemes {
                        free_mint: Some(FreeMint {
                            supply_cap: Some(U128(1000)),
                            amount_per_mint: U128(10),
                        }),
                        preallocated: None,
                        purchase: None,
                    },
                }),
            }),
            contract_call: None,
        };

        let tx = Transaction {
            input: Vec::new(),
            lock_time: locktime::absolute::LockTime::ZERO,
            output: vec![TxOut {
                script_pubkey: dummy_message.into_script(),
                value: Amount::from_int_btc(0),
            }],
            version: Version(2),
        };

        let parsed = OpReturnMessage::parse_tx(&tx);

        if let Some(contract_creation) = parsed.unwrap().contract_creation {
            match contract_creation.contract_type {
                ContractType::Asset(asset_contract) => {
                    let free_mint = asset_contract.distribution_schemes.free_mint.unwrap();
                    assert_eq!(asset_contract.asset.supply_cap, Some(U128(1000)));
                    assert_eq!(asset_contract.asset.divisibility, 18);
                    assert_eq!(asset_contract.asset.live_time, 0);
                    assert_eq!(free_mint.supply_cap, Some(U128(1000)));
                    assert_eq!(free_mint.amount_per_mint, U128(10));
                }
            }
        }
    }

    #[test]
    pub fn validate_op_return_message_from_tx_hex_success() {
        let tx_bytes = hex::decode("02000000018fadc57a81127e5c69373de674bd48d874b13698199d4ed2841a9da53714c5960000000000ffffffff02cac0000000000000736a06474c495454524c697b2274785f74797065223a7b22636f6e74726163745f63616c6c223a7b22636f6e7472616374223a5b332c315d2c2263616c6c5f74797065223a7b226d696e74223a7b22706f696e746572223a302c226f7261636c655f6d657373616765223a6e756c6c7d7d7d7d7d2202000000000000160014cbc188f7a134a6d52afca200090171fb7ab171a600000000").unwrap();

        let tx: Transaction = deserialize(&tx_bytes).unwrap();

        let op_return_message = OpReturnMessage::parse_tx(&tx);

        assert!(op_return_message.is_ok());
    }
}
