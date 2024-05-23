use std::collections::HashMap;
use crossterm::terminal;
use marlowe_lang::plutus_data;
use marlowe_lang::plutus_data::{ToPlutusDataDerive, FromPlutusDataDerive};
use serde::Deserialize;
use sled::IVec;
use tokio::sync::RwLock;
use tracing::debug;


#[derive(Debug,PartialEq,Hash,Eq,Clone)]
pub(crate) struct OutputReference { pub tx_hash: String, pub output_index : u64 }
pub(crate) type ContractShortId = String;
pub(crate) type SlotId = u64;


#[derive(Debug,Clone,ToPlutusDataDerive,FromPlutusDataDerive)]
pub struct Contract {

    /// + slot id of the block in which this contract was initialized 
    /// + index of the tx in that block 
    /// + index of the output containing the contract inside the tx.
    /// ... converted to base58.
    pub short_id: String, 

    /// "tx_hash#utxo_id" of the tx that initially created this contract instance
    pub id: String,

    /// All states in the contract, in order of first to last.
    pub transitions: Vec<MarloweTransition>,

    /// Each version of the Marlowe validator script has its own hash.
    pub validator_hash : String
}

impl Contract {
    pub fn get_out_ref_if_still_active(&self) -> Option<OutputReference> {
        let last_tx = self.transitions.last().unwrap();
        if last_tx.end {
            None
        } else {
            Some(OutputReference {
                output_index: last_tx.utxo_index.u64(),
                tx_hash: last_tx.tx_id.clone()
            })
        }
    }
}

#[derive(Clone,Debug,ToPlutusDataDerive,FromPlutusDataDerive,Deserialize)]
pub struct GraphQLHackForu64 {
    data : String
}
impl std::fmt::Display for GraphQLHackForu64 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.data)
    }
}

impl GraphQLHackForu64 {
    pub fn f64(&self) -> f64 {
        self.data.parse().unwrap()
    }
    pub fn u64(&self) -> u64 {
        self.data.parse().unwrap()
    }
    pub fn from_usize(x:usize) -> Self {
        GraphQLHackForu64 {
            data : x.to_string()
        }
    }
    pub fn from_f64(x:f64) -> Self {
        GraphQLHackForu64 {
            data : x.to_string()
        }
    }
}
pub struct OptionGraphQLHackForu64(Option<GraphQLHackForu64>);
// Implement conversion from OptionGraphQLHackForu64 to Option<f64>
impl From<OptionGraphQLHackForu64> for Option<f64> {
    fn from(item: OptionGraphQLHackForu64) -> Self {
        item.0.map(|val| {
            val.data.parse::<f64>().unwrap_or_default()
        })
    }
}
impl PartialEq<u64> for GraphQLHackForu64 {
    fn eq(&self, other: &u64) -> bool {
        self.data == other.to_string()
    }
}
impl PartialEq<GraphQLHackForu64> for u64 {
    fn eq(&self, other: &GraphQLHackForu64) -> bool {
        other == self
    }
}
// Implement conversion from u64 to GraphQLHackForu64
impl From<u64> for GraphQLHackForu64 {
    fn from(num: u64) -> Self {
        GraphQLHackForu64 {
            data: num.to_string(),
        }
    }
}

// Implement conversion from GraphQLHackForu64 to u64
impl From<GraphQLHackForu64> for u64 {
    fn from(item: GraphQLHackForu64) -> Self {
        item.data.parse().unwrap_or(0) // You might want to handle parse errors more gracefully
    }
}
/// This represents a single UTXO inside of a transaction which 
/// changes the state of a Marlowe contract on chain.
/// One can either initialize multiple contracts in a single tx,
/// or "step" a single contract state. It is not possible to step multiple contracts
/// or Initialize a contract while also stepping another contract.
#[derive(Debug,Clone,ToPlutusDataDerive,FromPlutusDataDerive)]
pub struct MarloweTransition {

    #[cfg(feature="debug")]
    pub validity_range: (Option<u64>,Option<u64>),

    pub tx_id : String, // index of the tx which created this utxo

    /// Index of the UTXO that caused this transition.
    pub utxo_index : GraphQLHackForu64, 

    /// This is true if there can not be any more transitions after this one.
    /// It signifies that the transaction causing this transition did not output any new
    /// utxo to the marlowe validator address, effectively ending the contract's utxo chain.
    pub end : bool,

    /// Slot number in which this transition occurred
    pub abs_slot : GraphQLHackForu64,    

    /// Block in which this transition occurred
    pub block : String,

    // Number of the block in which this transition occurred
    pub block_num : GraphQLHackForu64,

    // TEMP
    pub marlowe_scan_status: Option<String>,

