use async_graphql::Object;
use marlowe_lang::types::marlowe::AccMap;

use crate::modules::marlowe::{state::MarloweTransition, graphql::types::LockedAmountsResult};


#[Object]
impl MarloweTransition {

    async fn meta(&self) -> &Option<String> { &self.meta }
    
    //async fn validity_range(&self) -> String { format!("{:#?}",&self.validity_range) }
    

    async fn datum_json(&self) -> Option<String> { self.datum.as_ref().map(|d| marlowe_lang::serialization::json::serialize(d).unwrap_or_default())}
    
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
    async fn utxo_index(&self) -> f64 { self.utxo_index.f64() }
    async fn end(&self) -> &bool { &self.end }
    async fn slot(&self) -> f64 { self.abs_slot.f64() }
    async fn block_hash(&self) -> &str { &self.block }
    async fn block_num(&self) -> f64 { self.block_num.f64() }
    async fn marlowe_scan_status(&self) -> &Option<String> { 
         &self.marlowe_scan_status
    }

    
    async fn locked_funds(&self) -> Option<Vec<LockedAmountsResult>> {
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