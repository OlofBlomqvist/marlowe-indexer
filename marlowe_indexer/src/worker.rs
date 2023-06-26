use std::{sync::Arc, collections::HashMap};
use marlowe_lang::{types::marlowe::{MarloweDatum}, plutus_data::FromPlutusData};
use cardano_chain_sync::{ChainSyncEvent, ChainSyncBlock};
use opentelemetry::KeyValue;
use pallas::ledger::addresses::Address;
use pallas_primitives::{byron::BlockId, Fragment};
use pallas_traverse::{MultiEraTx, OriginalHash, MultiEraOutput};
use tracing::{info, info_span, Instrument, warn};

use crate::state::{State, MarloweUtxo, SlotId};

#[derive(Debug)]
pub struct MarloweSyncWorker{
    state: Arc<tokio::sync::RwLock<State>>
}
impl MarloweSyncWorker {
    pub fn new(state:Arc<tokio::sync::RwLock<State>>) -> Self {
        MarloweSyncWorker { 
            state : state
        }
    }
}
use async_trait::async_trait;
#[async_trait]
impl crate::ChainSyncReceiver for MarloweSyncWorker {
    
    async fn handle_event(&mut self, event: ChainSyncEvent) {
        
        match &event {
            cardano_chain_sync::ChainSyncEvent::Block(b) => {
                let span = tracing::info_span!("handle_new_block",block_id = tracing::field::Empty);
                let decoded = b.decode().expect("decode block from vec should always work");
                let txs = decoded.txs();
                let tx_iter = txs.iter().enumerate();
                let _guard = span
                    .record("tx_count",&txs.len())
                    .record("block_id",&decoded.hash().to_string()).enter(); 

                for (i,t) in tx_iter {
                    self.apply_transaction(
                        i,
                        decoded.slot(), 
                        &t,
                        b,
                    ).instrument(span.clone()).await;
                }                
            }
            
            cardano_chain_sync::ChainSyncEvent::Rollback { block_slot,block_hash } => {
                let span = tracing::info_span!("Handle_Rollback_Event",block_slot = &block_slot);                
                
                self.rollback(
                    *block_slot,
                    if let Some(b) = block_hash { 
                        let barr: [u8; 32] = (&b[..]).try_into().unwrap();
                        Some(BlockId::new(barr)) 
                    } else { None }
                ).instrument(span).await;
            }

            cardano_chain_sync::ChainSyncEvent::Noop => {
                tracing::info_span!("Handle_NOOP_Event").in_scope(|| info!("TIP REACHED. WAITING FOR NEW BLOCKS."))               
            },
        }
    }
}

