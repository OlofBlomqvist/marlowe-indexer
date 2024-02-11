use async_graphql::{SimpleObject, InputObject};
// use async_graphql::{SimpleObject, OneofObject, InputObject, Enum};
use tokio::time::Duration;
use async_graphql::async_stream::stream;
use async_graphql::{
    Subscription,
    futures_util::Stream, 
    Context
};

use crate::modules::marlowe::MarloweSubscriptionEvent;
use crate::modules::marlowe::state::{MarloweState, Contract, ContractShortId};
use crate::modules::marlowe::graphql::types::MarloweSubscriptionRoot;

#[derive(SimpleObject)]
pub struct PerfStats {
    pub indexed_contracts : usize
}

// TODO: Make this more useful by adding event types and filters.

// #[derive(OneofObject)]
// pub enum EventFilterType {
//     ContractInitialized(EventFilter),
//     ContractUpdated(EventFilter),
//     ContractClosed(Subthing),
// }



// #[derive(OneofObject,Clone)]
// pub enum Subthing2 {
//     H(u64),
//     I(i64),
//     J(String)
// }

// #[derive(OneofObject,Clone)]
// pub enum Subthing {
//     D(u64),
//     E(i64),
//     F(String),
//     G(Subthing2)
// }

// #[derive(InputObject)]
// pub struct EventFilter {
//     pub A : i64,
//     pub B : Thing,
//     pub C: Subthing
// }



// TODO -> Add support for filtering by expected inputs
//         so that we can allow oracles to listen for interesting contracts?

#[derive(SimpleObject,InputObject)]
pub struct MarloweSubscriptionFilter {
    pub closings : bool,
    pub inits : bool,
    pub updates : bool
}


// TODO: make this not stupid
fn check_filter(filter:&MarloweSubscriptionFilter, event:&MarloweSubscriptionEvent) -> bool {
   
    match event.evt_type {
        crate::modules::marlowe::MarloweSubscriptionEventType::Init => {
            if !filter.inits { return false }
        },
        crate::modules::marlowe::MarloweSubscriptionEventType::Update => {
            if !filter.updates { return false }
        },
        crate::modules::marlowe::MarloweSubscriptionEventType::Closed => {
            if !filter.closings { return false }
        },
    }

    true
}

#[Subscription]
impl MarloweSubscriptionRoot {

    /// NOTE: POC.
    #[tracing::instrument(skip_all)]
    async fn event<'ctx> (&self,ctx: &Context<'ctx>, filter: MarloweSubscriptionFilter) -> impl Stream<Item = MarloweSubscriptionEvent> {

        let my_context = ctx.data::<std::sync::Arc<MarloweState>>().unwrap().clone();
        let mut xx  = my_context.broadcast_tx.0.subscribe();
        
        stream! {
            while let Ok(msg) = xx.recv().await {
                if check_filter(&filter,&msg) {
                    yield msg
                }
            }   
        }
    }

    /// Wait for updates to a contract. Subscription will end when contract does.
    #[tracing::instrument(skip_all)]
    async fn contract<'ctx> (&self,ctx: &Context<'ctx>, short_id: ContractShortId) -> impl Stream<Item = Option<Contract>> {

        let my_context = ctx.data::<std::sync::Arc<MarloweState>>().unwrap().clone();
        
        stream! {
            
            loop {
                
                let possibly_contract = my_context.get_by_shortid(short_id.clone());

                if let Some(c) = possibly_contract {        
                    yield Some(c.clone());
                    // no need to continue waiting for things that will never change
                    if c.transitions.last().unwrap().end {
                        return
                    }
                } else {
                    yield None
                }
                
                futures_timer::Delay::new(Duration::from_secs(2)).await;
            } 
        }
    
    }


}

