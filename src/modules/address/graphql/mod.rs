use async_graphql::Context;
use futures::Stream;
use crate::modules::address::stream;
use super::AddressState;

pub mod subscription;
pub mod query;