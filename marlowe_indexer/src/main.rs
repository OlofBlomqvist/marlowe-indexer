
#![feature(addr_parse_ascii)]
use std::{sync::Arc, env::args};
use cardano_chain_sync::{*, configuration::ConfigurationBuilder};
use opentelemetry_api::global::tracer;
use pallas::network::miniprotocols::PREVIEW_MAGIC;
use tracing::{info, info_span, Instrument, warn, warn_span, log::LevelFilter, Span, Metadata, dispatcher::get_default};
use tracing_subscriber::{FmtSubscriber, fmt::format::FmtSpan};
use anyhow::{Result, anyhow};
use warp::{http::Response as HttpResponse, Rejection, hyper::HeaderMap};
use async_graphql_warp::{GraphQLBadRequest};
use async_graphql::{http::GraphiQLSource, EmptyMutation, EmptySubscription, Schema};
use warp::Filter;

use crate::worker::MarloweSyncWorker;
mod worker;
mod state;
mod graphql;


use opentelemetry::trace::Tracer;
use opentelemetry::sdk::trace::Sampler;
use opentelemetry_otlp::Protocol;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry::{KeyValue, sdk::Resource};

macro_rules! any_err {    
    ($expr:expr) => (anyhow::Context::with_context($expr.map_err(|e| anyhow!("{:?}", e)), ||concat!("@ ", file!(), " line ", line!(), " column ", column!())))}


pub async fn init_logging(level:&str,enable_otel_tracing:bool) -> Result<()> {

    env_logger::Builder::from_env(
        env_logger::Env::new().default_filter_or(level)
    ).init();

    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(tracing::log::max_level().as_str()))
        .add_directive("pallas_network=warn".parse().unwrap());

    let console_subscriber = FmtSubscriber::builder()
        .with_env_filter(env_filter)
        .with_line_number(true)
        .with_span_events(FmtSpan::NONE)
        .log_internal_errors(false)
        .finish();

    
        
    // todo : switch to generic otel impl
    if enable_otel_tracing {
       
        let resource = Resource::new(vec![KeyValue::new("service.name", "Marlowe Indexer 11")]);
        let otlp_exporter = opentelemetry_otlp::new_exporter()
            .tonic() // Using tonic for gRPC
            .with_endpoint("https://127.0.0.1:4317/api/traces") // Jaeger collector endpoint
            .with_protocol(Protocol::Grpc) // Assuming Jaeger is using gRPC
            .with_timeout(tokio::time::Duration::from_secs(3)); // Set timeout as needed
        
        let tracer = opentelemetry_otlp::new_pipeline()
            .tracing()
            .with_exporter(otlp_exporter)
            .with_trace_config(
                opentelemetry_sdk::trace::Config::default()
                    .with_sampler(opentelemetry_sdk::trace::Sampler::AlwaysOn)
                    .with_resource(resource)
            )
            .install_batch(opentelemetry::runtime::Tokio)?;
        
        let otel_layer = 
            tracing_opentelemetry::OpenTelemetryLayer::default()
            .with_tracer(tracer)
            .with_tracked_inactivity(true);
        
        let s = 
            tracing_subscriber::prelude::__tracing_subscriber_SubscriberExt::with(console_subscriber, otel_layer);
        
        tracing::subscriber::set_global_default(s)?;



        Ok(())    
    } else {
        
        let s = 
            console_subscriber;//.with(otel_layer);
        
        tracing::subscriber::set_global_default(s)?;

        Ok(())    
    }
}

// custom handler for instrumentation and context management
async fn graphql_handler(
     state: Arc<tokio::sync::RwLock<state::State>>,
     bytes: warp::hyper::body::Bytes,
     src_ip:std::net::IpAddr,
     headers:std::collections::HashMap<String,String>
) -> Result<impl warp::Reply, Rejection> {
    
   
    let span = info_span!("graphql-request","request-headers" = tracing::field::Empty);    
    let _guard = span.enter();
    
    let log_level = tracing::log::max_level();
    if log_level == LevelFilter::Trace {
        span.record("request-headers", serde_json::to_string(&headers).unwrap());
    }
    
    info!("Got request from {}",src_ip);
    
    let schema = 
        Schema::build(graphql::QueryRoot, EmptyMutation, EmptySubscription)
            .data(state.clone())
            .data(graphql::create_schema())
            .finish();    
    let body_string = String::from_utf8_lossy(&bytes);
    let json_value : serde_json::Value = serde_json::from_str(&body_string).unwrap();
    let query_string = json_value["query"].as_str().unwrap_or("");
    let request = async_graphql::Request::new(query_string);
    let response = schema.execute(request).in_current_span().await;
    Ok(warp::reply::json(&response))
}


