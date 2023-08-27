use std::collections::VecDeque;

use crate::graphql::types::*;
use async_graphql::InputObject;
use async_graphql::SimpleObject;

use tokio::time::Duration;
use async_graphql::async_stream::stream;
use async_graphql::{
    Subscription,
    futures_util::Stream, 
    Context
};


#[derive(SimpleObject)]
pub struct PerfStats {
    pub indexed_contracts : usize
}

#[Subscription]
impl crate::graphql::types::SubscriptionRoot {
    
    // TODO : Read stats from statistics context
    async fn indexer_perf_stats<'ctx>(&self, ctx: &Context<'ctx>) -> impl Stream<Item = PerfStats> { 
        let my_context = ctx.data::<std::sync::Arc<tokio::sync::RwLock<crate::state::State>>>().unwrap().clone();
        stream! {
            loop {
                {   
                    let read_guard = my_context.read().await;
                    let current_count = read_guard.ordered_contracts.contract_count();
                    
                    yield PerfStats { indexed_contracts: current_count }
                }
                futures_timer::Delay::new(Duration::from_secs(5)).await;
            }
        }
    }

}

