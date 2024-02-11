use async_graphql::{EmptyMutation, Schema};
use self::types::{QueryRoot, SubscriptionRoot};

pub mod types;



#[cfg(feature="debug")]
pub(crate) fn slot_to_posix_time(slot: u64, sc: &types::SlotConfig) -> u64 {
    let ms_after_begin = (slot - sc.zero_slot) * sc.slot_length;
    sc.zero_time + ms_after_begin
}

#[cfg(feature="debug")]
#[allow(dead_code)]
pub(crate) fn posix_time_to_slot(posix_time: u64, sc: &types::SlotConfig) -> u64 {
    ((posix_time - sc.zero_time) / sc.slot_length) + sc.zero_slot
}

pub(crate) mod filters;
pub(crate) mod resolvers;
pub(crate) mod query;
pub(crate) mod subscriptions;

#[cfg(test)]
mod tests;


pub fn create_schema() -> async_graphql::Schema<QueryRoot, async_graphql::EmptyMutation, SubscriptionRoot> {
    Schema::build(QueryRoot, EmptyMutation, SubscriptionRoot).finish()
}
