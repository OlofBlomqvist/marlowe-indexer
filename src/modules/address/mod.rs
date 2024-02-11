/*

   This just serves as a basic example of how to implement a simple module

*/

pub mod state;
pub mod graphql;
mod worker;

use async_graphql::async_stream::stream;
use tracing::info;

use self::state::AddressState;

pub struct AddressModule {
    pub state : std::sync::Arc<AddressState>
}

impl AddressModule {
    pub fn new(state : std::sync::Arc<AddressState>) -> Self {
        info!("Example Sync Worker Initialized");
        AddressModule { 
            state
        }
    }
}
