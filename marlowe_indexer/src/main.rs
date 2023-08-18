
#![feature(addr_parse_ascii)]
#![feature(async_closure)]

use std::sync::Arc;
use cardano_chain_sync::{*, configuration::ConfigurationBuilder};
use clap::Parser;
use tracing::debug;
use tracing::{info, info_span, Instrument, warn, warn_span, log::LevelFilter};
use tracing_subscriber::{FmtSubscriber, fmt::format::FmtSpan};
use anyhow::{Result, anyhow};
use warp::{http::Response as HttpResponse, Rejection, hyper::HeaderMap};
use crate::worker::MarloweSyncWorker;
use warp::Filter;
mod worker;
mod state;

use cardano_chain_sync::pallas_network_ccs as pallas_network;

use async_graphql_warp::GraphQLBadRequest;
use async_graphql::{http::GraphiQLSource, EmptyMutation, EmptySubscription, Schema};
mod graphql;
mod graphql_tests;

use opentelemetry_otlp::Protocol;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry::{KeyValue, sdk::Resource};

macro_rules! any_err {    
    ($expr:expr) => (anyhow::Context::with_context($expr.map_err(|e| anyhow!("{:?}", e)), ||concat!("@ ", file!(), " line ", line!(), " column ", column!())))}


    pub async fn init_logging(level:&str,otel_tracing_endpoint:Option<String>) -> Result<()> {

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
            .pretty()     
            .finish();
    
        if let Some(otel_addr) = otel_tracing_endpoint {
        
            let resource = Resource::new(vec![KeyValue::new("service.name", "Marlowe Indexer 11")]);
            let otlp_exporter = opentelemetry_otlp::new_exporter()
                .tonic() 
                .with_endpoint(otel_addr)
                .with_protocol(Protocol::Grpc) 
                .with_timeout(tokio::time::Duration::from_secs(3));
            
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
            
            tracing::subscriber::set_global_default(console_subscriber)?;

            Ok(())    
        }
    }



#[tracing::instrument(skip_all)]
#[cfg_attr(feature = "GraphQL", derive(async_graphql::SimpleObject))]
async fn graphql_handler(
     state: Arc<tokio::sync::RwLock<state::State>>,
     bytes: warp::hyper::body::Bytes,
     src_ip:std::net::IpAddr,
     //headers:std::collections::HashMap<String,String>
) -> Result<impl warp::Reply, Rejection> {
    

    info!("Got request from {}",src_ip);
    let schema = graphql::create_schema();
    let schema = 
        Schema::build(graphql::QueryRoot, EmptyMutation, crate::graphql::SubscriptionRoot)
            .data(state.clone())
            .data(schema)
            .finish();    
    let body_string = String::from_utf8_lossy(&bytes);
    let json_value : serde_json::Value = serde_json::from_str(&body_string).unwrap();
    let query_string = json_value["query"].as_str().unwrap_or("");
    let request = async_graphql::Request::new(query_string);
    let response = schema.execute(request).await;
    Ok(warp::reply::json(&response))
}



mod args;

