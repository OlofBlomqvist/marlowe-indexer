use crate::graphql::types::*;
use async_graphql::*;
use marlowe_lang::plutus_data::ToPlutusData;
use marlowe_lang::types::marlowe::AccMap;
use marlowe_lang::semantics::ContractSemantics;
use pallas_primitives::Fragment;
use pallas_traverse::ComputeHash;
use tracing::info;
use async_graphql::{
    connection::{Connection, Edge},
    Result
};

use crate::state::{Contract, OrderedContracts};
use std::collections::HashSet;

#[Object]
impl QueryRoot {

    // TODO: Add some actual statistics.
    #[tracing::instrument(skip_all)]
    async fn stats<'ctx>(
        &self,
        ctx: &Context<'ctx>
    ) -> Result<String> {
        let my_context = ctx.data::<std::sync::Arc<tokio::sync::RwLock<crate::state::State>>>().unwrap();
        let read_guard = my_context.read().await;
        let s = format!("{:?}",read_guard.ordered_contracts.utxo_to_contract_lookup_table);
        
        Ok(s)
    }

    // todo: filter on sub fields ? ie, not in the main filter
    #[tracing::instrument(skip_all)]
    async fn contracts<'ctx>(
        &self,
        ctx: &Context<'ctx>,
        filter: Option<ContractsFilter>, // APPLIED AFTER HAVING SLICED THE DATA USING "AFTER" AND "BEFORE"
        pagination: Option<Pagination>,
    ) -> Result<Connection<String, Contract, ConnectionFields>> {

        let (first,last,before,after,page,page_size) = match pagination {
            Some(p) => (p.first,p.last,p.before,p.after,p.page,p.page_size),
            None => (None,None,None,None,None,None)
        };

        if first.is_some() && last.is_some() {
            return Err("Cannot use both 'first' and 'last' at the same time.".into());
        }
    
        if let Some(selected_page) = page {
            if selected_page < 1.0 {
                return Err("Minimum selectable page is 1".into())
            }
        }

        let my_context = ctx.data::<std::sync::Arc<tokio::sync::RwLock<crate::state::State>>>().unwrap();
        let read_guard = my_context.read().await;
        let contracts = &read_guard.ordered_contracts;
        
        info!("Handling request to list contracts");
        match testable_query(contracts,QueryParams { filter, after, before, first, last, page_size, page }).await {
            Ok(r) => Ok(r),
            Err(e) => Err(e.into()),
        }
        
    }
    
    
    #[tracing::instrument(skip_all)]
    async fn current_block<'ctx>(&self,ctx: &Context<'ctx>) -> Option<String> {
        let my_context = ctx.data::<std::sync::Arc<tokio::sync::RwLock<crate::state::State>>>().unwrap();
        my_context.read().await.last_seen_block().as_ref().map(|block_id| block_id.clone().to_string())
    }
}

pub fn locked_amounts_filter_matches(filter:&LockedAmountFilter,contract:&Contract) -> bool {

    let current_transition = contract.transitions.last().expect("marlowe-indexer bug: there must be at least one transition or there would be no contract");

    let current_datum = if let Some(d) = &current_transition.datum { d } else {
        // if the current state has no datum, it means the contract has ended,
        // so this filter would only match if someone searches for contracts that have NO locked amounts 
        return 
            filter.account_owner.is_none()
            && filter.amount.is_none()
            && filter.token_name.is_none()
            && filter.currency_symbol.is_none()
            && filter.number_of_accounts.is_none()
            // ^ if you add a new locked amount filter, do not forget to add it here
    };

    if !num_check_opt(&filter.number_of_accounts, current_datum.state.accounts.len() as f64) {
        return false;
    }
    

    let mut result = vec![];
    let thing : AccMap<(marlowe_lang::types::marlowe::Party,marlowe_lang::types::marlowe::Token),u128> = current_datum.state.accounts.clone();
    for ((account_owner,token),amount) in thing.iter() {
        if *amount > 0 {
            match account_owner {
                marlowe_lang::types::marlowe::Party::Address(a) => {
                    let o = a.as_bech32().expect("marlowe-rs bug: failed to convert address to bech32");
                    result.push(
                        LockedAmountsResult {
                            account_owner_role: None,
                            account_owner_addr: Some(o),
                            currency_symbol: token.currency_symbol.clone(),
                            token_name: token.token_name.clone(),
                            amount: *amount as f64
                        }
                    )
                },
                marlowe_lang::types::marlowe::Party::Role {role_token} => { 
                    let o = role_token.to_owned();
                    result.push(
                        LockedAmountsResult {
                            account_owner_role: Some(o),
                            account_owner_addr: None,
                            currency_symbol: token.currency_symbol.clone(),
                            token_name: token.token_name.clone(),
                            amount: *amount as f64
                        }
                    )
                }
            }
            
        }
    }
    
    for x in result  {


        // Filter by amounts
        let amounts_match = num_check_opt(&filter.amount, x.amount as f64);
        
        // Filter by currency_symbol
        let symb_match = str_check_opt(&filter.currency_symbol, &x.currency_symbol);
        
        // Filter by token_name
        let tok_name_match = str_check_opt(&filter.token_name, &x.token_name);
        
        // Filter by owner     
        let owner_match = match &filter.account_owner {
            Some(af) => {
                match af {
                    AccountOwnerFilter::Role(role_filter) => {
                        if let Some(aor) = &x.account_owner_role {
                            str_check(role_filter,&aor)
                        } else {
                            false
                        }
                    },
                    AccountOwnerFilter::Address(addr_filter) => {
                        if let Some(aoa) = &x.account_owner_addr {
                            str_check(addr_filter,&aoa)
                        } else {
                            false
                        }
                    },
                }
            },
            None => true, // any owner
        };

        // at least one account in the contracts current state matches the filter
        if amounts_match && symb_match && tok_name_match && owner_match {
            return true;
        }

        
    }

    return false

}