use std::net::{SocketAddr, ToSocketAddrs};

#[tokio::main]
async fn main() -> Result<()> {
    
    // TODO: use clap for args + parse config file

    let args = args().collect::<Vec<String>>();
    
    let address = 
        if let Some(a) = &args.get(1) {
            let ip_port = a.to_socket_addrs().unwrap().next().expect("please specify node addr:port");
            let ip = ip_port.ip();
            let port = ip_port.port();
            format!("{ip}:{port}")
        } else {
            panic!("provide ip:port of cardano node")
        };
 

    _ = init_logging("info",false).await?;

    info!("Application starting.");
    
    let configuration = 
        match args.get(2) {
            Some(x) if x == "preprod" => {
                any_err!(ConfigurationBuilder::new()
                    .with_magic(PRE_PRODUCTION_MAGIC)
                    .with_address(address.into())
                    .build())?
            },
            Some(x) if x == "mainnet" => {
                any_err!(ConfigurationBuilder::new()
                    .with_magic(MAINNET_MAGIC)
                    .with_address(address.into())
                    .build())?
            },
            Some(x) if x == "preview" => {
                any_err!(ConfigurationBuilder::new()
                    .with_magic(PREVIEW_MAGIC)
                    .with_address(address.into())
                    .build())?
            },            
            _ => panic!("please also specify network: preprod, preview or mainnet")
        };
        
    
    let shared_state = state::State::new();
    let locked_share = tokio::sync::RwLock::new(shared_state);
    let shared_state_arc = Arc::new(locked_share);
    
    let ccc = shared_state_arc.clone();

    tokio::spawn(async move {
        let span = tracing::info_span!("marlowe_worker");
        let _enter = span.enter();
        let marlowe_worker = Box::new(MarloweSyncWorker::new(ccc));
        let mut ccs = 
            CardanoChainSync::new(
                configuration,
                marlowe_worker
            );
        ccs.run().await
    });


    let cors = warp::cors()
        .allow_any_origin() 
        .allow_methods(vec!["POST", "GET", "OPTIONS"]) 
        .allow_headers(vec!["Content-Type","origin"]); 
    
    let graphql_post = warp::post()
        .and(warp::body::bytes())
        .and(warp::addr::remote())
        .and(warp::header::headers_cloned())
        .and_then(move |bytes,remote_addr: Option<std::net::SocketAddr>, headers:HeaderMap| {
            let shared_state_arc = shared_state_arc.clone();
            let source_ip = remote_addr.unwrap().ip();
            let headers_map: std::collections::HashMap<String,String>  = headers
                .iter()
                .map(|header| (header.0.as_str().to_owned(), header.1.to_str().unwrap().to_owned()))
                .collect();
            async move {
                
                let span = warn_span!("warp-request");
                graphql_handler(
                    shared_state_arc, 
                    bytes,
                    source_ip,
                    headers_map
                ).instrument(span).await
            }
        }).with(cors.clone());


    let graphiql = warp::path::end().and(warp::get()).map(|| {
        HttpResponse::builder()
            .header("content-type", "text/html")
            .body(GraphiQLSource::build().endpoint("/").finish())
    }).with(cors.clone());
    
 
    let routes = 
        graphiql
        .or(graphql_post)
        .recover(|err: Rejection| async move {
            if let Some(GraphQLBadRequest(err)) = err.find() {
                return Ok::<_, std::convert::Infallible>(warp::reply::with_status(
                    err.to_string(),
                    warp::hyper::StatusCode::BAD_REQUEST,
                ));
            }
            warn!("INVALID REQUEST: {:?}",err);
            Ok(warp::reply::with_status(
                "INTERNAL_SERVER_ERROR".to_string(),
                warp::hyper::StatusCode::INTERNAL_SERVER_ERROR,
            ))
        });

    warp::serve(routes).run(([0, 0, 0, 0], 8000)).await;
    //=======================================================================

    info!("Application stopping.");

    opentelemetry::global::shutdown_tracer_provider(); // export remaining spans

    Ok(())
}

