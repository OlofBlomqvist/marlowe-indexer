use std::sync::Arc;

use async_graphql::{EmptyMutation, Schema};
use warp::reject::Rejection;

use crate::{modules::marlowe::{graphql::{self, types::MarloweSubscriptionRoot}, state::MarloweState}, state::GlobalState};


#[tracing::instrument(skip_all)]
#[cfg_attr(feature = "GraphQL", derive(async_graphql::SimpleObject))]
pub async fn graphql_handler(
     state: Arc<GlobalState<MarloweState>>,
     bytes: warp::hyper::body::Bytes,
     src_ip:std::net::IpAddr,
     //headers:std::collections::HashMap<String,String>
) -> Result<impl warp::Reply, Rejection> {
    tracing::info!("Got request from {}",src_ip);
    let schema = 
        Schema::build(graphql::types::MarloweQueryRoot, EmptyMutation, MarloweSubscriptionRoot)
            .data(state.clone())
            .finish();    
    let body_string = String::from_utf8_lossy(&bytes);
    let json_value : serde_json::Value = serde_json::from_str(&body_string).unwrap();
    let query_string = json_value["query"].as_str().unwrap_or("");
    let request = async_graphql::Request::new(query_string);
    let response = schema.execute(request).await;
    let response_json = warp::reply::json(&response);

    Ok(warp::reply::with_status(response_json,reqwest::StatusCode::OK))
}