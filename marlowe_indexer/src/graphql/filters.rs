use marlowe_lang::types::marlowe::AccMap;

use crate::state::Contract;

use super::types::*;

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


/// NOTE: Not yet implemented
#[allow(dead_code)]
pub fn uses_merkleization(_contract:&Contract) -> bool {
    false
}

#[cfg(feature="debug")]
pub fn datum(filter:&Option<StringFilter>, contract: &Contract) -> bool {
    
    // if no filter is given, everything will match
    let filter = if let Some(f) = filter { f } else {return true };

    // we dont want to compare the full datum everywhere so we convert the datum to hash and compare that instead
    let datum_to_hash = |hex:&str| -> String {
        let pd = <pallas_primitives::alonzo::PlutusData as pallas_primitives::Fragment>::decode_fragment(&hex::decode(&hex).unwrap()).unwrap();
        pallas_traverse::ComputeHash::compute_hash(&pd).to_string()                    
    }; 
    
    let hash_filter = match &filter {
        StringFilter::Eq(v) => StringFilter::Eq(datum_to_hash(v)),
        StringFilter::Neq(v) => StringFilter::Neq(datum_to_hash(v)),
        StringFilter::Contains(v) => StringFilter::Contains(datum_to_hash(v)),
        StringFilter::NotContains(v) => StringFilter::NotContains(datum_to_hash(v)),
    };

    // if we find any datum that matches the filter, return true.
    for t in &contract.transitions {
        if let Some(original_datum_hash) = &t.datum_hash {
            if str_check(&hash_filter,&original_datum_hash) {
                return true;
            }   
        }
    }
    
    
    false // filter was given but no match found

}


/// Returns true if the current state has any bounds values
pub fn bound_values(filter:&Option<NumFilter>,contract:&Contract) -> bool {
    if let Some(bounds_filter) = filter {
        let datum_to_check = &contract.transitions.last().expect("must be at least one transition..").datum;
        if let Some(d) = datum_to_check {
            let numer_of_bounds = d.state.bound_values.len();
            return num_check(bounds_filter, numer_of_bounds as f64)
        } else {
            return false // no match
        }                
    }
    true // no filter
}

pub fn meta(filter:&Option<StringFilter>,contract:&Contract) -> bool {
    
    if let Some(meta_filter) = filter {
        for t in &contract.transitions {
            if str_check_opt_opt(
                Some(meta_filter),
                t.meta.as_deref()
            ) {
                return true // at least one match
            } 
        }
        false // no match
    } else {
        true // no filter
    }
}

pub fn locked_funds(filter:&LockedAmountFilter,contract:&Contract) -> bool {

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

    // TODO --> we dont seem to handle search by role/addr etc.
    // Also, we want to do early return rather than running all checks 
    
    
    for x in result  {


        // Filter by amounts
        let amounts_match = num_check_opt(&filter.amount, x.amount);
        
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
                            str_check(role_filter,aor)
                        } else {
                            false
                        }
                    },
                    AccountOwnerFilter::Address(addr_filter) => {
                        if let Some(aoa) = &x.account_owner_addr {
                            str_check(addr_filter,aoa)
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

    false

}






