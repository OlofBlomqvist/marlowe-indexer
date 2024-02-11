use pallas::codec::{minicbor::Decode, utils::KeyValuePairs};
use pallas_primitives::conway::AssetName;
use warp::filters::trace::named;

use crate::core::lib::ChainSyncReceiver;
use super::{AddressModule, graphql::query::UtxoInfo};


#[async_trait::async_trait]
impl ChainSyncReceiver for AddressModule {
    async fn handle_event(&mut self, event: &crate::core::lib::ChainSyncEvent) {
        
        let cs = self.state.clone();
        
        let mut consumed_utxos = vec![];
        let mut new_utxos = vec![];
        
        match event {
            crate::core::lib::ChainSyncEvent::Block(a, _tip) => {
                let block = a.decode().unwrap();
                for tx in block.txs().iter() {
                    
                    for (i,output) in tx.outputs().iter().enumerate() {
                        
                        let addr = output.address().unwrap();
 
                        let mut assets = std::collections::HashMap::new();

                        
                        for a in output.non_ada_assets() {
                            for output in a.assets() {
                               let name = if let Some(name) = output.to_ascii_name() {
                                    name
                               } else {
                                    tracing::warn!("failed to read name of asset: {a:?}");
                                    "unknown".into()
                               };
                               let amount = output.any_coin().to_string();
                               assets.insert(name, amount);
                            }
                        }

                        new_utxos.push(UtxoInfo {
                            hash: tx.hash().to_string(),
                            index: i as u64,
                            lovelace: output.lovelace_amount().to_string(),
                            other_assets: assets,
                            address: addr.to_string(),
                            block_abs_slot: block.slot(),
                            block_num: block.number()
                        });

                       
                    }
      
                    for consumed in tx.consumes() {
                        consumed_utxos.push((consumed.hash().to_string(),consumed.index()));
                    }
                }
              

            },
            crate::core::lib::ChainSyncEvent::Rollback { abs_slot_to_go_back_to, tip:_, block_hash } => {
                tracing::info!("Address module - performing rollback to slot {}",abs_slot_to_go_back_to);
             
                for item in cs.addresses.iter() {
                    if let Ok((key, serialized_utxos)) = item {
                        let addr: String = key.as_ascii().unwrap().as_str().to_string();
                        let utxos: Vec<UtxoInfo> = serde_json::from_slice(&serialized_utxos).expect("Deserialization of utxos failed");
                        let updated_utxos : Vec<&UtxoInfo> = utxos.iter().filter(|x| &x.block_abs_slot <= abs_slot_to_go_back_to).collect::<Vec<&UtxoInfo>>();
                        // only re-serialize and update if we actually removed anything
                        if utxos.len() > updated_utxos.len() {
                            let serialized_utxos = serde_json::to_vec(&utxos).expect("Serialization failed");
                            cs.addresses.insert(addr, serialized_utxos).unwrap();
                        }
                    }
                }    
                
                return

            }
            crate::core::lib::ChainSyncEvent::Noop => {
                tracing::debug!("Address module - hangout out at the tip!");
                return
            }
        };

        // Remove consumed utxos from memory
        for (k,v) in consumed_utxos {
            cs.remove_utxo( k, v);
        }

        // Create the new utxos
        for x in new_utxos.into_iter() {
             cs.add_utxo(x);
        }
        
    }
}