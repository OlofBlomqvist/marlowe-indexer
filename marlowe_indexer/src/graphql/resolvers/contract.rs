use std::collections::HashSet;

use async_graphql::Object;
use marlowe_lang::semantics::{ContractSemantics, MachineState};

use crate::state::Contract;


#[Object]
impl Contract {

    // ===================================================================================================================
    // >> DEBUG FIELDS IN HERE
    // ===================================================================================================================

    /// WARNING: This information should not normally be queried as it is rather slow. 
    /// It can be used to validate the marlowe-rs contract simulation state-machine implementation 
    /// such that each redeemer is applied to the previous datum, validating the resulting output datum
    /// which should always match the actual datum as seen on chain. If at any point these differ,
    /// we know there is a bug in marlowe-rs. This property will return null if there are no issues detected.
    /// Note that this is only for testing on PREPROD.
    #[cfg(feature="debug")]
    async fn describe_debug(&self) -> String {
        crate::graphql::resolvers::contract_debug::describe_complete(self)
    }

    /// WARNING: This information should not normally be queried as it is rather slow. 
    /// It can be used to validate the marlowe-rs contract simulation state-machine implementation 
    /// such that each redeemer is applied to the previous datum, validating the resulting output datum
    /// which should always match the actual datum as seen on chain. If at any point these differ,
    /// we know there is a bug in marlowe-rs. This property will return null if there are no issues detected.
    /// Note that this is only for testing on PREPROD.
    #[cfg(feature="debug")]
    async fn marlowe_rs_test(&self) -> String {
        super::contract_debug::marlowe_rs_test(self)
    }

    // ===================================================================================================================

    /// Describe the current contract state.
    /// This will only provide very limited information regarding merkleized contracts.
    async fn describe(&self) -> Result<Option<String>,String> {

        let current_utxo = self.transitions.last().unwrap();
        
        if current_utxo.end {return Ok(Some(String::from("Closed")))}

        if let Some(d) = &current_utxo.datum {

            let contract_instance = marlowe_lang::semantics::ContractInstance::from_datum(&d);

            let (_a,b) = contract_instance.step(false).unwrap();
            if let &MachineState::Closed = &b {
                Ok(Some(String::from("Pending close: the utxo still lives on the marlowe validator address and is waiting to be closed/consumed.")))
            } else {
                Ok(Some(format!("{}",serde_json::to_string_pretty(&b).map_err(|e|format!("{e:?}"))?)))
            }
        } else {
            Ok(Some(String::from("This contract has no current datum available. Perhaps it uses merkleization, or only has the datum-hash attached to the utxo pending the next input. Anyhow - we cannot tell you more about this one!")))
        }

    }

    /// true if the contract expects a notification to be made, and we can tell that 
    /// any notification sent at this time will cause the notification to be true.
    async fn expected_notification_is_true(&self) -> Option<bool> {None}

    /// The datum of the last observed utxo in the contract chain
    async fn datum_json(&self) -> Option<String> {
        let x = self.transitions.last().expect("marlowe-indexer bug: any contract must have at least one transition");
        x.datum.as_ref().map(|d| marlowe_lang::serialization::json::serialize(d).unwrap_or_default())
    }

    /// Script hash of the marlowe payout validator script used for this contract
    async fn payout_validator_hash(&self) -> Option<String> {
        let x = self.transitions.last().expect("marlowe-indexer bug: any contract must have at least one transition");
        x.datum.as_ref().map(|ei| ei.marlowe_params.0.to_string())
    }

    /// The minimum time at which point this contract will be possible to transition to a possible next stage.
    async fn min_time(&self) -> Option<u64> {
        let x = self.transitions.last().expect("marlowe-indexer bug: any contract must have at least one transition");
        x.datum.as_ref().map(|ei| ei.state.min_time)
    }

    /// Current contract definition in DSL format
    async fn contract_dsl(&self) -> Option<String> {
        let x = self.transitions.last().expect("marlowe-indexer bug: any contract must have at least one transition");
        x.datum.as_ref().map(|ei| ei.contract.to_dsl())
    }

    /// Current contract definition in JSON format
    async fn contract_json(&self) -> Option<String> {
        let x = self.transitions.last().expect("marlowe-indexer bug: any contract must have at least one transition");
        x.datum.as_ref().map(|ei| ei.contract.to_json().expect("marlowe-rs bug: should always be possible to serialize on-chain parties"))
    }

