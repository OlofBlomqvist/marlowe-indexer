// not sure i even want to have rest endpoints yet - this is just here for experimentation
 
use std::{collections::HashMap, sync::Arc};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use warp::{filters::BoxedFilter, reject::Rejection, reply::Reply, Filter};
use crate::{modules::ModulesStates, state::GlobalState};

use utoipa::{ToSchema, OpenApi};

#[derive(ToSchema)]
struct Contract { }


#[derive(OpenApi)]
#[openapi(
    paths(
        get_contract_handler,
        echo_handler
    ),
    components(schemas(SearchQuery, Contract)),
    tags(
        (name = "Marlowe", description = "Operations with Marlowe contracts")
    )
)]
struct ApiDoc;

#[derive(Serialize, Deserialize)]
struct JsonResponse {
    message: String,
}

#[derive(Deserialize, ToSchema)]
struct SearchQuery {
    search: String,
    page: Option<u32>,
}

#[derive(Debug)]
struct CustomError {
    message: String,
}

impl warp::reject::Reject for CustomError {}

// Custom error type for not found contracts
#[derive(Debug)]
struct ContractNotFoundError;

impl warp::reject::Reject for ContractNotFoundError {}

async fn handle_rejection(err: warp::reject::Rejection) -> Result<impl Reply, std::convert::Infallible> {
    if let Some(custom_error) = err.find::<CustomError>() {
        let json = warp::reply::json(&HashMap::from([("error", custom_error.message.clone())]));
        Ok(warp::reply::with_status(json, StatusCode::BAD_REQUEST))
    } else {
        // Fallback error response
        let json = warp::reply::json(&HashMap::from([("error", format!("{err:?}"))]));
        Ok(warp::reply::with_status(json, StatusCode::INTERNAL_SERVER_ERROR))
    }
}

pub fn routes(state: Arc<GlobalState<ModulesStates>>) -> BoxedFilter<(impl Reply,)> {
    let state_for_hello = state.clone();

    let hello = warp::path("hello")
        .and(warp::get())
        .and(warp::query::<SearchQuery>())
        .and_then(move |query: SearchQuery| {
            let state = state_for_hello.clone();
            async move {
                
                let data = state.sub_state.marlowe_state
                    .get_by_shortid_from_mem_cache(query.search).await
                    .ok_or_else(|| warp::reject::custom(ContractNotFoundError))?;

                let last_tx = data.transitions.last()
                    .ok_or_else(|| warp::reject::custom(CustomError { message: "No transactions found".to_string() }))?;
                
                let c = last_tx.datum.as_ref()
                    .ok_or_else(|| warp::reject::custom(CustomError { message: "No datum found".to_string() }))?;
                
                let jj = serde_json::to_value(&c)
                    .map_err(|_| warp::reject::custom(CustomError { message: "Serialization error".to_string() }))?;
                
                Ok::<_, warp::Rejection>(warp::reply::json(&jj))
            }
        });

    let echo = warp::path("echo")
        .and(warp::path::param())
        .and_then(echo_handler);

    let routes = hello.or(echo)
        ;//.recover(handle_rejection);


    let api_doc_route = warp::path("api-doc.json")
        .and(warp::get())
        .map(|| warp::reply::json(&ApiDoc::openapi()));

    let config = Arc::new(utoipa_swagger_ui::Config::new(["/api-doc.json","/api-doc1.json", "/api-doc2.json"]));
    let swagger_ui = warp::path("swagger-ui")
        .and(warp::get())
        .and(warp::path::full())
        .and(warp::path::tail())
        .and(warp::any().map(move || config.clone()))
        .and_then(serve_swagger);

    api_doc_route.or(swagger_ui).or(routes).boxed()

}

async fn serve_swagger(
    full_path: warp::filters::path::FullPath,
    tail: warp::filters::path::Tail,
    config: Arc<utoipa_swagger_ui::Config<'static>>,
) -> Result<Box<dyn Reply + 'static>, Rejection> {
    if full_path.as_str() == "/swagger-ui" {
        return Ok(Box::new(warp::redirect::found(warp::hyper::http::Uri::from_static(
            "/swagger-ui/",
        ))));
    }

    let path = tail.as_str();
    match utoipa_swagger_ui::serve(path, config) {
        Ok(file) => {
            if let Some(file) = file {
                let mut r = warp::reply::Response::new(file.bytes.into());
                r.headers_mut().append("Content-Type", file.content_type.parse().unwrap());
                Ok(Box::new(r))
            } else {
                Ok(Box::new(StatusCode::NOT_FOUND))
            }
        }
        Err(error) => {
            let body : String = error.to_string();
            let mut r = warp::reply::Response::new(body.into());
            *r.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
            Ok(Box::new(r))
        },
    }
}



// Annotated hello handler for OpenAPI
#[utoipa::path(
    get,
    path = "/echo",
    responses(
        (status = 200, description = "yay"),
        (status = 404, description = "boo")
    ),
    params(
        ("param" = String, Query, description = "hmmm")
    )
)]

async fn echo_handler(param: String) -> Result<impl Reply, warp::reject::Rejection> {
   Ok(format!("Echo: {}", param))
}

// Annotated hello handler for OpenAPI
#[utoipa::path(
    get,
    path = "/get_contract",
    responses(
        (status = 200, description = "Contract data fetched successfully", body = Contract),
        (status = 404, description = "Contract not found")
    ),
    params(
        ("search" = String, Query, description = "Search query parameter"),
        ("page" = Option<u32>, Query, description = "Optional page number")
    )
)]

async fn get_contract_handler(query: SearchQuery, state: Arc<GlobalState<ModulesStates>>) -> Result<impl Reply, warp::reject::Rejection> {
    let data = state.sub_state.marlowe_state
        .get_by_shortid_from_mem_cache(query.search).await
        .ok_or_else(|| warp::reject::custom(CustomError { message: "BOO THAT CONTRACT WAS NOT FOUND".to_string() }))?;

    let last_tx = data.transitions.last()
        .ok_or_else(|| warp::reject::custom(CustomError { message: "No transactions found".to_string() }))?;

    let c = last_tx.datum.as_ref()
        .ok_or_else(|| warp::reject::custom(CustomError { message: "No datum found".to_string() }))?;

    let jj = serde_json::to_value(&c)
        .map_err(|_| warp::reject::custom(CustomError { message: "Serialization error".to_string() }))?;

    Ok::<_, warp::Rejection>(warp::reply::json(&jj))
}