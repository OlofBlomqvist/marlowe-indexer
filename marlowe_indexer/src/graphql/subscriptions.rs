



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
    

    /// Subscribe to events related to any contract or specific contracts using filters.
    #[tracing::instrument(skip_all)]
    async fn contract<'ctx> (&self,ctx: &Context<'ctx>) -> impl Stream<Item = String> {

        // TODO:
        // - Allow subscribing to events such as:
        // - Init/Update/Close
        
        // Filterable using:
        // - contract id / short_id
        // - role tokens
        // - participant addresses

        // TODO: decide on event format

        let _my_context = ctx.data::<std::sync::Arc<tokio::sync::RwLock<crate::state::State>>>().unwrap().clone();
        stream! {
            yield String::from("not yet implemented");
            return;      
        }
    
    }

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