    /// List of all remaining contract participants.
    async fn remaining_participants(&self) -> Vec<String> {
        let mut results  = vec![];
        let x = self.transitions.last().expect("marlowe-indexer bug: any contract must have at least one transition");
        if let Some(ei) = &x.datum {
            for p in ei.contract.parties() {
                results.push(p.to_string())
            }
        } 
        results
    }

    /// List of all known contract participants.
    async fn participants(&self) -> Vec<String> {
        let mut results  = HashSet::new();
        for t in &self.transitions {
            if let Some(ei) = &t.datum {
                for p in ei.contract.parties() {
                    results.insert(p);
                }
            } 
        }
        results.iter().map(|x|x.to_string()).collect()
    }

    /// JSON serialized representation of the current contract state.
    async fn contract_state_json(&self) -> Option<String> {
        let x = self.transitions.last().expect("marlowe-indexer bug: any contract must have at least one transition");
        x.datum.as_ref().map(|ei| marlowe_lang::serialization::json::serialize(ei.state.clone()).expect("marlowe-rs bug: should always be possible to serialize on-chain data"))
    }

    
    async fn has_timed_out(&self) -> bool {
        let x = self.transitions.last().expect("marlowe-indexer bug: any contract must have at least one transition");
        if let Some(ei) = &x.datum {
           let machine = marlowe_lang::semantics::ContractInstance::from_datum(ei);
            machine.has_timed_out()
        } else {
            false
        }
    }

    #[cfg(feature="debug")] 
    async fn continuations(&self) -> std::collections::HashMap<String,String> {
        let mut result : std::collections::HashMap<String,String>  = std::collections::HashMap::new();
        for x in &self.transitions {
            for (hash,data) in &x.continuations {
                result.insert(hash.clone(),data.to_dsl());
            }
        }
        result
    }

    /// Available only if the contract is in a state at which it may time out.
    /// This will be a negative value if the contract has already timed out.
    async fn next_time_out(&self) -> Result<Option<f64>,String> {
        let x = self.transitions.last().expect("marlowe-indexer bug: any contract must have at least one transition");
        if let Some(ei) = &x.datum {
           let machine = marlowe_lang::semantics::ContractInstance::from_datum(ei);
           if machine.has_timed_out() {
             return Ok(Some(-1.0))
           }
           let (a,_b) = machine.step(false)?;
           match a.contract {
                marlowe_lang::types::marlowe::Contract::When { when:_ ,timeout: Some(t), timeout_continuation:_ } => {
                    match t {
                        marlowe_lang::types::marlowe::Timeout::TimeConstant(tt) => Ok(Some(tt as f64)),
                        marlowe_lang::types::marlowe::Timeout::TimeParam(p) => Err(p),
                    }
                },
                _ => Ok(None)
            }
        } else {
            Ok(None)
        }
    }

    /// Base58 encoded version of block_num,tx_num,utxo_index. Used as cursor for marlowe-indexer.
    async fn short_id(&self) -> &str {
        &self.short_id
    }
    
    /// The hash of the transaction that initiated the contract utxo chain
    async fn id(&self) -> &str {
        &self.id
    }
    
    /// The marlowe validator hash where the contracts utxo chain lives.
    async fn validator_hash(&self) -> &str {
        &self.validator_hash
    }

    /// All observed states that this contract has been in
    async fn transitions(&self,filter:Option<crate::graphql::types::TransitionFilter>) -> Vec<&crate::state::MarloweTransition> {
        let results = self.transitions.iter().filter(|x| {
            
            if let Some(f) = &filter {
                
                if let Some(end_filter) = f.end { 
                    if end_filter != x.end { return false } 
                }
                
                if let Some(slot_filter) = f.slot { 
                    if slot_filter != x.abs_slot { return false } 
                }

                if let Some(block_num_filter) = f.block_num { 
                    if block_num_filter != x.block_num { return false } 
                }

                if let Some(block_hash_filter) = &f.block_hash { 
                    if *block_hash_filter != x.block { return false } 
                }
                
                if let Some(tx_id) = &f.tx_id { 
                    if tx_id != &x.tx_id { return false } 
                }
                
                if let Some(marlowe_scan_status_filter) = &f.marlowe_scan_status { 
                    match &x.marlowe_scan_status {
                        None => return false,
                        Some(marlowe_status) => {
                            let found_match = marlowe_status.contains(marlowe_scan_status_filter);
                            if !found_match { return false }
                        }
                    }                    
                }

                true
            } 
            else {
                true
            }
            
        }).collect::<Vec<&crate::state::MarloweTransition>>();

        if let Some(f) = &filter {
            if let Some(true) = f.last {
                vec![results.last().unwrap()]
            } else {
                results
            }
        } else {
            results
        }

    }
}
