use std::collections::HashMap;

use async_graphql::SimpleObject;
use serde::{Deserialize, Serialize};

use super::*;



#[derive(SimpleObject)]
pub struct AddressInfo {
    pub address : String,
    pub lovelace : String,
    pub other_assets : HashMap<String,String>,
    pub utxos : Vec<UtxoInfo>
}


#[derive(SimpleObject,Clone,Serialize,Deserialize)]
pub struct UtxoInfo {
    pub hash : String,
    pub index : u64,
    pub lovelace : String,
    pub address : String,
    pub other_assets : HashMap<String,String>,
    pub block_num : u64,
    pub block_abs_slot: u64
}

#[derive(Default)]
pub struct AddressQueryType;

#[allow(unreachable_code)]
#[async_graphql::Object]
impl AddressQueryType {

    // Simple implementation that does otf materialization of statebased on the utxos available in each address
    async fn addresses<'ctx>(&self,ctx: &Context<'ctx>) -> Vec<AddressInfo> {
        
        let state = ctx.data::<std::sync::Arc<AddressState>>().unwrap().clone();
        
        let mut results = vec![];

        for item in state.addresses.iter() {
            if let Ok((key, serialized_utxos)) = item {
                let addr: String = key.as_ascii().unwrap().as_str().to_string();
                let utxos: Vec<UtxoInfo> = serde_json::from_slice(&serialized_utxos).expect("Deserialization of utxos failed");
                    
                let mut assets_info = HashMap::<String,String>::new();

                let mut lovelace_sum = 0;
                for utxo in utxos.iter() {
                    lovelace_sum += utxo.lovelace.parse::<u128>().unwrap();
                    for (k,v) in &utxo.other_assets {
                        if assets_info.contains_key(k) {
                            let mutable_existing_asset_value = assets_info.get_mut(k).unwrap();
                            *mutable_existing_asset_value = 
                                (mutable_existing_asset_value.parse::<u128>().unwrap() + (v.parse::<u128>().unwrap())).to_string()
                        } else {
                            assets_info.insert(k.clone(),v.clone());
                        }
                    }   
                }

                results.push(AddressInfo { 
                    address: addr.to_owned(), 
                    lovelace: lovelace_sum.to_string(),
                    other_assets: assets_info,
                    utxos: utxos.to_vec()
                })
            }
        }

        results

        // let adrguard = state.addresses.read().await;

        // let mut results = vec![];

        // for (addr,info) in adrguard.iter() {

        //     let mut assets_info = HashMap::<String,String>::new();

        //     let infoguard = info.read().await;
        //     let mut lovelace_sum = 0;
        //     for utxo in infoguard.iter() {
        //         lovelace_sum += utxo.lovelace.parse::<u128>().unwrap();
        //         for (k,v) in &utxo.other_assets {
        //             if assets_info.contains_key(k) {
        //                 let mutable_existing_asset_value = assets_info.get_mut(k).unwrap();
        //                 *mutable_existing_asset_value = 
        //                     (mutable_existing_asset_value.parse::<u128>().unwrap() + (v.parse::<u128>().unwrap())).to_string()
        //             } else {
        //                 assets_info.insert(k.clone(),v.clone());
        //             }
        //         }   
        //     }

        //     results.push(AddressInfo { 
        //         address: addr.to_owned(), 
        //         lovelace: lovelace_sum.to_string(),
        //         other_assets: assets_info,
        //         utxos: info.read().await.to_vec()
        //     })
        // }

        // return results;
    }
}

