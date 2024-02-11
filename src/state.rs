use std::sync::Arc;

use crate::core::lib::ChainSyncState;

// TODO - implement this
pub struct SubscriptionEvent {
    _module_id : String,
    _message : serde_json::Value
}

pub struct GlobalState<T> {
    pub sub_state : T,
    pub sync_state: Arc<ChainSyncState>,
     
    pub subscribers: Arc<tokio::sync::RwLock<Vec<crossbeam::channel::Sender<SubscriptionEvent>>>>
}

impl<T> GlobalState<T> {
    pub fn new(sub_state:T,sync_state: Arc<ChainSyncState>) -> Self {
        Self { 
            sub_state, 
            sync_state,
            subscribers: Arc::new(tokio::sync::RwLock::new(vec![]))
        }
        
    }
}