// TODO - Dedup, prevent cloning, only decode block once
impl MarloweSyncWorker {
    
    
    pub async fn apply_transaction<'a>(&mut self, tx_index:usize, slot:SlotId,transaction:&MultiEraTx<'a>,block:&ChainSyncBlock) -> bool {
        
        let mut state_accessor = self.state.write().await;
        state_accessor.last_block_slot = Some(slot);
        state_accessor.last_block_hash = Some(block.decode().unwrap().hash());
        let mut consumed_from_contract_id : Option<String> = None;
        let mut consumed_from_marlowe_address : Vec<(String,MarloweUtxo)> = vec![];
        let inputs = transaction.inputs();

        for x in inputs {
            for (contract_id,txs) in &state_accessor.contracts { 
                if let Some(tx) = txs.get(&(x.hash().to_string(),x.index())) {
                    if tx.end == false {
                        consumed_from_marlowe_address.push((contract_id.clone(), tx.clone()));
                    }
                };
            }
        }

        if consumed_from_marlowe_address.len() > 1 {
            panic!("this tx {:?} consumes multiple utxos consumed from marlowe address: {:?}",transaction.hash(),consumed_from_marlowe_address)
        }

        let datums = transaction.plutus_data();  
        let out_to_marlowe : Vec<(usize,pallas_traverse::MultiEraOutput)> = 
        
            transaction.outputs().into_iter().enumerate().filter(|(_i,o)| {
                let a = o.address().unwrap();
                let payment_hash = match &a {
                    Address::Byron(_b) => None,
                    Address::Shelley(sh) => Some(sh.payment().to_hex()),
                    Address::Stake(_st) => None,
                };

                // TODO - separate checks per version so we can tag contracts with validator version
                // instead of decoding this multiple times
                let mut is_marlowe = false;
                if payment_hash.as_ref().is_some_and(|x|
                        x == "6a9391d6aa51af28dd876ebb5565b69d1e83e5ac7861506bd29b56b0"
                    || x == "2ed2631dbb277c84334453c5c437b86325d371f0835a28b910a91a6e"
                ) {
                    is_marlowe = true;
                    
                }
                is_marlowe
            }
        ).collect();

        let block = block.decode().unwrap();
        if consumed_from_marlowe_address.len() == 1 {
            let (consumed_id,consumed_utxo) = consumed_from_marlowe_address.first().unwrap().to_owned();
            
            consumed_from_contract_id = Some(consumed_id.clone());
            if out_to_marlowe.is_empty() {
                info!("contract {:?} is now closed by tx {:?}",&consumed_from_contract_id,transaction.hash().to_string());
            }

            if let Some(redeemers) = transaction.redeemers() {
                
                if &redeemers.len() != &1 {
                    panic!("should not be possible to use more than one redeemer when consuming from marlowe, right?")
                }

                let redeemer_plutus_data = &redeemers.first().unwrap().data;
                let b = redeemer_plutus_data.encode_fragment().unwrap();
                let redeemer_text = {
                    // todo - add direct vec dec to marlowe_lang and get rid of this nonsense
                    match marlowe_lang::extras::utils::try_decode_redeemer_input_cbor_hex(&hex::encode(b)) {
                        Ok(r) => {
                            match marlowe_lang::serialization::json::serialize(r) {
                                Ok(j) => j,
                                Err(e) => {
                                    format!("error serializing redeemer: {e}")
                                },
                            }
                        },
                        Err(e) => {
                            format!("error decoding redeemer: {e}")
                        },
                    }
                };

                if out_to_marlowe.len() > 1 {
                    panic!("not possible... cant step a marlowe contract more than one output to marlowe validator.");
                }

                // in case of a tx closing the contract, there does not need to be a datum
                let datum = 
                    if let Some(o) = out_to_marlowe.first() { 
                        read_marlowe_info_from_utxo(&o.1,&datums)  
                    } else {
                        "".into()
                    };

                let txs = state_accessor.contracts.get_mut(&consumed_id).unwrap();
                txs.insert(
                    (transaction.hash().to_string(),consumed_utxo.utxo_index),
                    MarloweUtxo { 
                        block_num: block.number(),
                        tx_index: tx_index as u64,
                        block_hash: block.hash(),
                        redeemer: redeemer_text,
                        datum,
                        id: transaction.hash().to_string(), slot: slot, 
                        utxo_index : consumed_utxo.utxo_index, end: out_to_marlowe.is_empty() 
                    }
                );
            } else {
                unreachable!("it should not be possible to consume a marlowe utxo without redeemer")
            }


        }

        
       
        if out_to_marlowe.len() > 0 || consumed_from_contract_id.is_some() {
            
            let span = info_span!("marlowe_tx_handled",init_count=tracing::field::Empty,modified_count=tracing::field::Empty);
            let _guard = span.enter();

            if &out_to_marlowe.len() > &0 {
            
                if consumed_from_contract_id.is_none() {
                    let mut created_ids = vec![];
                    
                    span.record("init_count", &out_to_marlowe.len());                    
                    
                                    
                    
                    for (i,o) in &out_to_marlowe {
                        info!("New contract initialized: {}#{}",transaction.hash(),i);
                        
                        // todo: more useful info than just a dumb string
                        let text = read_marlowe_info_from_utxo(o,&datums);

                        let mut initial_map = HashMap::new();
                        
                        initial_map.insert(
                            (transaction.hash().to_string(),*i as u64),
                            MarloweUtxo {
                                block_num : block.number(),
                                tx_index: tx_index as u64,
                                block_hash: block.hash(),
                                redeemer: "".into(),
                                datum: text,
                                id: transaction.hash().to_string(),
                                slot: slot,
                                utxo_index: *i as u64,
                                end: false
                            });
                        state_accessor.contracts.insert(transaction.hash().to_string(), initial_map);
                        created_ids.push(
                            KeyValue::new(
                                transaction.hash().to_string(),
                                format!("#{}",*i as f64)
                            )
                        );
                    }
                }
            }
        
            info!("There are now {:?} known contract chains with a total of {:?} transactions.",
                &state_accessor.contracts.len(),
                &state_accessor.contracts.iter().map(|(_, b)| b.len() as u64).sum::<u64>()
            );
            
            span.record("modified_count", consumed_from_marlowe_address.len());
            

            if consumed_from_marlowe_address.len() > 0 {
                for (id,utxo) in consumed_from_marlowe_address {
                    
                    
                    info!("tx {:?} consumes utxo {:?} (from the tx chain of {:?})",transaction.hash(),utxo.id,id);                        
                }
            }

        }


       
        out_to_marlowe.first().is_some() || consumed_from_contract_id.is_some()

    
    }

    // TODO - This is most likely not correct, so fix it
    pub async fn rollback<'a>(&mut self,  block_slot: SlotId,block_hash: Option<BlockId>) -> bool {
        let mut state_accessor = self.state.write().await;
        info!("rolling back everything from slot id {:?} to {} (block: {block_hash:?})",state_accessor.last_block_slot,block_slot);
        state_accessor.last_block_slot = Some(block_slot);
        state_accessor.last_block_hash = if let Some(b) = block_hash { 
            Some(BlockId::new(*b)) 
        } else {
            None
        };
        for (_contract_id,txs)in state_accessor.contracts.iter_mut() {
            txs.retain(|a,b| {
                let result = b.slot <= block_slot;
                if !result {
                    info!("dropping tx due to rollback: {:?}",a)
                }
                result  
            });        
        }
        true
    }

}



