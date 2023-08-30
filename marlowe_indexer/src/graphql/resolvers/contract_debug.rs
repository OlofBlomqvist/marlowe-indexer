use marlowe_lang::semantics::ContractSemantics;


#[cfg(feature="debug")]
use crate::{graphql::{slot_to_posix_time,types::SlotConfig}, state::Contract};


#[cfg(feature="debug")]
pub(crate) fn marlowe_rs_test(contract:&Contract) -> String {
    use marlowe_lang::plutus_data::ToPlutusData;

    
    let preprod_slot_config = SlotConfig {
        zero_time: 1655683200000,
        zero_slot: 0,
        slot_length: 1000,
    };

    for window  in contract.transitions.windows(2) {
        
        if let [first, second] = window {

            let slot_time = slot_to_posix_time(second.abs_slot as u64, &preprod_slot_config);
            let time_interval_end = slot_to_posix_time(second.validity_range.1.expect("cant run contracts without ttl"),&preprod_slot_config)  - 1;
            
            let first_datum: &marlowe_lang::types::marlowe::MarloweDatum = 
                first.datum.as_ref().expect(&format!("there must be a datum or there could not be a second transition: {}",contract.id));

            let mut instance = 
                marlowe_lang::semantics::ContractInstance::from_datum(
                    first_datum
                ).with_current_time(slot_time)
                 .with_time_interval_end(time_interval_end);

            for (merkle_hash,contract) in &second.continuations {
                instance.add_continuation(merkle_hash, contract)
            } 

            if let Some(inputs) =  &second.inputs {
                for action in inputs{
                    
                    let action = match &action {
                        marlowe_lang::types::marlowe::PossiblyMerkleizedInput::Action(action) => action,
                        marlowe_lang::types::marlowe::PossiblyMerkleizedInput::MerkleizedInput(action, hash) => {
                            if !second.continuations.contains_key(hash) {
                                return format!("a merkleized input was found using hash {hash}, but that continuation is missing from the continuations map of the transaction")
                            }
                            action
                        },
                    };
                    
                    let r = instance.apply_input_action(action.clone());

                    match r {
                        Err(e) => return format!("failed to apply an input! {e:?}"),
                        Ok(v) => instance = v                        
                    }
                }
            }

            let (new_instance,_new_state) = 
                instance.process().expect(&format!("should always be possible to process.. {}",contract.id));
            
            if new_instance.warnings.len() > 0 {
                return format!("Contains Warnings: \n {:#?}",new_instance.warnings)
            }

            let mut calculated_datum_as_per_marlowe_rs = new_instance.as_datum();
            if second.end {
                if &calculated_datum_as_per_marlowe_rs.contract.to_dsl() != "Close" {
                    return format!("Found a transition that marks the end of a contract on-chain, but marlowe rs shows a different result after applying the inputs: {calculated_datum_as_per_marlowe_rs:?}")
                }
            } else {
                let second_datum = second.datum.as_ref().expect("marlowe-indexer bug: any transition that is not final must have a datum");
                
                if second_datum.contract != calculated_datum_as_per_marlowe_rs.contract {
                    return 
                        format!(
                            "MISSMATCH!!! marlowe_rs resulted in different continuations: \n\n'{}'\n\n, while actual result on-chain is known to be this: \n\n{}",
                                calculated_datum_as_per_marlowe_rs.contract.to_json()
                                .expect("marlowe-rs bug: should always be possible to serialize existing contracts to json"),
                                second_datum.contract.to_json()
                                .expect("marlowe-rs bug: should always be possible to serialize existing contracts to json")
                        )
                }

                // we cant control how marlowe runtime (the haskell impl) sets min-time
                // so we just override this here. marlowe-rs just uses the current time for now..
                calculated_datum_as_per_marlowe_rs.state.min_time = second_datum.state.min_time;
                
                if slot_time < second_datum.state.min_time {
                    panic!("marlowe-indexer bug: slottime less than secondary min_time")
                }

                if &second_datum.state.to_string() != &calculated_datum_as_per_marlowe_rs.state.to_string() {

                    let the_applied_input = second.inputs.clone().unwrap();
                    let inp = format!("{:?}",the_applied_input);
                    let dsl = first.datum.clone().unwrap().contract.to_dsl();
                    let logs = new_instance.logs.join("\n");
                    return 
                        format!( 
                            "marlowe-rs bug: marlowe_rs resulted in different state ({} ----> {}) [INPUTS: {inp:?}]: \n\n'{}'\n\n, while actual result on-chain is known to be: \n\n{} .... the dsl we applied input to was: {dsl} ..... machine logs: {logs}", first.tx_id,second.tx_id,
                            marlowe_lang::serialization::json::serialize(calculated_datum_as_per_marlowe_rs.state)
                            .expect("marlowe-rs bug: should always be possible to serialize existing contracts to json"),
                            marlowe_lang::serialization::json::serialize(&second_datum.state)
                                .expect("marlowe-rs bug: should always be possible to serialize existing contracts to json"), 
                                
                        )
                }

                if second_datum.marlowe_params.0 != calculated_datum_as_per_marlowe_rs.marlowe_params.0 {
                    return format!("marlowe_rs resulted in different params: \n'{}'\n, while actual result on-chain is known to be: \n{}",
                        calculated_datum_as_per_marlowe_rs.marlowe_params.0,second_datum.marlowe_params.0)
                }

                // todo: compare cbor encoding as well:

                let mrs_cbor = calculated_datum_as_per_marlowe_rs.to_plutus_data(&vec![])
                     .expect("it should always be possible to convert mrs datum to cbor");

                //let calculated_cbor_hex = marlowe_lang::plutus_data::to_hex(&mrs_cbor).expect("marlowe-rs bug: should always be possible to encode existing, known-to-be-vald plutus-data to hex");

                let calc_bytes = marlowe_lang::plutus_data::to_bytes(&mrs_cbor).expect("should always be possible to convert plutus data to bytes");
                let original_bytes = second.original_plutus_datum_bytes.as_ref().unwrap().clone();
                
                if !calc_bytes.eq(&original_bytes) {
                    
                    let (our_hex,original_hex) = (hex::encode(calc_bytes),hex::encode(original_bytes));

                    return format!("calculated bytes do not equal original. ({} --> {})  calculated: {our_hex}\n\noriginal: {original_hex}\n\nContract DSL of the result: {}\n\nState of the result: {:?}",
                    first.tx_id,second.tx_id, calculated_datum_as_per_marlowe_rs.contract.to_dsl(), calculated_datum_as_per_marlowe_rs.state)
                } 

                if new_instance.warnings.len() > 0 {
                    return format!("New instance has warnings: {:#?}",new_instance.warnings)
                }
            }
            
        }
    }

   // info!("MARLOWE-RS TEST OF CONTRACT {} WAS SUCCESSFULL!",self.short_id);
    "OK".into()
}