    pub datum : Option<marlowe_lang::types::marlowe::MarloweDatum>,
    pub datum_hash : Option<String>,

    pub inputs : Option<Vec<marlowe_lang::types::marlowe::PossiblyMerkleizedInput>>,
    
    pub meta : Option<String>,

    // For contracts that use merkleized continuations, this property will be populated when a transition uses merkleized inputs. 
    // Todo: wipe intermediate continuations
    #[cfg(feature="debug")] pub continuations : std::collections::HashMap<String,marlowe_lang::types::marlowe::Contract>,

    // -- Original data so we can validate that marlowe-rs gets to the same result after applying inputs
    // -- These are optional because merkleized contracts and transactions that use datum-hash rather than inline datums
    // -- will not have them available until the utxo is consumed.
    #[cfg(feature="debug")] pub original_plutus_datum_bytes : Option<Vec<u8>>, // for memory reasons, we ONLY index these when using the debug feature
    #[cfg(feature="debug")] pub original_redeemer_bytes : Option<Vec<u8>>      // for memory reasons, we ONLY index these when using the debug feature
}


#[derive(Debug)]
pub struct MarloweState {

    state_db : sled::Tree,
    
    contracts_db: sled::Tree,

    contracts_mem_cache: RwLock<HashMap<String,Contract>>,
    contracts_long_to_short_lookup : RwLock<HashMap<String,ContractShortId>>,
    out_ref_to_short_id_lookup: RwLock<HashMap<OutputReference,(SlotId,ContractShortId)>>,

    pub (crate) broadcast_tx: 
        (
            tokio::sync::broadcast::Sender<super::MarloweSubscriptionEvent>,
            tokio::sync::broadcast::Receiver<super::MarloweSubscriptionEvent>
        )
}


impl MarloweState {
    
    
    pub fn ivec_to_string(v:IVec) -> String {
        v.as_ascii().unwrap().as_str().to_owned()
    }

    pub async fn rollback(&self,slot_num: u64, block_hash:Option<String>) -> Result<(),sled::transaction::TransactionError<sled::transaction::TransactionError>> {
        
        let mut to_be_replaced : Vec<(IVec,Contract)> = vec![];
        let mut to_be_removed : Vec<(IVec,String)> = vec![];
        
        // drop refs to anything later than slot_num
        let mut guard =  self.out_ref_to_short_id_lookup.write().await;
        guard.retain(|_k,(slot,_short_id)| *slot > slot_num);

        self.contracts_db.iter().for_each(|item| {
            match item {
                Ok((k,v)) => {
                    let key = Self::ivec_to_string(k.clone());
                    let mut val : Contract = marlowe_lang::plutus_data::from_bytes_specific(&v).unwrap();
                    let pre_count = val.transitions.len();
                    val.transitions.retain(|x|u64::from(x.abs_slot.clone()) <= slot_num);

                    if val.transitions.len() == 0 {
                        debug!("rollback causes contract {key} to be completely removed.");
                        to_be_removed.push((k,key))
                    } else if val.transitions.len() != pre_count {
                        debug!("rollback causes contract {key} to have less transitions.");
                        to_be_replaced.push((k,val))
                    }
                    
                },
                Err(e) => {
                    panic!("rollback failed. possibly due to db corruption? {e:?}")
                },
            }
        });

        self.contracts_db.transaction(|t| {

            for (k,_) in &to_be_removed {
                t.remove(k)?;
            }

            for (k,v) in &to_be_replaced {
                let serialized = v.to_plutus_data(&vec![]).expect("serialization should always work");
                let val = marlowe_lang::plutus_data::to_bytes(&serialized).unwrap();
                _ = t.insert(k, val).unwrap();
            }

            Ok::<(), sled::transaction::ConflictableTransactionError<sled::transaction::TransactionError>>(())

        }).expect("rollbacks must always succeed");

        // re-initialize mem cache
        self.init_mem_cache().await;

        self.set_last_seen_slot_num(slot_num);
        self.set_last_seen_block_hash(block_hash);

        
        Ok(())
    }

    #[allow(dead_code)]
    pub async fn contract_count(&self) -> usize {
        self.contracts_db.len()
    }
    
    pub async fn insert(&self, contract: Contract) -> Option<IVec> {
        let id = contract.short_id.clone();

        {
            let mut lookup_guard = self.contracts_long_to_short_lookup.write().await;
            let mut guard = self.contracts_mem_cache.write().await;
            guard.entry(id.clone()).or_insert(contract.clone());
            let short_id = contract.short_id.to_owned();
            let id = contract.id.to_owned();
            lookup_guard.insert(id,short_id);
    
        }

        let pd = contract.to_plutus_data(&vec![]).expect("to_plutus_data should always work");
        
        let serialized = marlowe_lang::plutus_data::to_bytes(&pd).expect("should always be possible to serialize contracts");
        self.contracts_db.insert(id.clone(),serialized).unwrap()
    }