fn read_marlowe_info_from_utxo(o:&MultiEraOutput,datums:&Vec<&pallas::codec::utils::KeepRaw<'_, pallas_primitives::babbage::PlutusData>>) -> std::string::String {
    match o.datum() {
        Some(x) => {
            match x {
                pallas_primitives::babbage::PseudoDatumOption::Hash(datum_hash) => {
                    
                    let matching_datum = 
                        datums.iter().find(|x|x.original_hash() == datum_hash);

                    if let Some(datum) = matching_datum {
                        let bytes = datum.raw_cbor().to_owned();                                            
                        let pd = marlowe_lang::plutus_data::PlutusData::from_bytes(bytes);
                        match pd {
                            Ok(d) => {
                                let marlowe_state = MarloweDatum::from_plutus_data(d, &vec![]);
                                match marlowe_state {
                                    Ok(v) => match marlowe_lang::serialization::json::serialize(v) {
                                        Ok(vv) => vv,
                                        Err(e) => {
                                            warn!("failed to encode datum. error: {}",e);
                                            format!("{e:?}")
                                        },
                                    },
                                    Err(e) => {
                                        warn!("failed to decode datum. error: {}",e);
                                        format!("{e:?}")
                                    }
                                }
                            },
                            Err(e) => {
                                warn!("failed to deserialize datum. error: {:?}",e);
                                format!("Failed to decode datum in to plutus data. Error: {e:?}")
                            },
                        }
                        
                     
                    } else {
                        datum_hash.to_string()
                    }

                },
                pallas_primitives::babbage::PseudoDatumOption::Data(datum) => {
                    let bytes = datum.raw_cbor().to_owned();
                    let xhex = hex::encode(bytes);
                    
                    let marlowe_state = marlowe_lang::extras::utils::try_decode_cborhex_marlowe_plutus_datum(&xhex);
                    match marlowe_state {
                        Ok(v) => match marlowe_lang::serialization::json::serialize(v) {
                            Ok(vv) => vv,
                            Err(e) => {
                                warn!("failed to decode datum. error: {}",e);
                                format!("{e:?}")
                            }
                        },
                        Err(e) => {
                            warn!("failed to decode datum. error: {}",e);
                            format!("{e:?}")
                        }
                    }
                },
            }
        },
        None => String::new(),
    }
}