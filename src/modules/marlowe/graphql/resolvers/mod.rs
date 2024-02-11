use async_graphql::{*, connection::Connection};
use tracing::debug;

use crate::{modules::{marlowe::state::{MarloweState, Contract}, ModulesStates}, state::GlobalState};
use super::types::MarloweQueryRoot;

pub mod transition;
pub mod contract;

#[cfg(feature="debug")]
pub(crate) mod contract_debug;

#[Object]
impl MarloweQueryRoot {

    #[tracing::instrument(skip_all)]
    async fn contracts<'ctx>(
        &self,
        ctx: &Context<'ctx>,
        filter: Option<super::types::ContractsFilter>, // APPLIED AFTER HAVING SLICED THE DATA USING "AFTER" AND "BEFORE"
        pagination: Option<super::types::Pagination>,
    ) -> Result<Connection<String, Contract, super::types::ConnectionFields>> {


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

        //let my_context = ctx.data::<std::sync::Arc<MarloweState>>().unwrap();
        let my_context = ctx.data::<std::sync::Arc<GlobalState<ModulesStates>>>().unwrap();
        let contracts = my_context.sub_state.marlowe_state.clone();
        
        let magic = my_context.sync_state.configuration.magic;

        debug!("Handling request to list contracts");
        match super::query::contracts_query_base(magic,&contracts,crate::modules::marlowe::graphql::types::QueryParams { filter, after, before, first, last, page_size, page }).await {
            Ok(r) => Ok(r),
            Err(e) => Err(e.into()),
        }
        
    }
    
    
    #[tracing::instrument(skip_all)]
    async fn current_block<'ctx>(&self,ctx: &Context<'ctx>) -> Option<String> {
        let my_context = ctx.data::<std::sync::Arc<MarloweState>>().unwrap();
        my_context.last_seen_slot_num().as_ref().map(|block_id| block_id.clone().to_string())
    }
    

}