    pub fn all_indexed_contract_ids(&self) -> impl Iterator<Item = String> {
        self.contracts_db.iter().keys().map(|x|{
            let bytes = x.unwrap().to_vec();
            String::from_utf8(bytes).unwrap()
        })
    }

    pub fn contracts_count(&self) -> usize {
        self.contracts_db.len()
    }

    pub fn set_last_seen_block_hash(&self,block_hash: Option<String>) {
        if let Some(bh) = block_hash {
            self.set_string_in_db("index_tip_block_hash",&bh);
        } else {
            self.state_db.remove("index_tip_block_hash").unwrap();
        }
    }
    
    pub fn last_seen_block_hash(&self) -> Option<String> {
        self.get_string_from_db("index_tip_block_hash")
    }

    pub fn set_known_chain_tip(&self,abs_slot_num: SlotId) {
        self.set_u64_in_db("known_chain_tip", abs_slot_num)
    }
    
    pub fn known_chain_tip(&self) -> Option<SlotId> {
        self.get_u64_from_db("known_chain_tip")
    }

    pub fn set_last_seen_slot_num(&self,abs_slot_num: SlotId) {
        self.set_u64_in_db("last_seen_slot_num", abs_slot_num)
    }


    pub fn last_seen_slot_num(&self) -> Option<SlotId> {
        self.get_u64_from_db("last_seen_slot_num")
    }

    #[allow(dead_code)]
    pub fn new_mock(contracts:Vec<Contract>,id:&str) -> Self {

        let temp_dir_exists =  std::fs::try_exists(format!(".marlowe_db_tests/{id}")).unwrap();
        if temp_dir_exists {
            std::fs::remove_dir_all(format!(".marlowe_db_tests/{id}")).unwrap();
        }
        let db = sled::open(format!(".marlowe_db_tests/{id}")).expect("Database initialization failed");
        db.clear().unwrap();

        let db_state = db.open_tree("state").unwrap();
        let contracts_tree = db.open_tree("contracts").unwrap();
        
        for c in contracts {
            let pd = c.to_plutus_data(&vec![]).unwrap();
            db.insert(c.short_id, 
                marlowe_lang::plutus_data::to_bytes(&pd).unwrap()
            ).unwrap();
        }

        MarloweState {
            contracts_long_to_short_lookup: RwLock::new(HashMap::new()),
            contracts_mem_cache: RwLock::new(HashMap::new()),
            state_db: db_state,
            contracts_db: contracts_tree,
            out_ref_to_short_id_lookup: RwLock::new(HashMap::new()),
            broadcast_tx: tokio::sync::broadcast::channel(1000)
        }
        
    }
    pub fn new() -> Self {
        let db = sled::open(".marlowe_db").expect("Database initialization failed");
        let db_state = db.open_tree("state").unwrap();
        let contracts_tree = db.open_tree("contracts").unwrap();
    
        let state = MarloweState {
            contracts_long_to_short_lookup: RwLock::new(HashMap::new()),
            contracts_mem_cache: RwLock::new(HashMap::new()),
            state_db: db_state,
            contracts_db: contracts_tree,
            out_ref_to_short_id_lookup: RwLock::new(HashMap::new()),
            broadcast_tx: tokio::sync::broadcast::channel(1000)
        };

        state
    }

    // Populate mem cache from db
    pub async fn init_mem_cache(&self) {
        tracing::info!("INITIALIZING MEM CACHE FROM DB");
        let mut contracts = vec![];
        for short_id in self.all_indexed_contract_ids() {
            let c = self.get_by_shortid_from_db(short_id.clone()).await.expect(&format!("db contains corrupt data - failed to deserialize contract: {short_id}"));
            contracts.push(c);
        }
        let mut lookup_guard = self.contracts_long_to_short_lookup.write().await;
        let mut guard = self.contracts_mem_cache.write().await;
        guard.clear();
        lookup_guard.clear();
        for c in contracts {
            let short_id = c.short_id.to_owned();
            let id = c.id.to_owned();
            guard.insert(short_id.clone(), c);
            lookup_guard.insert(id,short_id);

        }
        tracing::info!("MEM CACHE INITIALIZED!");
    }

    // contracts that have not ended (ie. contracts that live on the validator address) can be found by the out_ref
    pub async fn get_by_outref(&self,reference:&OutputReference) -> Option<Contract> {
        let map_guard = self.out_ref_to_short_id_lookup.read().await;
        let (_slot,short_id) = map_guard.get(reference)?;
        let data = self.contracts_db.get(short_id).unwrap().unwrap();
        Some(
            marlowe_lang::plutus_data::from_bytes_specific(&data).unwrap()
        )
    }