#[Object]
impl crate::state::MarloweTransition {

    async fn meta(&self) -> &Option<String> { &self.meta }
    
    //async fn validity_range(&self) -> String { format!("{:#?}",&self.validity_range) }
    

    async fn datum_json(&self) -> Option<String> { if let Some(d) = &self.datum { 
        Some(marlowe_lang::serialization::json::serialize(d).unwrap_or_default())
    }  else { None }}
    
    async fn inputs_json(&self) -> Option<String> {
        if self.inputs.is_none() { return None }
        Some(marlowe_lang::serialization::json::serialize(&self.inputs).unwrap_or_default())
    } 
    async fn inputs(&self) -> Option<Vec<String>> {
        
        if let Some(items) = &self.inputs {
            let mut inputs = vec![];
            for x in items {
                inputs.push(x.to_string())
            }
            Some(inputs)
        } else {
            None
        }
    } 

    //async fn datum(&self) -> &Option<marlowe_lang::types::marlowe::MarloweDatum> { &self.datum }    
    async fn tx_id(&self) -> &str { &self.tx_id }
    async fn utxo_index(&self) -> &Option<f64>{ &self.utxo_index }
    async fn end(&self) -> &bool { &self.end }
    async fn slot(&self) -> &f64 { &self.abs_slot }
    async fn block_hash(&self) -> &str { &self.block }
    async fn block_num(&self) -> &f64 { &self.block_num }
    async fn invalid(&self) -> &bool { &self.invalid }
    async fn issues(&self) -> &Vec<std::string::String> { &self.issues }
    async fn marlowe_scan_status(&self) -> &Option<String> { 
         &self.marlowe_scan_status
    }

    // this seems to only return the first item?
    async fn locked_amounts(&self) -> Option<Vec<LockedAmountsResult>> {
        match &self.datum {
            None => None,
            Some(d) => {
                let mut result = vec![];
                let thing : AccMap<(marlowe_lang::types::marlowe::Party,marlowe_lang::types::marlowe::Token),u128> = d.state.accounts.clone();
                for ((account_owner,token),amount) in thing.iter() {
                    if *amount > 0 {
                        match account_owner {
                            marlowe_lang::types::marlowe::Party::Address(a) => {
                                let o = a.as_bech32().expect("marlowe-rs bug: failed to convert address to bech32");
                                result.push(
                                    LockedAmountsResult {
                                        account_owner_role: None,
                                        account_owner_addr: Some(o),
                                        currency_symbol: token.currency_symbol.clone(),
                                        token_name: token.token_name.clone(),
                                        amount: *amount as f64
                                    }
                                )
                            },
                            marlowe_lang::types::marlowe::Party::Role {role_token} => { 
                                let o = role_token.to_owned();
                                result.push(
                                    LockedAmountsResult {
                                        account_owner_role: Some(o),
                                        account_owner_addr: None,
                                        currency_symbol: token.currency_symbol.clone(),
                                        token_name: token.token_name.clone(),
                                        amount: *amount as f64
                                    }
                                )
                            }
                        }
                       
                    }
                }
                Some(result)

            }
        }

    }
}

