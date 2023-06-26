use std::collections::HashMap;
use pallas_primitives::byron::BlockId;

pub(crate) type ContractId = String;
pub(crate) type SlotId = u64;

type TxId = String;
type TxOutRef = (String,u64);


#[derive(Debug,Clone)]
pub struct MarloweUtxo {
    /// todo : store raw bytes 
    pub redeemer: String, 
    /// todo : store raw bytes
    pub datum : String, 
    /// txhash of transaction that created this utxo.
    pub id : TxId,   
    /// index of this output in the transaction.
    pub utxo_index : u64, 
    pub slot : SlotId,
    /// block in which this utxo was created 
    pub block_hash : BlockId, 
    /// did the contract end with the tx that created this utxo?
    pub end : bool ,
    /// index of the transaction that created this utxo in the block
    pub tx_index : u64,
    /// number of the block in which this utxo was created
    pub block_num : u64
}

#[derive(Debug)]
pub struct State {
    pub (crate) contracts: HashMap<ContractId, HashMap<TxOutRef,MarloweUtxo>>,
    pub (crate) last_block_slot: Option<SlotId>,
    pub (crate) last_block_hash: Option<BlockId>
}

impl State {
    pub fn contracts(&self) -> &HashMap<ContractId, HashMap<TxOutRef,MarloweUtxo>> {
        &self.contracts
    }
    pub fn last_seen_block(&self) -> &Option<BlockId> {
        &self.last_block_hash
    }
    pub fn new() -> Self {
        State {
            contracts: HashMap::new(),
            last_block_slot: None,
            last_block_hash: None
        }
    }
}



