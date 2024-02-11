use super::graphql::query::UtxoInfo;

use crossbeam::atomic::AtomicCell;
use sled::Tree;

pub type Bech32AddrStr = String;

pub enum PersistanceMode {
    InMemory,
    FileSystem { file_path : String}
}

pub struct AddressState {
    persistance_configuration : PersistanceMode,
    pub utxo_addr_map : Tree,
    pub addresses : Tree,
    persisted_tip_block_abs_slot : AtomicCell<u64>,
    tip_block_abs_slot : AtomicCell<u64>,
    index_start_slot : u64

}

impl AddressState {


    pub fn new(persistance_mode:PersistanceMode,initial_slot:u64) -> Self {

        let db = sled::open(".marlowe_indexer_addr_db").expect("Database initialization failed");
        
        let adr_tree = db.open_tree("addresses").unwrap();
        let utxo_tree = db.open_tree("utxo_addr").unwrap();

        AddressState {
            addresses: adr_tree, //tokio::sync::RwLock::new(HashMap::new()),
            persisted_tip_block_abs_slot: AtomicCell::new(0),
            persistance_configuration: persistance_mode,
            tip_block_abs_slot :  AtomicCell::new(0),
            utxo_addr_map: utxo_tree,
            index_start_slot: initial_slot
        }

    }

    fn find_addr_of_utxo(&self,utxo_hash:String,utxo_id:u64) -> Option<String> {
        if let Ok(Some(serialized_addr)) = self.utxo_addr_map.get(format!("{}#{}",utxo_hash,utxo_id)) {
            Some(serialized_addr.as_ascii().unwrap().as_str().to_string())
        } else {
            None
        }
    }
    
    pub fn add_utxo(&self,utxo:UtxoInfo) {
        
        let addr = utxo.address.clone();
        let hash = utxo.hash.clone();
        let index = utxo.index;

        let new_utxos = 
            if let Some(existing_addr_utxos) = self.addresses.get(addr.clone()).expect("should always be able to get result from db") {
                let mut utxos: Vec<UtxoInfo> = serde_json::from_slice(&existing_addr_utxos).expect("Deserialization of utxos failed");
                utxos.push(utxo);
                utxos           
            } else {
                vec![utxo]
            };

        let new_utxos = serde_json::to_vec(&new_utxos).expect("Serialization failed");
        self.addresses.insert(addr.clone(), new_utxos).unwrap();
        self.utxo_addr_map.insert(format!("{}#{}",hash,index),addr.as_bytes()).unwrap();
    }

    pub fn remove_utxo(&self,utxo_hash:String,utxo_index:u64) {

        if let Some(current_addr) = self.find_addr_of_utxo(utxo_hash.clone(), utxo_index) {
            let serialized_utxos = self.addresses.get(current_addr.clone()).unwrap().expect("no such addr- there is a bug in the address module!");
            let mut utxos: Vec<UtxoInfo> = serde_json::from_slice(&serialized_utxos).expect("Deserialization of utxos failed");
            utxos.retain(|x|!(x.hash == utxo_hash && x.index == utxo_index));
            let utxos_without_the_removed_one = serde_json::to_vec(&utxos).expect("Serialization failed");
            self.addresses.insert(current_addr, utxos_without_the_removed_one).unwrap();
            self.utxo_addr_map.remove(format!("{}#{}",utxo_hash,utxo_index)).unwrap();

            tracing::debug!("Removed utxo during rollback: {}#{}", utxo_hash,utxo_index)

        } else if self.index_start_slot == 0 {
            panic!("There is a bug in marlowe indexer. Attempted to remove utxo during rollback: {}#{} - but that does not exist!",utxo_hash,utxo_index)
        }
    }

}