// This method supports fully describing merkleized contracts which are only available in debug mode.
#[cfg(feature="debug")]
pub(crate) fn describe_complete(contract:&Contract) -> String {
    

    let x = contract.transitions.last().unwrap();
        
    if x.end.eq(&true) {
        return String::from("This contract is Closed")
    }

    
    let d = x.datum.as_ref().expect(&format!("marlowe-indexer bug: we expect that any contract that has not been closed have a datum. ({})", contract.id));
    


    let mut accly = vec![];
    let accs : marlowe_lang::types::marlowe::AccMap<(marlowe_lang::types::marlowe::Party,marlowe_lang::types::marlowe::Token),u128>= d.state.accounts.clone();
    for ((pt,tok),val) in accs.iter() {
        
        let pstring = match &pt {
            &marlowe_lang::types::marlowe::Party::Address(a) => format!("{}",a.as_bech32().unwrap()),
            &marlowe_lang::types::marlowe::Party::Role { role_token } => role_token.to_string()
        };

        let tokstr = match &tok {
            &marlowe_lang::types::marlowe::Token { token_name, currency_symbol } => format!("{currency_symbol}.{token_name}")
        };

        let row = format!("{pstring} ; {tokstr} ; {val}");
        accly.push(row);
    }

    match &d.contract {
        marlowe_lang::types::marlowe::Contract::Close => {                
            let locked = d.state.locked_amounts();
            let locked_amounts : Vec<&(&marlowe_lang::types::marlowe::Party,&marlowe_lang::types::marlowe::Token,u128)> = 
                locked.iter().filter(|(_owner,_token,amount)| *amount > 0).collect();
            if locked_amounts.is_empty() {
                return String::from("This contract is of type 'Close'. The only possible transaction remaining is one that completes the contract.")
            } else {
                return format!("This contract is of type 'Close'. The only possible transaction remaining is one that completes the contract. The transaction that completes this contract must distribute the remaining assets : {locked_amounts:#?}")
            }
        },
        _ => {}
    };

    let mut contract_instance = marlowe_lang::semantics::ContractInstance::from_datum(d);
  
    for (hash,cont) in &x.continuations {
        contract_instance.add_continuation(hash, cont)
    }
    
    // running process here will step the contract forward until it requires an input action or is closed.
    // this effectively means that, the result of processing here is that we get the contract in the same state that
    // the marlowe validator will see before attempting to apply any new inputs to the contract.
    let (a,b) = marlowe_lang::semantics::ContractSemantics::process(&contract_instance).unwrap();


    match &b {
        marlowe_lang::semantics::MachineState::Closed => {
            let locked = d.state.locked_amounts();
            let locked_amounts : Vec<&(&marlowe_lang::types::marlowe::Party,&marlowe_lang::types::marlowe::Token,u128)> = 
                locked.iter().filter(|(_owner,_token,amount)| *amount > 0).collect();
            if locked_amounts.is_empty() {
                return String::from("Pending Close: The only possible transaction remaining is one that completes the contract. This state is expected to occur very rarely on-chain.")
            } else {
                return format!("Pending Close: The only possible transaction remaining is one that completes the contract. The transaction that completes this contract must distribute the remaining assets : {locked_amounts:#?}")
            }
        }
        marlowe_lang::semantics::MachineState::Faulted(e) => format!("marlowe-rs failed to process the contract: {e}"),
        marlowe_lang::semantics::MachineState::ContractHasTimedOut => {
            match a.contract {
                marlowe_lang::types::marlowe::Contract::When { when:_, timeout, timeout_continuation } => {
                    format!("This contract has timed (at {timeout:?}) in a 'when' branch, and requires a transaction to move the contract based on the continuation: {timeout_continuation:?}")
                },
                _ => panic!("marlowe-rs bug: a contract was found to be timed out, but the current contract type is not WHEN. This should not be possible.")
            }                
        },
        marlowe_lang::semantics::MachineState::WaitingForInput { expected, timeout } => {
            format!("This contract is waiting (until {timeout}) for one of the following inputs to be applied: \n{expected:#?}")
        },
        marlowe_lang::semantics::MachineState::ReadyForNextStep => unreachable!()
    }

}
