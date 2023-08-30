use async_graphql::{*, connection::Connection};
use tracing::info;

pub mod transition;
pub mod contract;

#[cfg(feature="debug")]
pub(crate) mod contract_debug;

#[Object]
impl crate::graphql::QueryRoot {


    // TODO: Add some actual statistics.
    #[tracing::instrument(skip_all)]
    async fn stats<'ctx>(
        &self,
        ctx: &Context<'ctx>
    ) -> Result<String> {
        let my_context = ctx.data::<std::sync::Arc<tokio::sync::RwLock<crate::state::State>>>().unwrap();
        let read_guard = my_context.read().await;
        let s = format!("{:?}",read_guard.ordered_contracts.utxo_to_contract_lookup_table);
        
        // known chain tip
        // index tip
        // percentage synced
        // blocks synced per second?
        // estimated time until fully synced?

        Ok(s)
    }

    // todo: filter on sub fields ? ie, not in the main filter
    #[tracing::instrument(skip_all)]
    async fn contracts<'ctx>(
        &self,
        ctx: &Context<'ctx>,
        filter: Option<super::types::ContractsFilter>, // APPLIED AFTER HAVING SLICED THE DATA USING "AFTER" AND "BEFORE"
        pagination: Option<super::types::Pagination>,
    ) -> Result<Connection<String, crate::state::Contract, super::types::ConnectionFields>> {


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
        



        // normally we dont want to serve graphql responses that may be outdated, but in some cases users will want to enable this
        if !read_guard.allow_graphql_without_full_sync {
            let diff = {
                (read_guard.index_tip_abs_slot,read_guard.tip_abs_slot)
            };
        
        
            match diff {
                (Some(index_tip_abs_slot),Some(tip_abs_slot)) => {
                    let slot_diff = tip_abs_slot.abs_diff(index_tip_abs_slot);
                    let max_diff = 10;
                    if slot_diff > max_diff {
                        let err = format!("Index is not ready, come back later. Slot diff between index and chain tip is {slot_diff}. Max allowed: {max_diff}. Index tip: {index_tip_abs_slot}, Chain tip: {tip_abs_slot}. This is done so that we do not provide any outdated information, but you can disable this feature using the '--force-enable-graphql' server flag.");
                        return Err(Error::new(err).extend_with(|_, e| e.set("details", "CAN_NOT_FETCH")))                    
                    }
                },
                _ => {
                    let err = "Index is not ready, come back later.";
                    return Err(Error::new(err).extend_with(|_, e| e.set("details", "CAN_NOT_FETCH")))              
                
                }
            }
        }
        
    



        info!("Handling request to list contracts");
        match super::query::contracts_query_base(contracts,crate::graphql::types::QueryParams { filter, after, before, first, last, page_size, page }).await {
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
