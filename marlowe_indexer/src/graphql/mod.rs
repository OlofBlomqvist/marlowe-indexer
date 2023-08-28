use async_graphql::{EmptyMutation, Schema};
use self::types::{QueryRoot, SubscriptionRoot};

pub mod types;
pub mod query;
pub mod subscriptions;

#[cfg(test)]
mod tests;

pub fn create_schema() -> async_graphql::Schema<QueryRoot, async_graphql::EmptyMutation, SubscriptionRoot> {
    Schema::build(QueryRoot, EmptyMutation, SubscriptionRoot).finish()
}
