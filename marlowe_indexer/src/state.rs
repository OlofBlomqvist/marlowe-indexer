use std::collections::HashMap;
use pallas_primitives::byron::BlockId;

#[derive(Debug,PartialEq,Hash,Eq)]
pub(crate) struct OutputReference { pub tx_hash: String, pub output_index : u64 }
pub(crate) type ContractId = String;
pub(crate) type SlotId = u64;


#[derive(Debug,PartialEq)]
pub (crate) struct OrderedContracts {
    pub contracts_in_order: Vec<Contract>,
    pub lookup_table: HashMap<ContractId, usize>,

    // usize is the index of the contract the output reference refers to, and f64 is the block_slot
    // so that we can use it for when doing rollbacks
    pub utxo_to_contract_lookup_table: HashMap<OutputReference, (usize, f64)>,
}

impl OrderedContracts {
    pub fn new() -> Self {
        
        // todo : might want to allow tuning the initial reservations thru config or params later 
        
        let mut hashly : HashMap<String,usize> = HashMap::new();
        hashly.reserve(10_000);

        let mut veccy : Vec<Contract> = Vec::new();
        veccy.reserve(10_000);

        OrderedContracts {
            contracts_in_order: veccy,
            lookup_table: hashly,
            utxo_to_contract_lookup_table: HashMap::new()
        }
    }

    // DROPS CONTRACTS, TRANSITIONS AND OR OUTPUT REFS WHERE SLOT > SLOT_NUM
    pub fn rollback(&mut self, slot_num: f64) {

        // DROP ALL TRANSITIONS AFTER THE SPECIFIED SLOT; AND COLLECT ALL CONTRACT IDS THAT END UP HAVING ZERO TRANSACTIONS
        // SINCE THAT MEANS EVEN THE INITIAL TX GOT ROLLED BACK, SO WE CAN DROP IT!
        let contracts_to_remove: Vec<ContractId> = self.contracts_in_order
            .iter_mut()
            .filter_map(|contract| {
                contract.transitions.retain(|utxo| utxo.slot <= slot_num);
                if contract.transitions.is_empty() {
                    Some(contract.id.clone())
                } else {
                    None
                }
            })
            .collect();

        // DROP THE ROLLBACKED CONTRACTS THAT WERE CREATED AFTER THE SPECIFIED SLOT
        self.contracts_in_order.retain(|contract| !contracts_to_remove.contains(&contract.id));

        // DROP REFERENCES THAT WERE CREATED AFTER THE SPECIFIED SLOT
        self.utxo_to_contract_lookup_table.retain(|_,v| v.1 > slot_num);

        // CLEAN UP LOOKUP TABLE SO WE DONT KEEP REFERENCES TO NONEXISTANT 
        for contract_id in &contracts_to_remove {
            self.lookup_table.remove(contract_id);
        }

        // UPDATE INDEXES FOR ALL REMAINING CONTRACTS
        for (index, contract) in self.contracts_in_order.iter().enumerate() {
            self.lookup_table.insert(contract.id.clone(), index);
        }

    }
    
    pub fn contract_count(&self) -> usize {
        self.contracts_in_order.len()
    }

    pub fn insert(&mut self, contract: Contract) {
        
        let id = contract.short_id.clone();
        self.contracts_in_order.push(contract);

        let index = self.contracts_in_order.len() - 1;
        self.lookup_table.insert(id, index);

    }

    #[allow(dead_code)]
    pub fn remove(&mut self, id: &ContractId) -> Option<Contract> {
        if let Some(&index) = self.lookup_table.get(id) {
            
            let contract = self.contracts_in_order.remove(index);

            for (_, idx) in self.lookup_table.iter_mut() {
                if *idx > index {
                    *idx -= 1;
                }
            }

            self.lookup_table.remove(id);            

            Some(contract)
        } else {
            None
        }
    }

    pub fn get_mut(&mut self, id: &ContractId) -> Option<&mut Contract> {
        self.lookup_table.get(id).map(|&index| &mut self.contracts_in_order[index])
    }

    #[allow(dead_code)]
    pub fn get(&self, id: &ContractId) -> Option<&Contract> {
        self.lookup_table.get(id).map(|&index| &self.contracts_in_order[index])
    }
    
}

#[derive(Debug,Clone,async_graphql::SimpleObject,PartialEq)]
pub struct Contract {
    /// + slot id of the block in which this contract was initialized 
    /// + index of the tx in that block 
    /// + index of the output containing the contract inside the tx.
    /// ... converted to base58.
    pub short_id: String, 
    
    /// hash id of the tx that initially created this contract instance
    pub id:String,

    // todo: current contract state avail here as well.
    // a: resolved once ?
    // b: resolved on first access ?
    // c: shortcut to last tx ?

    /// All states in the contract, in order of first to last.
    pub transitions: Vec<MarloweTransition>
}

/// This represents a single UTXO inside of a transaction which 
/// changes the state of a Marlowe contract on chain.
/// One can either initialize multiple contracts in a single tx,
/// or "step" a single contract state. It is not possible to step multiple contracts
/// or Initialize a contract while also stepping another contract.
#[derive(Debug,Clone,async_graphql::SimpleObject,PartialEq)]
pub struct MarloweTransition {
    pub datum : String, // todo - more useful info here..
    pub redeemer : String, // todo - more useful info here..
    pub tx_id : String, // index of the tx which created this utxo

    /// Index of the UTXO that caused this transition. None if this transition finalized the contract (eg. no output was made to the Marlowe validator) 
    pub utxo_index : Option<f64>, 

    /// This is true if there can not be any more transitions after this one.
    pub end : bool,

    /// Slot number in which this transition occurred
    pub slot : f64,    

    /// Block in which this transition occurred
    pub block : String,

    // Number of the block in which this transition occurred
    pub block_num : f64,

    // Invalid contracts are filtered out by default when querying
    pub invalid : bool,

    // List of issues found when processing the transaction that caused this transition
    pub issues : Vec<String>,

    // TEMP
    pub marlowe_scan_status: Option<String>
}




#[derive(Debug)]
pub struct State {
    pub(crate) ordered_contracts: OrderedContracts,
    pub (crate) last_block_slot: Option<SlotId>,
    pub (crate) last_block_hash: Option<BlockId>,

    // TODO : add tip info and such ?
}

impl State {
    pub fn last_seen_block(&self) -> &Option<BlockId> {
        &self.last_block_hash
    }
    pub fn new() -> Self {
        State {
            ordered_contracts : OrderedContracts::new(),
            last_block_slot: None,
            last_block_hash: None
        }
    }
}