pub fn num_check_opt(filter:&Option<NumFilter>,input_number:f64) -> bool {
    if let Some(f) = filter { num_check(f,input_number) } else { true }
}

pub fn str_check_opt(filter:&Option<StringFilter>,input_str:&str) -> bool {
    if let Some(f) = filter { str_check(f,input_str) } else { true }
}
pub fn str_check_opt_opt(filter:Option<&StringFilter>,input_str:Option<&str>) -> bool {
    if let Some(f) = filter { str_check(f,input_str.unwrap_or_default()) } else { true }
}

pub fn num_check(filter:&NumFilter,input_number:f64) -> bool {
    match filter {
        NumFilter::Eq(n)  => input_number == *n,
        NumFilter::Gt(n)  => input_number >  *n,
        NumFilter::Lt(n)  => input_number <  *n,
        NumFilter::Lte(n) => input_number <= *n,
        NumFilter::Gte(n) => input_number >= *n
    }
}

pub fn str_check(filter:&StringFilter,input_str:&str) -> bool {
    match filter {
        StringFilter::Eq(s) => input_str == s,
        StringFilter::Neq(s) => input_str != s,
        StringFilter::Contains(s) => input_str.contains(s),
        StringFilter::NotContains(s) => !input_str.contains(s)
    }
}

fn slot_to_posix_time(slot: u64, sc: &SlotConfig) -> u64 {
    let ms_after_begin = (slot - sc.zero_slot) * sc.slot_length;
    sc.zero_time + ms_after_begin
}

fn posix_time_to_slot(posix_time: u64, sc: &SlotConfig) -> u64 {
    ((posix_time - sc.zero_time) / sc.slot_length) + sc.zero_slot
}


