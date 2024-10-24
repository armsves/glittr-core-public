use super::*;

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Flaw {
    // parse tx
    InvalidInstruction(String),
    InvalidScript,
    FailedDeserialization,

    // call type
    MessageInvalid,
    ContractNotMatch,
    ContractNotFound,
    WriteError,

    // call type::mint
    SupplyCapExceeded,

    // asset contract
    OverflowAmountPerMint,
}
