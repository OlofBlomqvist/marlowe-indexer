use std::sync::Arc;
use async_graphql::*;

use tracing::{info_span, info, debug, warn, trace};

pub struct QueryRoot;


//#[derive(GraphQLObject)]
#[derive(SimpleObject)]
pub struct CONTRACT {
    /// + slot id of the block in which this contract was initialized 
    /// + index of the tx in that block 
    /// + index of the output containing the contract inside the tx.
    /// ... converted to base58.
    pub short_id: String, 
    
    /// hash id of the tx that initially created this contract instance
    pub id:String,

    // todo: current contract state avail here as well

    /// all utxos that has caused state transitions in this contract
    pub transitions: Vec<GMarloweUtxo>
}

//#[derive(GraphQLObject)]
#[derive(SimpleObject)]
pub struct GMarloweUtxo {
    pub datum : String, // todo - more useful info here..
    pub redeemer : String, // todo - more useful info here..
    pub tx_id : String, // index of the tx which created this utxo
    pub utxo_index : f64, // index of this utxo in its parent tx
    pub end : bool, // does this utxo mark the end of a contract
    pub slot : f64,    
    pub block : String,
    pub tx_index : f64,
    pub block_num : f64
}


//pub type MyContext = Arc<tokio::sync::RwLock<crate::state::State>>;


#[derive(InputObject)]
pub struct MyFilter {
    pub id : Option<StringFilter>,
    pub short_id : Option<StringFilter>
}


#[derive(OneofObject,Debug)]
pub enum StringFilter {
    Eq(String),
    Neq(String),
    Contains(String),
    NotContains(String)
}

#[derive(OneofObject)]
pub enum NumFilter {
    Eq(f64),
    Gt(f64),
    Lt(f64),
    Lte(f64),
    Gte(f64)
}

//#[graphql_object(context=Arc<RwLock<crate::state::State>>)]
#[Object]
impl QueryRoot {
   
    async fn contracts<'ctx>(
        &self,
        //ctx: &Arc<RwLock<crate::state::State>>,
        ctx: &Context<'ctx>,
        filter: Option<MyFilter>,
    ) -> Vec<CONTRACT> {
        
        let my_context = ctx.data::<Arc<tokio::sync::RwLock<crate::state::State>>>().unwrap();

        let span = info_span!("contracts query",contract_count = tracing::field::Empty);        
        let _guard = span.enter();

        info!("Handling request to list contracts");
        
        // if filter.is_some() {
        //     tracing::info!("filtering");
        // } else {
        //     tracing::info!("unfiltered query");
        // }

        // let fields = ctx.field().selection_set();
        // for f in fields {
        //     let tn = f.name();
        //     let cn = f.selection_set();
        //     for x in cn {
        //          recurse..
        //     }
        // }

        let mut v = vec![];
        let read_guard = my_context.read().await;
        let contracts = read_guard.contracts();
        span.record("contract_count", contracts.len());

        for x in contracts.iter() {
            let initial_utxo = x.1.iter().find(|tx|tx.1.id == *x.0).unwrap();
            let bid = format!("{}{}{}",initial_utxo.1.block_num ,initial_utxo.1.tx_index, initial_utxo.1.utxo_index);
            let short_id = bs58::encode(bid).into_string();
            if filter.is_none() {
                
                v.push(CONTRACT {
                    id : x.0.to_owned(),                    
                    short_id: short_id,
                    transitions: {
                        let mut transition_vec: Vec<GMarloweUtxo> = x.1.iter().map(|(_k,v)|{
                            // todo - ignore non-selected ?
                            GMarloweUtxo {
                                block_num : v.block_num as f64,
                                tx_index: v.tx_index as f64,
                                block: v.block_hash.to_string(),
                                datum: v.datum.clone(),
                                redeemer: v.redeemer.clone(),
                                tx_id: v.id.clone(),
                                utxo_index: v.utxo_index as f64,
                                slot: v.slot as f64,
                                end: v.end
                            }
                        }).collect();
                        
                        transition_vec.sort_by(|a, b| a.block_num.partial_cmp(&b.block_num).unwrap_or(std::cmp::Ordering::Equal));
                    
                        transition_vec
                    }
                })
            
            } else if let Some(filter) = &filter {

                if !match &filter.id {
                    Some(StringFilter::Eq(id)) => x.0 == id,
                    Some(StringFilter::Neq(id)) => x.0 != id,
                    Some(StringFilter::Contains(id)) => x.0.contains(id),
                    Some(StringFilter::NotContains(id)) => !x.0.contains(id),
                    _ => true
                } {
                    trace!("id filter {:?} did not match id: {}",&filter.id, x.0);
                    continue 
                };

                if !match &filter.short_id {
                    Some(StringFilter::Eq(id)) => &short_id == id,
                    Some(StringFilter::Neq(id)) => &short_id != id,
                    Some(StringFilter::Contains(id)) => short_id.contains(id),
                    Some(StringFilter::NotContains(id)) => !short_id.contains(id),
                    _ => true
                } {
                    trace!("short_id filter {:?} did not match id: {}",&filter.short_id, short_id);
                    continue 
                }

                let bid = format!("{}{}{}",initial_utxo.1.block_num ,initial_utxo.1.tx_index, initial_utxo.1.utxo_index);
                let short_id = bs58::encode(bid).into_string();
                v.push(CONTRACT {
                    short_id: short_id,
                    id : x.0.to_owned(),
                    transitions: x.1.iter().map(|(_k,v)|{
                        GMarloweUtxo {
                            block_num : v.block_num as f64,
                            tx_index: v.tx_index as f64,
                            block: v.block_hash.to_string(),
                            datum: v.datum.clone(),
                            redeemer: v.redeemer.clone(),
                            tx_id: v.id.clone(),
                            utxo_index: v.utxo_index as f64,
                            slot: v.slot as f64,
                            end: v.end
                        }
                    }).collect()
                });
                
            }
        }
        
        v
    }

    
    async fn current_block<'ctx>(&self,ctx: &Context<'ctx>) -> Option<String> {
        let my_context = ctx.data::<Arc<tokio::sync::RwLock<crate::state::State>>>().unwrap();
        let span = info_span!("current_slot query");        
        let _guard = span.enter();
        
        if let Some(block_id) = my_context.read().await.last_seen_block() {
            Some(block_id.clone().to_string())
        } else {
            None
        }
    }
}

pub fn create_schema() -> async_graphql::Schema<QueryRoot, async_graphql::EmptyMutation, async_graphql::EmptySubscription> {
    Schema::build(QueryRoot, EmptyMutation, EmptySubscription).finish()
}



// ====================== NOTES =======================================================================



// ---------------------------------------------------
// :: GET SELECTED FIELDS OF QUERY
// :: So that we can create a more efficient db call?
// ---------------------------------------------------
//  
//  let fields = ctx.field().selection_set();
//  for f in fields {
//      // top level field F name:
//      _ = f.name();
//      // sub fields selected from F:
//      _ = f.selection_set() // <-- recurse
//  }
