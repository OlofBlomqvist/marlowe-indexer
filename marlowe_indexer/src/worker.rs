use std::sync::Arc;
use marlowe_lang::{types::marlowe::{MarloweDatum, PossiblyMerkleizedInput}, plutus_data::FromPlutusData};
use cardano_chain_sync::{ChainSyncEvent, ChainSyncBlock};
use opentelemetry::KeyValue;
use pallas::ledger::addresses::Address;
use pallas_primitives::{Fragment, babbage::PlutusV2Script};
use pallas_traverse::{MultiEraTx, OriginalHash, MultiEraOutput};
use tracing::{info, info_span, Instrument, warn, trace};
use crate::state::{State, SlotId, MarloweTransition, Contract, OutputReference};

#[derive(Debug)]
pub struct MarloweSyncWorker{
    state: Arc<tokio::sync::RwLock<State>>,
    tip : pallas_network::miniprotocols::chainsync::Tip
}
impl MarloweSyncWorker {
    pub fn new(state:Arc<tokio::sync::RwLock<State>>) -> Self {

        info!("Marlowe Sync Worker Initialized");
            
        MarloweSyncWorker { 
            state,
            tip:pallas_network::miniprotocols::chainsync::Tip(pallas_network::miniprotocols::Point::Origin,0)
        }
    }
}
use async_trait::async_trait;
#[async_trait]
impl crate::ChainSyncReceiver for MarloweSyncWorker {
    
    #[tracing::instrument(skip_all)]
    async fn handle_event(&mut self, event: &ChainSyncEvent) {
        
        match &event {
            cardano_chain_sync::ChainSyncEvent::Block(b,tip) => {
                
                self.tip = tip.clone();
                //self.tip = pallas_network::miniprotocols::chainsync::Tip(Point::Specific(tip.0.), tip.0);
                //let span = tracing::info_span!("handle_new_block",block_id = tracing::field::Empty);
                let decoded = b.decode().expect("decode block from vec should always work");
                let txs = decoded.txs();
                let tx_iter = txs.iter().enumerate();
                // let _guard = span
                //     .record("tx_count",txs.len())
                //     .record("block_id",&decoded.hash().to_string())
                //     .record("block_num",&decoded.number().to_string()).enter(); 
                    

                for (i,t) in tx_iter {
                    self.apply_transaction(
                        i,
                        decoded.slot(), 
                        t,
                        b,
                    ).await; 
                }        
                
            }
            
            cardano_chain_sync::ChainSyncEvent::Rollback { block_slot,tip} => {
                self.tip = tip.clone();
                self.rollback(
                    *block_slot
                ).await;
            }

            cardano_chain_sync::ChainSyncEvent::Noop => {
                tracing::info_span!("Handle_NOOP_Event").in_scope(|| info!("TIP REACHED. WAITING FOR NEW BLOCKS."));          
            },
        }
        
    }
}

impl MarloweSyncWorker {
    