    pub async fn get_by_long_id_from_mem_cache(&self,long_id:String) -> Option<Contract> {
        let read_guard = self.contracts_long_to_short_lookup.read().await;
        match read_guard.get(&long_id) {
            Some(x) => self.get_by_shortid_from_mem_cache(x.into()).await,
            None => None,
        }
    }

    pub async fn get_by_shortid_from_mem_cache(&self,short_id:ContractShortId) -> Option<Contract> {
        let read_guard = self.contracts_mem_cache.read().await;
        if let Some(c) = read_guard.get(&short_id) { Some(c.clone()) } else { None }
    }

    pub async fn get_by_shortid_from_db(&self,short_id:ContractShortId) -> Option<Contract> {
        match self.contracts_db.get(short_id) {
            Ok(Some(data)) => {
                Some(marlowe_lang::plutus_data::from_bytes_specific(&data).unwrap())
            },
            _ => None,
        }       
    }

    pub async fn apply_internal(&self,event:MarloweEvent) {

        //tracing::info!("HANDLING EVENT: {event:?}");

        let (new_ref,slot,contract_short_id,old_ref_to_remove) = match event {
            MarloweEvent::Add(c) => {   
                let r = c.get_out_ref_if_still_active();
                let csid: String = c.short_id.clone();
                let slot = c.transitions.last().unwrap().abs_slot.clone();
                self.insert(c).await; 
                (r,slot,csid,None)
            },
            MarloweEvent::Update(c) => {
                let r = c.get_out_ref_if_still_active();
                let csid = c.short_id.clone();
                let slot = c.transitions.last().unwrap().abs_slot.clone();
                tracing::info!("a contract was updated!!! {csid}");
                let old_version = self.insert(c).await;                 
                let old_pd = marlowe_lang::plutus_data::from_bytes_specific::<Contract>(&old_version.unwrap()).unwrap();
                let old_ref = old_pd.get_out_ref_if_still_active();
                
                (r,slot,csid,old_ref)
            },
            MarloweEvent::Close(c) => {
                let r = c.get_out_ref_if_still_active();
                let csid = c.short_id.clone();
                let slot = c.transitions.last().unwrap().abs_slot.clone();
                let old_version = self.insert(c).await;      
                tracing::info!("a contract was closed!!! {csid}");                           
                let old_pd = marlowe_lang::plutus_data::from_bytes_specific::<Contract>(&old_version.unwrap()).unwrap();
                let old_ref = old_pd.get_out_ref_if_still_active();
                
                (r,slot,csid,old_ref)             
            },
        };

        // IF THE CONTRACT CHAIN IS ENDED (there was no output to the marlowe validator) WE WILL NEVER NEED A REF AGAIN
        if let Some(k) = old_ref_to_remove {
            let mut guard = self.out_ref_to_short_id_lookup.write().await;
            guard.remove(&k).unwrap();
        }

        // IF NOT ENDED, WE ADD A THIS OUT AS A REF TO THE LOOKUP TABLE SO WE FIND IT FAST WHEN IT IS STEPPED OR CLOSED
        if let Some(r) = new_ref {
            let mut guard = self.out_ref_to_short_id_lookup.write().await;
            _ = guard.insert(r,(slot.into(),contract_short_id));
        }


    }
    
    
    fn set_u64_in_db(&self,key:&str,val:u64) {
        let ivec = IVec::from(&val.to_be_bytes()[..]);
        self.state_db.insert(key, ivec).expect("db should work");
    }

    fn set_string_in_db(&self,key:&str,val:&str) {
        let ivec = IVec::from(val.as_bytes());
        self.state_db.insert(key, ivec).expect("db should work");
    }

    fn get_string_from_db(&self,key:&str) -> Option<String> {
        match self.state_db.get(key) {
            Ok(Some(v)) => {
                let bytes = v.to_vec();
                Some(String::from_utf8(bytes).unwrap())
            },
            _ => None,
        }
    }
    
    fn get_u64_from_db(&self,key:&str) -> Option<u64> {
        let result = self.state_db.get(key).expect("db should work");
        match result {
            Some(ivec) => {
                let bytes = <[u8; 8]>::try_from(ivec.to_vec().as_slice()).unwrap();
                let num = u64::from_be_bytes(bytes);
                Some(num)
            },
            None => None,
        }
    }


}


#[derive(Debug,Clone)]
pub enum MarloweEvent{
    Add(Contract),
    Update(Contract),
    Close(Contract)
}

