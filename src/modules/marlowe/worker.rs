use std::{io::Read, thread::current, ops::DerefMut};

use marlowe_lang::{types::marlowe::{MarloweDatum, PossiblyMerkleizedInput}, plutus_data::FromPlutusData};
use crate::core::lib::{ChainSyncEvent, ChainSyncBlock};
use opentelemetry::KeyValue;
use pallas::ledger::addresses::Address;
use pallas_primitives::Fragment;
use pallas_traverse::{MultiEraTx, OriginalHash, MultiEraOutput};
use tracing::{info,  warn, info_span, Instrument, trace_span};
use super::state::{SlotId, MarloweTransition, Contract, OutputReference, GraphQLHackForu64, ContractShortId, MarloweEvent};

use async_trait::async_trait;

#[async_trait]
impl crate::core::lib::ChainSyncReceiver for super::MarloweSyncModule {
    
    #[tracing::instrument(skip_all)]
    async fn handle_event(&mut self, event: &ChainSyncEvent) {
        
        match &event {
            crate::core::lib::ChainSyncEvent::Block(b,tip) => {
                
                self.tip = tip.clone();
                //self.tip = pallas_network::miniprotocols::chainsync::Tip(Point::Specific(tip.0.), tip.0);
                //let span = tracing::info_span!("handle_new_block",block_id = tracing::field::Empty);
                let decoded = b.decode().expect("decode block from vec should always work");
                let txs = decoded.txs();
                let tx_iter = txs.iter().enumerate();
                let txcount = txs.len();
                let block_num = decoded.number();
                
                let span = info_span!("handling block", tx_count = txcount, block_hash = &decoded.hash().to_string(), block_num = &block_num.to_string());
                        
                let current_slot = decoded.slot();
                
                for (i,t) in tx_iter {
                    self.apply_transaction(
                        i,
                        current_slot, 
                        t,
                        b,
                    ).instrument(span.clone()).await; 
                }        



                self.state.set_last_seen_block_hash(Some(decoded.hash().to_string()));
                self.state.set_known_chain_tip(self.tip.0.slot_or_default());                
                self.state.set_last_seen_slot_num(current_slot);

                
            }
            
            crate::core::lib::ChainSyncEvent::Rollback { abs_slot_to_go_back_to,tip, block_hash} => {
                self.tip = tip.clone();

                let span = info_span!("rollback", rollback_to_block_slot = abs_slot_to_go_back_to);
                let _guard = span.enter();

                self.state.rollback(
                    *abs_slot_to_go_back_to,
                    block_hash.clone()
                ).await.unwrap();
            }

            crate::core::lib::ChainSyncEvent::Noop => {
                tracing::info_span!("Handle_NOOP_Event").in_scope(|| info!("TIP REACHED. WAITING FOR NEW BLOCKS."));          
            },
        }
        
    }
}

#[derive(Debug,Clone)]
pub struct ConsumedThing<'a> {
    input:pallas_traverse::MultiEraInput<'a>, // INPUT
    short_id:ContractShortId, // SHORT ID
    transition:MarloweTransition, // the utxo
    index_of_output:usize, // index of output
    index_of_input:usize,
    consumed_contract: Contract
}

impl crate::modules::marlowe::MarloweSyncModule {
    
    pub async fn apply(&mut self,event:crate::modules::marlowe::MarloweEvent) {
        self.state.apply_internal(event.clone()).await;
        _ = self.evt_main_chan.send(event).unwrap();

    }