    #[tracing::instrument(skip_all,fields(block_num=tracing::field::Empty,init_count=tracing::field::Empty,modified_count=tracing::field::Empty))]
    pub async fn apply_transaction<'a>(&mut self, tx_index:usize, slot:SlotId,transaction:&MultiEraTx<'a>,block:&ChainSyncBlock) {
        
        let mut state_accessor = self.state.write().await;
        state_accessor.last_block_slot = Some(slot);
        state_accessor.last_block_hash = Some(block.decode().unwrap().hash());
        
        let mut consumed_from_contract_id : Option<String> = None;

        // ID,SHORTID,CONSUMED,TRANSITION/UTXO, CONTRACT INDEX
        // TODO : Get this shit in a struct
        let mut consumed_from_marlowe_address : Vec<(pallas_traverse::MultiEraInput,String,String,MarloweTransition,usize,usize)> = vec![];

        let inputs = transaction.inputs();

        for (index_of_input,x) in inputs.into_iter().enumerate() {
            
            let tx_hash = x.hash().to_string();
            let output_utxo_index = x.index();

            let consumed_from_contract_ref = {
                state_accessor.ordered_contracts.utxo_to_contract_lookup_table.get(
                    &{
                        OutputReference {
                            tx_hash: tx_hash.clone(),
                            output_index: output_utxo_index,
                        }
                    }
                ).map(|v| v.0)
            };
            
            if let Some(consumed_contract_index) = consumed_from_contract_ref {
                
                //tracing::trace!("Found reference for {tx_hash}#{output_utxo_index} with index: {consumed_contract_index}");

                let consumed_contract = state_accessor.ordered_contracts.contracts_in_order
                    .get_mut(consumed_contract_index).expect(&format!("marlowe-indexer bug: broken contract reference for {:?}",transaction.hash()));
                
                let consumed_utxo = 
                    consumed_contract.transitions.iter().find(|t| 
                        if let Some(tui) = t.utxo_index { tui as u64 == output_utxo_index } 
                        else { false } 
                        && t.tx_id == tx_hash
                    ).expect(&format!("Found transaction {:?} that consumes an utxo from a contract, but failed to find that utxo in the contract via utxo lookup table. This is a bug in marlowe-indexer.",transaction.hash()));

                if !consumed_utxo.end {
                    consumed_from_marlowe_address.push(
                        (
                            x,
                            consumed_contract.id.clone(),
                            consumed_contract.short_id.clone(), 
                            consumed_utxo.clone(),
                            consumed_contract_index,
                            index_of_input
                        )
                    );
                } else {
                    panic!("bug in marlowe-indexer. processed a transaction that consumes the utxo of an already closed contract. This should never happen.")
                }
            }
            
        }

        if consumed_from_marlowe_address.len() > 1 {
            panic!("this tx {:?} consumes multiple utxos from marlowe address: {:?}",transaction.hash(),consumed_from_marlowe_address)
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

            let (consumed_utxo,consumed_id,consumed_short_id,_consumed_transition_item,contract_index,index_of_input_in_this_tx) = 
                consumed_from_marlowe_address.first().unwrap().to_owned();
            
            consumed_from_contract_id = Some(consumed_id.clone());

            if out_to_marlowe.is_empty() {
                trace!("Contract {:?} was closed by tx {:?}",&consumed_from_contract_id,transaction.hash().to_string());
            }
            
            let c = consumed_from_marlowe_address.first().unwrap();
            
            
            if let Some(redeemers) = transaction.redeemers() {
                
                let redeemer_plutus_data = &redeemers.iter().find(|r|r.index as usize ==index_of_input_in_this_tx)
                    .expect("because this transaction consumes an utxo from the marlowe validator, there MUST be a redeemer here.").data;

                let b = redeemer_plutus_data.encode_fragment().unwrap();
                
                let mut issues = vec![];

                let redeemer_text = {
                    
                    match marlowe_lang::plutus_data::PlutusData::from_bytes(b) {
                        Ok(pd) => {
                            match Vec::<PossiblyMerkleizedInput>::from_plutus_data(pd, &vec![]) {
                                Ok(red) => {
                                    match marlowe_lang::serialization::json::serialize(red) {
                                        Ok(j) => j,
                                        Err(e) => {
                                            issues.push(format!("failed to serialize redeemer! {}",e));
                                            "".into()
                                        },
                                    }
                                },
                                Err(e) => {
                                    issues.push(format!("failed to decode redeemer from plutus data! {}",e));
                                    "".into()
                                },
                            }
                        },
                        Err(e) => {
                            issues.push(format!("failed to decode redeemer plutus data!! Because the tx exists and consumes data from marlowe, the redeemer must be valid. if you see this, marlowe-indexer or marlowe-rs has a bug.. {e:?}"));
                            "".into()
                        }
                    }

                   
                };

                if out_to_marlowe.len() > 1 {
                    panic!("not possible... cant step a marlowe contract more than one output to marlowe validator.");
                }

                // in case of a tx closing the contract, there does not need to be a datum
                let (datum,utxo_id) = 
                    if let Some(o) = out_to_marlowe.first() { 
                        let rr = read_marlowe_info_from_utxo(&o.1,&datums) ;
                        match rr {
                            Ok(rrr) => (Ok(rrr) , o.0),
                            Err(rrr) => {
                                issues.push(rrr.clone());
                                (Err(rrr) , o.0)
                            },
                        }
                    } else {
                        (Ok("".into()),99999)
                    };
                    
                tracing::trace!("This tx consumes {consumed_short_id}");

                let contract = state_accessor.ordered_contracts.get_mut(&consumed_short_id).unwrap();         

                // obviously this contract is valid because this tx moves it.
                // therefore we dont need to check marlowe_scan - we know it existts.
                let marlowe_scan_status = None; 
                
                let new_transition = MarloweTransition {
                    block_num : block.number() as f64,
                    //tx_index: tx_index as f64,
                    block: block.hash().to_string(),
                    datum: datum.unwrap_or(String::new()),
                    redeemer: redeemer_text,
                    tx_id: transaction.hash().to_string(),
                    utxo_index: Some(utxo_id as f64),
                    slot: slot as f64,
                    end: out_to_marlowe.is_empty(),
                    invalid: !issues.is_empty(),
                    issues,
                    marlowe_scan_status
                };

                // EVERY TIME WE ADD A TRANSITION, WE MUST ALSO ADD IT TO THE LOOKUP TABLE.
                // PERHAPS WE SHOULD HAVE A COMBINED METHOD FOR THIS..
                contract.transitions.push(new_transition);     

                let oldref = contract.transitions.last().expect("since we consume an utxo, it must exist");
                let k = {
                    OutputReference {
                        tx_hash: oldref.tx_id.to_string(),
                        output_index: oldref.utxo_index.expect("since we consume an utxo, it must exist") as u64,
                    }
                };

                // drop old ref since it can never be consumed again
                state_accessor.ordered_contracts.utxo_to_contract_lookup_table.remove(&k);
                

                // only add a new ref if the contract is still active
                if !out_to_marlowe.is_empty() {
                    state_accessor.ordered_contracts.utxo_to_contract_lookup_table.insert(
                        OutputReference { 
                            tx_hash: transaction.hash().to_string(), 
                            output_index: utxo_id as u64
                        }, 
                        (contract_index,slot as f64)
                    );        
                }
   

            } else {
                unreachable!("it should not be possible to consume a marlowe utxo without redeemer")
            }
        }
        

        tracing::Span::current().record("init_count", out_to_marlowe.len());
        
        if !out_to_marlowe.is_empty() || consumed_from_contract_id.is_some() {
            
            if !out_to_marlowe.is_empty() && consumed_from_contract_id.is_none() {
                let mut created_ids = vec![];
                
                              
                
                for (i,o) in &out_to_marlowe {
                    // todo: more useful info than just a dumb string
                    let rtext = read_marlowe_info_from_utxo(o,&datums); 
                    let mut text = String::new();
                    let mut issues = vec![];

                    if let Err(e) = &rtext {
                        issues.push(format!("init err. could not read datum: {}",e));
                    }

                    if let Ok(t) = &rtext {
                        text = t.to_string();
                    }

                    let cid = transaction.hash().to_string();
                    let block_num = block.number();
                    
                    
                    let bid = format!("{}{}{}",block_num,tx_index,i);
                    let short_id = bs58::encode(&bid).into_string();           
                    

                    let marlowe_scan_status = if !issues.is_empty() {
                        let ms = get_marlowe_scan_preprod_status(&cid, *i).await;
                        match ms {
                            Ok(v) => {
                                Some(v.as_str().to_string())
                            },
                            Err(e) => {
                                Some(format!("{e:?}"))
                            }
                        }
                    } else {
                        None
                    };
                    
                    let contract = Contract {
                            id : cid,                    
                            short_id,
                            transitions: vec![
                                MarloweTransition {
                                    block_num : block.number() as f64,
                                    //tx_index: tx_index as f64,
                                    block: block.hash().to_string(),
                                    datum:  text.clone(),
                                    redeemer: "".into(),
                                    tx_id: transaction.hash().to_string(),
                                    utxo_index: Some(*i as f64),
                                    slot: slot as f64,
                                    end: false,
                                    invalid: !issues.is_empty(),
                                    issues,
                                    marlowe_scan_status
                                }
                            ]
                        };

                    state_accessor.ordered_contracts.insert(contract);
                    
                    let new_count = state_accessor.ordered_contracts.contract_count();

                    info!("New contract found in block {}: {}#{}. There are now {} known contracts",block.number().to_string(),transaction.hash(),i,&new_count);
                    

                    // TODO - we should make force consumers of our state to not push directly to the internal structures because we
                    // will end up doing mistakes like forgetting to do so in some places.. also we should remove unused refs after someone consumes them
                    state_accessor.ordered_contracts.utxo_to_contract_lookup_table.insert(
                        OutputReference { 
                            tx_hash: transaction.hash().to_string(), 
                            output_index: *i as u64
                        }, 
                        (new_count - 1,slot as f64)
                    ); 

                    created_ids.push(
                        KeyValue::new(
                            transaction.hash().to_string(),
                            format!("#{}",*i as f64)
                        )
                    );
                    
                }
            }

            tracing::Span::current().record("modified_count", consumed_from_marlowe_address.len());

            if !consumed_from_marlowe_address.is_empty() {
                for (_consumed_utxo,id,_short_id,consumed_transition_item,_contract_index,index_of_consumed_input) in consumed_from_marlowe_address {
                    tracing::debug!("tx {:?} consumes utxo {}#{} (from the tx chain of {:?})",transaction.hash(),consumed_transition_item.tx_id,consumed_transition_item.utxo_index.expect("any transition that is consumed must have an utxo_index"),id);                        
                }
            }
        }
       
    
    }

    
    pub async fn rollback<'a>(&mut self,  block_slot: SlotId) -> bool {
        let mut state_accessor = self.state.write().await;
        if state_accessor.last_block_slot.is_some() {
            warn!("rolling back everything from slot id {:?} to {}",state_accessor.last_block_slot,block_slot);
        }
        state_accessor.last_block_slot = Some(block_slot);
        state_accessor.ordered_contracts.rollback(block_slot as f64);

        true
    }

}