#[cfg(feature="debug")]
fn marlowe_rs_test(contract:&Contract) -> String {
    
    let preprod_slot_config = SlotConfig {
        zero_time: 1655683200000,
        zero_slot: 0,
        slot_length: 1000,
    };

    for window  in contract.transitions.windows(2) {
        
        if let [first, second] = window {

            let slot_time = slot_to_posix_time(second.abs_slot as u64, &preprod_slot_config);
            let time_interval_end = slot_to_posix_time(second.validity_range.1.expect("cant run contracts without ttl"),&preprod_slot_config)  - 1;
            
            if !first.issues.is_empty() && !second.issues.is_empty() {
                return "refer to the issues property of this contract".into()
            }
            
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

                    let hexly = compare_hex_strings(&our_hex,&original_hex);

                    return format!("calculated bytes do not equal original. ({} --> {})  calculated: {our_hex}\n\noriginal: {original_hex}\n\nContract DSL of the result: {}\n\nState of the result: {:?}\n\n\n---> hexly: {hexly}",
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

fn compare_hex_strings(a: &str, b: &str) -> String {
    let mut result = String::new();
    let mut markers = String::new();

    let chars_a = a.chars();
    let chars_b = b.chars();

    for (char_a, char_b) in chars_a.zip(chars_b) {
        if char_a == char_b {
            result.push(char_a);
            markers.push(' ');
        } else {
            result.push(char_b);
            markers.push('+');
        }
    }

    // Handle extra characters in either string
    for char_a in a.chars().skip(b.len()) {
        markers.push('-');
    }
    for char_b in b.chars().skip(a.len()) {
        result.push(char_b);
        markers.push('+');
    }

    result.push('\n');
    result.push_str(&markers);

    result
}

#[Object]
impl Contract {

    /// WARNING: This information should not normally be queried as it is rather slow. 
    /// It can be used to validate the marlowe-rs contract simulation state-machine implementation 
    /// such that each redeemer is applied to the previous datum, validating the resulting output datum
    /// which should always match the actual datum as seen on chain. If at any point these differ,
    /// we know there is a bug in marlowe-rs. This property will return null if there are no issues detected.
    /// Note that this is only for testing on PREPROD.
    #[cfg(feature="debug")]
    async fn marlowe_rs_test(&self) -> String {
        marlowe_rs_test(self)
    }

    // TODO: properly format expected distribution details for closure.
    /// This property will provide a human readable description of the contracts current status.
    async fn description(&self) -> String {


        let x = self.transitions.last().unwrap();
        
        if x.end.eq(&true) {
            return String::from("This contract is Closed")
        }

        
        let d = x.datum.as_ref().expect(&format!("marlowe-indexer bug: we expect that any contract that has not been closed have a datum. ({})", self.id));
        


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


        //return serde_json::to_string_pretty(&accly).unwrap();


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

        let mut contract_instance = marlowe_lang::semantics::ContractInstance::from_datum(&d);

        for (hash,cont) in &x.continuations {
            contract_instance.add_continuation(&hash, cont)
        }

        // running process here will step the contract forward until it requires an input action or is closed.
        // this effectively means that, the result of processing here is that we get the contract in the same state that
        // the marlowe validator will see before attempting to apply any new inputs to the contract.
        let (a,b) = contract_instance.process().unwrap();

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

    /// true if the contract expects a notification to be made, and we can tell that 
    /// any notification sent at this time will cause the notification to be true.
    async fn expected_notification_is_true(&self) -> Option<bool> {None}

    /// The datum of the last observed utxo in the contract chain
    async fn datum_json(&self) -> Option<String> {
        let x = self.transitions.last().expect("marlowe-indexer bug: any contract must have at least one transition");
        if let Some(d) = &x.datum {
            Some(marlowe_lang::serialization::json::serialize(d).unwrap_or_default())
        } else {
            None
        }
    }

    /// Script hash of the marlowe payout validator script used for this contract
    async fn payout_validator_hash(&self) -> Option<String> {
        let x = self.transitions.last().expect("marlowe-indexer bug: any contract must have at least one transition");
        if let Some(ei) = &x.datum {
            Some(ei.marlowe_params.0.to_string())
        } else {
            None
        }
    }

    /// The minimum time at which point this contract will be possible to transition to a possible next stage.
    async fn min_time(&self) -> Option<u64> {
        let x = self.transitions.last().expect("marlowe-indexer bug: any contract must have at least one transition");
        if let Some(ei) = &x.datum {
            Some(ei.state.min_time)
        } else {
            None
        }
    }

    /// Current contract definition in DSL format
    async fn contract_dsl(&self) -> Option<String> {
        let x = self.transitions.last().expect("marlowe-indexer bug: any contract must have at least one transition");
        if let Some(ei) = &x.datum {
            Some(ei.contract.to_dsl())
        } else {
            None
        }
    }

    /// Current contract definition in JSON format
    async fn contract_json(&self) -> Option<String> {
        let x = self.transitions.last().expect("marlowe-indexer bug: any contract must have at least one transition");
        if let Some(ei) = &x.datum {
            Some(ei.contract.to_json().expect("marlowe-rs bug: should always be possible to serialize on-chain parties"))
        } else {
            None
        }
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
        if let Some(ei) = &x.datum {
            Some(marlowe_lang::serialization::json::serialize(ei.state.clone()).expect("marlowe-rs bug: should always be possible to serialize on-chain data"))
        } else {
            None
        }
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
    /// BUG: Generic(\"This contract is not fully initialized. There is an invalid case in a When contract.\"
    /// BUG INFO: This happens when there are merkleized cases - bug created in marlowe-rs.
    async fn next_time_out(&self) -> Result<Option<f64>,String> {
        let x = self.transitions.last().expect("marlowe-indexer bug: any contract must have at least one transition");
        if let Some(ei) = &x.datum {
           let mut machine = marlowe_lang::semantics::ContractInstance::from_datum(ei);
           if machine.has_timed_out() {
             return Ok(Some(-1.0))
           }
           for (merkle_hash,continuation) in &x.continuations {
                machine.add_continuation(&merkle_hash, continuation);
           }
           let (a,_b) = machine.process()?;
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
    async fn transitions(&self,filter:Option<TransitionFilter>) -> Vec<&crate::state::MarloweTransition> {
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

                if let Some(issues_filter) = &f.issues { 
                    let has_issues = !x.issues.is_empty();
                    if &has_issues != issues_filter {
                        return false
                    }                    
                }

                if let Some(issue_filter) = &f.issues_match { 
                    let found_match = x.issues.iter().find(|issue|issue.contains(issue_filter));
                    if found_match.is_none() { return false }
                }

                if let Some(marlowe_scan_status_filter) = &f.marlowe_scan_status { 
                    match &x.marlowe_scan_status {
                        None => return false,
                        Some(marlowe_status) => {
                            let found_match = marlowe_status.contains(marlowe_scan_status_filter);
                            if found_match == false { return false }
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



#[derive(Debug, PartialEq)]
pub enum QueryError {
    Generic(String),
    BeforeNotFound,
    AfterNotFound,
    InvalidPagination,
    NoIndexedContracts,
    NoResult
}

impl QueryError {
    pub fn generic(msg:&str) -> Self {
        QueryError::Generic(msg.into())
    }
}

impl std::fmt::Display for QueryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let r = format!("{:?}",self);
        f.write_str(&r)
    }
}



pub(crate) async fn testable_query(contracts: &OrderedContracts, params: QueryParams) -> std::result::Result<Connection<String, Contract, ConnectionFields>, QueryError> {
    
    if contracts.contract_count() == 0 {
        return Err(QueryError::NoIndexedContracts)
    }

    let start_time = tokio::time::Instant::now();
    let mut log: Vec<String> = vec![];

    let resolved_page_size = match params.page_size {
        Some(n) if (5..=100_000).contains(&n) => n as usize,
        None => 50,
        _ => return Err(QueryError::generic("pageSize is allowed to be between 5 and 100"))
    } as i32;

    let def_end = contracts.contract_count();

    let start_pos = match params.after {
        Some(a) => {
            match contracts.lookup_table.get(&a) {
                Some(&i) => i + 1, // we dont want to include the "after" item in the result,
                None => return Err(QueryError::AfterNotFound),
            }
        },
        None => 0,
    };

    let end_pos = match params.before {
        Some(b) => {
            match contracts.lookup_table.get(&b) {
                Some(&i) => i,
                None => return Err(QueryError::BeforeNotFound),
            }
        },
        None => def_end,
    };

    if start_pos > end_pos {
        return Err(QueryError::generic("Invalid filter: 'after' and 'before' are in the wrong order."))
    }

    log.push(format!("Using range: {} to {}", start_pos, end_pos));
    let sliced_contracts = &contracts.contracts_in_order[start_pos..end_pos];
    
    let x_filtered_items: Vec<&Contract> = sliced_contracts.iter().filter(|&x| {
        if let Some(f) = &params.filter {

            if let Some(bounds_filter) = &f.bound_values {
                let datum_to_check = &x.transitions.last().expect("must be at least one transition..").datum;
                if let Some(d) = datum_to_check {
                    let numer_of_bounds = d.state.accounts.len();
                    if !num_check(&bounds_filter, numer_of_bounds as f64) {
                        return false
                    };
                } else {
                    return false
                }                
            }
            
            // might only be interested in a specific version of the validator
            if !str_check_opt(&f.validator_hash,& x.validator_hash ) {
                return false;
            }
            
            if let Some(filter_for_closed) = f.is_closed {
                let last_seen_transition = x.transitions.last().expect("There must always be at least one transition in a contract");
                if filter_for_closed != last_seen_transition.end {
                    // end means there will never exist a new transition in this contract - eg. it is completely closed.
                    return false;
                }
            }

            let issue_filter_matches = 
                if let Some(b) = &f.has_issues {

                    let mut has_issues = false;
                    
                    for t in &x.transitions {
                        if t.invalid {
                            has_issues = true;
                            break
                        }
                    }

                    &has_issues == b
                    
                } else {
                    true
                };

            
            if !issue_filter_matches { return false } // ISSUE_FILTER
            if !str_check_opt(&f.id, &x.id) { return false }; // ID FILTER
            if !str_check_opt(&f.short_id, &x.short_id) { return false }; // SHORT_ID FILTER

            // LOCKED AMOUNTS FILTER (if any account matches the filter, this contract should be included)
            if let Some(fla) = &f.account_state {
                if !locked_amounts_filter_matches(&fla,x) { return false };                
            }    
            
            if let Some(meta_filter) = &f.meta {
                let mut at_least_one_meta_match = false;
                for t in &x.transitions {
                    let is_match = str_check_opt_opt(
                        Some(meta_filter),
                        t.meta.as_deref()
                    );
                    if is_match {
                        at_least_one_meta_match = true;
                        break;
                    } 
                }
                if !at_least_one_meta_match { return false }
            } 
            
            #[cfg(feature="debug")]
            if let Some(dtest) = &f.datum {
                
                // we dont want to compare the full datum everywhere so we convert the datum to hash and compare that instead
                let datum_to_hash = |hex:&str| -> String {
                    let pd = pallas_primitives::alonzo::PlutusData::decode_fragment(&hex::decode(&hex).unwrap()).unwrap();
                    pd.compute_hash().to_string()                    
                }; 
                
                let hash_filter = match &dtest {
                    StringFilter::Eq(v) => StringFilter::Eq(datum_to_hash(v)),
                    StringFilter::Neq(v) => StringFilter::Neq(datum_to_hash(v)),
                    StringFilter::Contains(v) => StringFilter::Contains(datum_to_hash(v)),
                    StringFilter::NotContains(v) => StringFilter::NotContains(datum_to_hash(v)),
                };

                for t in &x.transitions {
                    if let Some(obytes) = &t.original_plutus_datum_bytes {
                        let hex = hex::encode(obytes);
                        if !str_check(&hash_filter,&hex) {
                            return false;
                        }   
                    }
                }
            }
            
            // run last since its the slowest
            #[cfg(feature="debug")]
            if let Some(rstest) = &f.marlowe_rs_status {
                // this MUST be done inside the let check so we dont run marlowe_rs_test when the filter is none
                if !str_check(&rstest,& marlowe_rs_test(x) ) {
                    return false;
                }            
            }

            true
        } else {
            true
        }
    }).collect();

    let total_matching_contracts_before_first_last = x_filtered_items.len() as i32;

    if total_matching_contracts_before_first_last == 0 {
        return Err(QueryError::NoResult)
    }

    log.push(format!("Total contracts after filtering: {}", total_matching_contracts_before_first_last));



    let first_last_items = match (params.first, params.last) {
        (Some(_),Some(_)) => return Err(QueryError::Generic("cannot use both 'first' and 'last' in the same query.".into())),
        (Some(f), _) => {
            if f < 1 {
                return Err(QueryError::InvalidPagination)
            }
            &x_filtered_items[0..f.min(x_filtered_items.len() as i32) as usize]
        },
        (_, Some(l)) => {
            if l < 1 {
                return Err(QueryError::InvalidPagination)
            }
            let s = x_filtered_items.len().saturating_sub(l as usize);
            &x_filtered_items[s..]
        },
        _ => &x_filtered_items[..],
    };

    log.push(format!("Items after first/last params: {}", first_last_items.len()));

    let total_matching_contracts = first_last_items.len() as i32;
    
    // Calculate total pages
    let total_pages = (total_matching_contracts as f32 / resolved_page_size as f32).ceil() as usize;

    // If no page parameter is given, set to the last page, otherwise use the provided page
    let current_page: usize = match params.page {
        Some(p) => p as usize,
        None => total_pages
    };

    // todo : this will be true if there are no items indexed!
    if current_page < 1 || current_page > total_pages {
        return Err(QueryError::InvalidPagination)
    }

    let (page_start, page_end) = {
        let s = (current_page - 1) * resolved_page_size as usize;
        let e = s + resolved_page_size as usize;
        (s, e)
    };

    let start = page_start.min(first_last_items.len());
    let end: usize = page_end.min(first_last_items.len());

    log.push(format!("Selected range after page: {} to {}", start, end));

    let paged_items = &first_last_items[start..end];

    let has_next_page = match (params.first, params.last) {
        (Some(_), _) => end < x_filtered_items.len(),
        (_, Some(_l)) => end < first_last_items.len(),
        _ => end < x_filtered_items.len()
    };
    

    let has_previous_page = match (params.first, params.last) {
        (Some(_), _) => start_pos != 0,
        (_, Some(_)) => start > 0,
        _ => start > 0
    };

    let elapsed_time = start_time.elapsed();
    let time_taken_ms = elapsed_time.as_millis() as f64;

    let mut connection = Connection::with_additional_fields(
        has_previous_page,
        has_next_page,
        ConnectionFields {
            total_indexed_contracts: contracts.contracts_in_order.len(),
            total_number_of_contracts_matching_current_filter: total_matching_contracts as usize,
            total_number_of_pages_using_current_filter: total_pages,
            page_size_used_for_this_result_set: resolved_page_size  as usize,
            total_contracts_in_requested_range: sliced_contracts.len(),
            time_taken_ms,
            current_page: current_page as f64,
            log
        },
    );

    connection.edges = 
        paged_items.iter().map(|node| 
            Edge::new(node.short_id.clone(), (*node).clone())
        ).collect();

    Ok(connection)
}