    //#[tracing::instrument(skip_all,fields(block_num=tracing::field::Empty,level="debug"))]
    //let span = tracing::info_span!("handle_new_block",block_id = tracing::field::Empty);
    pub async fn apply_transaction<'a>(&mut self, tx_index:usize, slot:SlotId,transaction:&MultiEraTx<'a>,block:&ChainSyncBlock) {
       
        let mut consumed_from_marlowe_address : Vec<ConsumedThing> = vec![];

        let inputs = transaction.inputs();
        {
            
            for (index_of_input,x) in inputs.into_iter().enumerate() {

                
                let consumed_tx_hash = x.output_ref().hash().to_string();
                let consumed_utxo_index = x.output_ref().index();
                
                
                if let Some(consumed_contract) = self.state.get_by_outref(&OutputReference {
                    tx_hash: consumed_tx_hash.clone(),
                    output_index: consumed_utxo_index
                }).await {
                    
                    info!("tx {} steps contract by ref, very cool. --> {} // {}",transaction.hash(),consumed_tx_hash,consumed_contract.short_id);

                    // directly access last element as we can clearly only consume the last known utxo in the chain
                    let consumed_utxo = 
                        &consumed_contract.transitions.last()
                            .expect(&format!("Found transaction {:?} that consumes an utxo from a contract, but failed to find that utxo in the contract via utxo lookup table. This is a bug in marlowe-indexer.",transaction.hash()));
                    
                    // some basic checks just to prove that our logic always holds                    
                    if &consumed_utxo.tx_id == &format!("{consumed_tx_hash}#{consumed_utxo_index}") {
                        panic!("marlowe-indexer bug: Unmatched tx hash!")
                    }
                    if consumed_utxo.end {
                        panic!("bug in marlowe-indexer. processed a transaction that consumes the utxo of an already closed contract. This should never happen.")
                    }

                    consumed_from_marlowe_address.push(
                        ConsumedThing {
                            input: x,
                            short_id: consumed_contract.short_id.clone(), 
                            transition: (*consumed_utxo).to_owned(),
                            index_of_output: consumed_utxo_index as usize, // TODO - this is oddly named
                            index_of_input: index_of_input, // TODO - this is oddly named
                            consumed_contract: consumed_contract
                        }
                    );
                   
                }
            }
        
        }

        let datums = transaction.plutus_data();  

        let known_validator_hashes = [
            "6a9391d6aa51af28dd876ebb5565b69d1e83e5ac7861506bd29b56b0", 
            "2ed2631dbb277c84334453c5c437b86325d371f0835a28b910a91a6e"
        ];

        // index_of_output_in_this_tx/output/txhash
        let out_to_marlowe : Vec<(usize,pallas_traverse::MultiEraOutput,String)> = 
        
            transaction.outputs().into_iter().enumerate().filter_map(|(i,o)| {
                let a = o.address().unwrap();
                let possibly_payment_hash = match &a {
                    Address::Byron(_b) => None,
                    Address::Shelley(sh) => Some(sh.payment().to_hex()),
                    Address::Stake(_st) => None,
                };
                
                if let Some(hash) = possibly_payment_hash {
                    if known_validator_hashes.contains(&hash.as_ref()) {
                        Some((i,o,hash.clone()))
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
        ).collect();

        // this tx does not consume anything from any marlowe validator address, and it does not add anything to any validator address
        // therefore it can be safely ignored.
        if out_to_marlowe.is_empty() && consumed_from_marlowe_address.is_empty() {
            return
        }

        let block = block.decode().unwrap();

        let tx_hash = transaction.hash().to_string();
        
        // we now know that this tx contains inputs or outputs to the marlowe validator, so lets create a span
        // with some specific attribs attached.
        let span = tracing::info_span!("MARLOWE-TX", 
            tx_hash, 
            closed_contracts = tracing::field::Empty, 
            initialized_contracts = tracing::field::Empty, 
            stepped_contracts = tracing::field::Empty
        );

        let mut closed = 0;
        let mut initialized = 0;
        let mut stepped = 0;

        for consumed in &consumed_from_marlowe_address {
         
            if out_to_marlowe.is_empty() {

                closed = closed + 1;
                span.record("closed_contracts", closed);
                span.in_scope(||{
                    tracing::debug!("Contract {:?} was ended by tx {:?}",&consumed.short_id,transaction.hash().to_string());
                });

            }
            let redeemers = transaction.redeemers();
            if redeemers.len() > 0 {
                
                let redeemer_plutus_data = &redeemers.iter().find(|r|r.index as usize == consumed.index_of_input)
                    .expect("because this transaction consumes an utxo from the marlowe validator, there MUST be a redeemer here.").data;

                let b = redeemer_plutus_data.encode_fragment().unwrap();
                
                let mut de_merk_conts: std::collections::HashMap<String,marlowe_lang::types::marlowe::Contract> = 
                    std::collections::HashMap::new();

                let mut marlowe_redeemers = vec![];
                
                #[cfg(feature="debug")]
                let original_redeemer_bytes : Option<Vec<u8>> = Some(b.clone());

                // EXTRACT REDEEMERS AND POSSIBLY MERKLE CONTINUATIONS
                match marlowe_lang::plutus_data::from_bytes(&b) {
                    Ok(pd) => {
                        match Vec::<PossiblyMerkleizedInput>::from_plutus_data(pd, &[]) {
                            Ok(red) => {
                                marlowe_redeemers = red.clone();

                                for x in &red {
                                    match &x {
                                        PossiblyMerkleizedInput::MerkleizedInput(_a, merkle_hash_cont) => {
             
                                            let matching_datum = 
                                                datums.iter().find(|x|x.original_hash().to_string() == *merkle_hash_cont);

                                            if let Some(demerk) = matching_datum {
                                                
                                                let pd = marlowe_lang::plutus_data::from_bytes(demerk.raw_cbor())
                                                    .expect("should always be possible to decode plutus data");

                                                let decoded_contract = 
                                                    marlowe_lang::types::marlowe::Contract::from_plutus_data(pd, &[])
                                                        .expect("marlowe-rs bug: should always be possible to decode continuations since they already were successfully used on-chain.");

                                                de_merk_conts.insert(demerk.original_hash().to_string(),decoded_contract);

                                            } else {
                                                panic!("marlowe-indexer bug: we found a transaction that consumes a marlowe utxo using a merkleized input, but the tx does not include the continuation contract.")
                                            }
                                        },
                                        _ => {}
                                    }
                                }
                            },
                            Err(e) => {
                                warn!("failed to decode redeemer from plutus data! {}",e);
                            },
                        }
                    },
                    Err(e) => {
                        warn!("failed to decode redeemer plutus data!! Because the tx exists and consumes data from marlowe, the redeemer must be valid. if you see this, marlowe-indexer or marlowe-rs has a bug.. {e:?}");
                    }
                };

                if out_to_marlowe.len() > 1 {
                    panic!("not possible... cant step a marlowe contract more than one output to marlowe validator.");
                }

                // in case of a tx closing the contract, there does not need to be a datum
                let (datum_hash, datum,utxo_id,_original_datum_bytes) = 
                    if let Some(o) = out_to_marlowe.first() { 
                        let rr = read_marlowe_info_from_utxo(&o.1,&datums) ;
                        match rr {
                            Ok(d) => {
                                match d {
                                    MarloweDatumRes::Hash(h) => {
                                        (Some(h), None , Some(o.0 as f64), None)
                                    },
                                    MarloweDatumRes::Raw(hash,r, bytes) => ( Some(hash), Some(r) , Some(o.0 as f64),Some(bytes)),
                                }
                                
                            },
                            Err(rrr) => {
                                tracing::error!("Failed to read marlowe datum from utxo: {}. This is most likely a bug in marlowe-indexer or marlowe-rs.",rrr);
                                (None, None , Some(o.0 as f64), None)
                            },
                        }
                    } else {
                        (None,None,None,None)
                    };
                    
                stepped = stepped + 1;
                span.record("initialized_contracts", stepped);
                span.in_scope(||{
                    tracing::debug!("TX {} causes a transition within contract {}",transaction.hash().to_string(),consumed.short_id);
                });

                
                // obviously this contract is valid because this tx moves it.
                // therefore we dont need to check marlowe_scan - we know it exists.
                let marlowe_scan_status = None; 
                

                let meta = match transaction.metadata() {
                    pallas_traverse::MultiEraMeta::NotApplicable => None,
                    pallas_traverse::MultiEraMeta::Empty => None,
                    pallas_traverse::MultiEraMeta::AlonzoCompatible(m) => {
                        
                        let j = serde_json::to_string_pretty(m).expect("pallas bug: should always be possible to encode metadatum to json");                         
                        Some(j)
                    },
                    _ => None
                };

                #[cfg(feature="debug")]
                let (vis,ttl) = {
                    match transaction.as_alonzo() {
                        None => match transaction.as_babbage() {
                            None => (None,None),
                            Some(t) => (t.transaction_body.validity_interval_start,t.transaction_body.ttl)
                        },
                        Some(t) => (t.transaction_body.validity_interval_start,t.transaction_body.ttl)
                    }
                };

            
                let new_transition = MarloweTransition {
                    block_num : block.number().into(),
                    block: block.hash().to_string(),
                    datum,
                    datum_hash,
                    inputs: Some(marlowe_redeemers),
                    tx_id: transaction.hash().to_string(),
                    utxo_index: GraphQLHackForu64::from_usize(consumed.index_of_output),
                    abs_slot: slot.into(),
                    end: out_to_marlowe.is_empty(),
                    marlowe_scan_status,
                    meta,
                    

                    // ------- for debug feature ------------------------------------------------
                    #[cfg(feature="debug")] continuations: de_merk_conts,
                    #[cfg(feature="debug")] validity_range: (vis,ttl),
                    #[cfg(feature="debug")] original_plutus_datum_bytes: _original_datum_bytes,
                    #[cfg(feature="debug")] original_redeemer_bytes
                    // --------------------------------------------------------------------------

                };
            
                let mut contract = consumed.consumed_contract.clone();
                let mut previous_transition_immutable = 
                    contract.transitions.last_mut().expect("since we consume an utxo, it must exist");
                
                let previous_is_missing_datum = previous_transition_immutable.datum.as_ref().is_none() == true;
                if previous_is_missing_datum {
                    let hash_of_previous_tx = previous_transition_immutable.datum_hash.clone().expect("if no datum exists, there must at least exist a hash ??");
                    let d = datums.iter().find(|p| p.original_hash().to_string().eq(&hash_of_previous_tx));
                        if let Some(dd) = d{
                        let bytes = dd.raw_cbor();
                        let pd = marlowe_lang::plutus_data::from_bytes(bytes).expect("marlowe-rs bug: the transaction is recorded on chain and must therefore be valid plutus data");

                        let deserialized_datum = 
                            marlowe_lang::types::marlowe::MarloweDatum::from_plutus_data(pd,&[])
                                .expect("marlowe-rs bug: the transaction is recorded on chain and must therefore be a valid marlowe datum.");
                        previous_transition_immutable.datum = Some(deserialized_datum);
                        info!("attached datum to previous transition")
                    } 
                    else  {
                        panic!("marlowe-indexer bug: found a transaction that consumes an utxo on the marlowe validator address, but the consumed utxo has no datum, only the hash - and the consuming transaction also does not have the datum matching the hash..")
                    }

                }
                
                // store index of the last transition before adding new
                let prev_index =  contract.transitions.len() - 1;
                let is_end = new_transition.end;
                // add ref so its quick to find this one for the next transition
                contract.transitions.push(new_transition);   

                let is_closed = match &contract.transitions.last().unwrap().datum {
                    Some(c) => {
                        match c.contract {
                            marlowe_lang::types::marlowe::Contract::Close => true,
                            _ => false
                        }
                    },
                    None => true, // no datum in the output means it must be closed
                };
                

                if is_end {

                    info!("CONTRACT ENDED");
                        
                    self.apply(
                        crate::modules::marlowe::state::MarloweEvent::Close(contract.clone())
                    ).await;

                } else if is_closed {

                    if out_to_marlowe.len() != 1 {
                        panic!("closed but not end.. there must exist an output")
                    }

                    info!("CONTRACT CLOSED");

                        
                    self.apply(
                        crate::modules::marlowe::state::MarloweEvent::Close(contract.clone())
                    ).await;
                    
                } else {
                    
                    if out_to_marlowe.len() != 1 {
                        panic!("closed but not end.. there must exist an output")
                    }
                    
                    info!("CONTRACT STEPPED");
                    self.apply(
                        crate::modules::marlowe::state::MarloweEvent::Update(contract.clone())
                    ).await;
                }



            } else {
                unreachable!("it should not be possible to consume a marlowe utxo without redeemer... {:?}--{}",&consumed.short_id,transaction.hash().to_string())
            }   
        }


        if !out_to_marlowe.is_empty() || !consumed_from_marlowe_address.is_empty() {
            
            if !out_to_marlowe.is_empty() && consumed_from_marlowe_address.is_empty() {
                
                initialized = initialized + 1;
                span.record("initialized_contracts", initialized);
                span.in_scope(|| {
                    tracing::debug!("This tx contains new contracts");
                });
                
                let mut datums : std::collections::HashMap<pallas::crypto::hash::Hash<32>,&[u8]> = std::collections::HashMap::new();

                let tx_datums = transaction.plutus_data();

                for d in tx_datums {
                    datums.insert(
                        d.original_hash(),
                        d.raw_cbor()
                    );
                }

                let mut potentially_has_issues = false;
                for (i,o,validator_hash) in &out_to_marlowe {
                    
                    let (datum_hash,marlowe_datum,_original_datum_bytes) = match read_marlowe_info_from_utxo(o,&tx_datums) {
                        Ok(MarloweDatumRes::Hash(h)) => {
                            (Some(h),None,None)
                        }, 
                        Ok(MarloweDatumRes::Raw(hash,v,ob)) => (Some(hash),Some(v),Some(ob)),
                        Err(e) => {
                            potentially_has_issues = true;
                            tracing::debug!("failed to read marlowe info from utxo: {e}");
                            (None,None,None)
                        },
                    };

                    let cid = transaction.hash().to_string();

                    if datum_hash.is_none() {
                        tracing::debug!("This tx does not even have a datum hash attached, so we will ignore it. {}",&cid);
                        continue // this cannot be an initial contract since it has no datum nor any datum hash
                    }


                    let block_num = block.number();
                    
                    
                    let bid = format!("{}{}{}",block_num,tx_index,i);
                    let short_id = bs58::encode(&bid).into_string();           
                    
                    // This is a new utxo that was added to the marlowe address but we failed to decode it.
                    // We will attempt to see if marlowe scan has successfully indexed it.
                    let marlowe_scan_status = if potentially_has_issues {
                        
                        let span = trace_span!("test-marlowe-scan-status");
                        
                        let ms = 
                            get_marlowe_scan_preprod_status(&cid, *i)
                            .instrument(span.clone()).await;
                        match ms {
                            Ok(v) => {
                                if v.is_success() {
                                    tracing::error!("Detected a contract that we could not index, but that is indexed by marlowescan.org. This is likely a bug with marlowe-rs or marlowe-indexer.  ({}#{i})", transaction.hash().to_string() )
                                } else {
                                    tracing::debug!("We found an utxo on the validator address which could not be decoded, but we have now validated it against marlowe scan, and they also do not index this contract.");
                                }
                                Some(v.as_str().to_string())
                            },
                            Err(e) => {
                                tracing::error!("We were unable to reach marlowe scan to test if this contract is indexed there. If this contract is valid and any transaction later consumes it, marlowe-indexer will crash. ");
                                Some(format!("{e:?}"))
                            }
                        }
                    } else {
                        None
                    };

                    let meta = match transaction.metadata() {
                        pallas_traverse::MultiEraMeta::NotApplicable => None,
                        pallas_traverse::MultiEraMeta::Empty => None,
                        pallas_traverse::MultiEraMeta::AlonzoCompatible(m) => {
                            
                            let j = serde_json::to_string_pretty(m).expect("pallas bug: should always be possible to encode metadatum to json");                         
                            Some(j)
                        },
                        _ => None
                    };

                    #[cfg(feature="debug")]
                    let (vis,ttl) = {
                        match transaction.as_alonzo() {
                            None => match transaction.as_babbage() {
                                None => (None,None),
                                Some(t) => (t.transaction_body.validity_interval_start,t.transaction_body.ttl)
                            },
                            Some(t) => (t.transaction_body.validity_interval_start,t.transaction_body.ttl)
                        }
                    };
    
                    // let vis = transaction.as_alonzo().unwrap().transaction_body.validity_interval_start;
                    // let ttl = transaction.as_alonzo().unwrap().transaction_body.ttl;
                 
                    let contract = Contract {
                            id : format!("{}#{}",transaction.hash().to_string(),i),                    
                            short_id,
                            validator_hash: validator_hash.to_string(),
                            transitions: vec![
                                MarloweTransition {
                                    inputs: None,
                                    block_num : block.number().into(),
                                    block: block.hash().to_string(),
                                    datum: marlowe_datum,
                                    datum_hash,
                                    tx_id: transaction.hash().to_string(),
                                    utxo_index: GraphQLHackForu64::from_usize(*i),
                                    abs_slot: slot.into(),
                                    end: false,
                                    marlowe_scan_status,
                                    meta,
                                    
                                    

                                    // ------- for debug feature ------------------------------------------------
                                    #[cfg(feature="debug")] continuations: std::collections::HashMap::new(),
                                    #[cfg(feature="debug")] validity_range: (vis,ttl),
                                    #[cfg(feature="debug")] original_plutus_datum_bytes: _original_datum_bytes,
                                    #[cfg(feature="debug")] original_redeemer_bytes: None
                                    // --------------------------------------------------------------------------
                                    
                                }
                            ]
                        };

                    _ = self.apply(crate::modules::marlowe::state::MarloweEvent::Add(contract.clone())).await;

                    let new_count = self.state.contracts_count();                    
                
                    tracing::debug!("New contract found in block {}: {}#{}. There are now {} known contracts",block.number().to_string(),transaction.hash(),i,&new_count);
                  
                    
                }
            }

        }
       
    
    }

}


pub enum MarloweDatumRes {
    Hash(String),
    Raw(String,marlowe_lang::types::marlowe::MarloweDatum,Vec<u8>)
}

fn read_marlowe_info_from_utxo(o:&MultiEraOutput,datums:&[pallas::codec::utils::KeepRaw<'_, pallas_primitives::babbage::PlutusData>]) -> Result<MarloweDatumRes,String> {
    match o.datum() {
        Some(x) => {
            match x {
                pallas_primitives::babbage::PseudoDatumOption::Hash(datum_hash) => {

                    let matching_datum = 
                        datums.iter().find(|x|x.original_hash() == datum_hash);

                    if let Some(datum) = matching_datum {
                        let bytes = datum.raw_cbor().to_owned();   
                                     
                        let pd = marlowe_lang::plutus_data::from_bytes(&bytes);
                        match pd {
                            Ok(d) => {
                                let hash = datum_hash.to_string();
                                Ok(MarloweDatumRes::Raw(hash,MarloweDatum::from_plutus_data(d, &[])?,bytes))       
                            },
                            Err(e) => {
                                warn!("failed to deserialize datum. error: {:?}",e);
                                Err(format!("Failed to decode datum in to plutus data. Error: {e:?}"))
                            },
                        }
                        
                     
                    } else {
                        Ok(MarloweDatumRes::Hash(datum_hash.to_string()))
                        //Err(format!("datumHash: {}",datum_hash.to_string()))
                    }

                },
                pallas_primitives::babbage::PseudoDatumOption::Data(datum) => {
                    let hash = datum.original_hash().to_string();
                    let bytes = datum.raw_cbor().to_owned();
                    let xhex = hex::encode(&bytes);                    
                    Ok(MarloweDatumRes::Raw(hash,marlowe_lang::extras::utils::try_decode_cborhex_marlowe_plutus_datum(&xhex)?,bytes))      
                },
            }
        },
        None => Err(String::from("no datum attached")),
    }
}


// TEMP : For contracts we cannot decode or otherwise have issues processing, we will call marlowe scan to see if they have processed the tx successfully.
// Logic being that if cant process it, and marlowescan has not been able to either, then the contract is most likely just invalid, but if marlowescan
// has it, then we probably have a bug in marlowe-rs/marlowe-indexer.
async fn get_marlowe_scan_preprod_status(initial_tx_hash:&str,utxo_index:usize) -> Result<reqwest::StatusCode,reqwest::Error> {
    let response = reqwest::get(format!("https://preprod.marlowescan.com/isContractOpen?contractId={initial_tx_hash}%23{utxo_index}")).await?;
    Ok(response.status())
}