fn read_marlowe_info_from_utxo(o:&MultiEraOutput,datums:&[&pallas::codec::utils::KeepRaw<'_, pallas_primitives::babbage::PlutusData>]) -> Result<std::string::String,String> {
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
                                        Ok(vv) => Ok(vv),
                                        Err(e) => {
                                            warn!("failed to encode datum. error: {}",e);
                                            Err(format!("{e:?}"))
                                        },
                                    },
                                    Err(e) => {
                                        warn!("failed to decode datum. error: {}",e);
                                        Err(format!("{e:?}"))
                                    }
                                }
                            },
                            Err(e) => {
                                warn!("failed to deserialize datum. error: {:?}",e);
                                Err(format!("Failed to decode datum in to plutus data. Error: {e:?}"))
                            },
                        }
                        
                     
                    } else {
                        Ok(datum_hash.to_string())
                    }

                },
                pallas_primitives::babbage::PseudoDatumOption::Data(datum) => {
                    let bytes = datum.raw_cbor().to_owned();
                    let xhex = hex::encode(bytes);
                    
                    let marlowe_state = marlowe_lang::extras::utils::try_decode_cborhex_marlowe_plutus_datum(&xhex);
                    match marlowe_state {
                        Ok(v) => match marlowe_lang::serialization::json::serialize(v) {
                            Ok(vv) => Ok(vv),
                            Err(e) => {
                                warn!("failed to decode datum. error: {}",e);
                                Err(format!("{e:?}"))
                            }
                        },
                        Err(e) => {
                            warn!("failed to decode datum. error: {}",e);
                            Err(format!("{e:?}"))
                        }
                    }
                },
            }
        },
        None => Ok(String::new()),
    }
}


// TEMP : For contracts we cannot decode or otherwise have issues processing, we will call marlowe scan to see if they have processed the tx successfully.
// Logic being that if cant process it, and marlowescan has not been able to either, then the contract is most likely just invalid, but if marlowescan
// has it, then we probably have a bug in marlowe-rs/marlowe-indexer.
async fn get_marlowe_scan_preprod_status(initial_tx_hash:&str,utxo_index:usize) -> Result<reqwest::StatusCode,reqwest::Error> {
    let response = reqwest::get(format!("https://preprod.marlowescan.com/isContractOpen?contractId={initial_tx_hash}%23{utxo_index}")).await?;
    Ok(response.status())
}