#[tokio::main(flavor = "multi_thread", worker_threads = 6)]
async fn main() -> Result<()> {
    
    let opt = crate::args::Opt::parse();
    let log_level = match opt.log_level {
        args::LogLevel::Verbose => "verbose",
        args::LogLevel::Info => "info",
        args::LogLevel::Warn => "warn",
    };

    init_logging(log_level,opt.otel_endpoint).await?;

    info!("Marlowe Indexer Started");

    debug!("Saving active schema to: {}/schema.sdl",std::env::current_dir().unwrap().display());        
    
    std::fs::write("schema.sdl", graphql::create_schema().sdl()).unwrap();
    
    let configuration = 
        match opt.network {
            crate::args::Network::Preprod => {
                any_err!(ConfigurationBuilder::new()
                    .with_magic(pallas_network::miniprotocols::PRE_PRODUCTION_MAGIC)
                    .with_address(opt.address)
                    .build())?
            },
            crate::args::Network::Mainnet => {
                any_err!(ConfigurationBuilder::new()
                    .with_magic(pallas_network::miniprotocols::MAINNET_MAGIC)
                    .with_address(opt.address)
                    .build())?
            },
            crate::args::Network::Preview => {
                any_err!(ConfigurationBuilder::new()
                    .with_magic(pallas_network::miniprotocols::PREVIEW_MAGIC)
                    .with_address(opt.address)
                    .build())?
            }
        };
        
    let shared_state = state::State::new();
    let locked_share = tokio::sync::RwLock::new(shared_state);
    let shared_state_arc = Arc::new(locked_share);
    
    let ccc = shared_state_arc.clone();

    let intersect = 
        match configuration.magic {
            pallas_network::miniprotocols::PRE_PRODUCTION_MAGIC => 
                (10847427,hex::decode("4194504ed513bedd432301f17738c8cc8c07eb9f5d58d673316344533fabfc23").unwrap()),
            pallas_network::miniprotocols::MAINNET_MAGIC => 
                (72748995,hex::decode("d86e9f41a81d4e372d9255a07856c46e9026d2eabe847d5404da2bbe5fb7f3c1").unwrap()),
            pallas_network::miniprotocols::PREVIEW_MAGIC => 
                (832495,hex::decode("641697acd9478e6aaafbdc6f08046ddc758df741d232342c87e2f26025286277").unwrap()),
            _ => panic!()
        };

    tokio::spawn(async move {
        let marlowe_worker = Box::new(MarloweSyncWorker::new(ccc));
        let mut ccs = 
            CardanoChainSync::new(
                configuration,
                marlowe_worker,
                intersect,
                if let crate::args::Mode::Socket = opt.mode { 
                    ConnectionType::Socket 
                } else { 
                    ConnectionType::Tcp 
                }
            );
        let span = info_span!("worker_thread");
        ccs.run().instrument(span).await;
        std::result::Result::<(),String>::Ok(())
    });

    let cors = warp::cors()
        .allow_any_origin() 
        .allow_methods(vec!["POST", "GET", "OPTIONS"]) 
        .allow_headers(vec!["Content-Type","origin"]); 
    
    let graphql_post = 
        warp::post()
            .and(warp::body::bytes())
            .and(warp::addr::remote())
            .and(warp::header::headers_cloned())
            .and_then(move |bytes,remote_addr: Option<std::net::SocketAddr>, headers:HeaderMap| {
                let shared_state_arc = shared_state_arc.clone();
                let source_ip = remote_addr.unwrap().ip();
                // let headers_map: std::collections::HashMap<String,String>  = headers
                //     .iter()
                //     .map(|header| (header.0.as_str().to_owned(), header.1.to_str().unwrap().to_owned()))
                //     .collect();
                async move {
                    
                    graphql_handler(
                        shared_state_arc, 
                        bytes,
                        source_ip,
                        //headers_map
                    ).await
                }
            }).with(cors.clone());


    let graphiql = warp::path::end().and(warp::get()).map(|| {
        HttpResponse::builder()
            .header("content-type", "text/html")
            .body(GraphiQLSource::build().endpoint("/").subscription_endpoint("/").finish())
    }).with(cors.clone());
    
 
    let routes = async_graphql_warp::graphql_subscription(
                graphql::create_schema()
            ).or(
        graphiql)
        .or(graphql_post)
        .recover(|err: Rejection| async move {
            if let Some(GraphQLBadRequest(err)) = err.find() {
                return Ok::<_, std::convert::Infallible>(warp::reply::with_status(
                    err.to_string(),
                    warp::hyper::StatusCode::BAD_REQUEST,
                ));
            }
            warn!("Invalid Request: {:?}",err);
            Ok(warp::reply::with_status(
                "INTERNAL_SERVER_ERROR".to_string(),
                warp::hyper::StatusCode::INTERNAL_SERVER_ERROR,
            ))
        }).with(warp::trace::request());

    _ = tokio::spawn(warp::serve(routes).run(([0, 0, 0, 0], 8000))).await;

    info!("Application stopping.");

    opentelemetry::global::shutdown_tracer_provider();

    Ok(())
}

