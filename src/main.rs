
#![feature(async_closure)]
#![feature(coroutines)]
#![feature(unsized_fn_params)]
#![feature(type_alias_impl_trait)]
#![feature(ascii_char)]
#![feature(fs_try_exists)]

mod core;
use std::sync::Arc;
use args::{NodeToConnect, NetworkArg};
use core::{*, configuration::ConfigurationBuilder};
use clap::Parser;
use anyhow::{Result, anyhow};
use warp::{http::Response as HttpResponse, Rejection};

mod types;
mod modules;
mod logging;
mod state;
mod args;

use types::*;
use warp::Filter;
use core::lib::pallas_network_ccs as pallas_network;
use async_graphql_warp::GraphQLBadRequest;
use async_graphql::http::GraphiQLSource;

use crate::core::lib::ChainSyncState;

macro_rules! any_err {    
    ($expr:expr) => (anyhow::Context::with_context($expr.map_err(|e| anyhow!("{:?}", e)), ||concat!("@ ", file!(), " line ", line!(), " column ", column!())))
}



#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<()> {
    
    //console_subscriber::init();

    let opt = crate::args::Opt::parse();

    let log_level = match opt.log_level {
        args::LogLevel::Debug => tracing::Level::DEBUG,
        args::LogLevel::Info => tracing::Level::INFO,
        args::LogLevel::Warn => tracing::Level::WARN,
        args::LogLevel::Trace => tracing::Level::TRACE,
        args::LogLevel::Error => tracing::Level::ERROR,    
    };

    logging::init(log_level,opt.otel_trace_endpoint).await?;

    let span = tracing::error_span!("MAIN");
    span.entered().in_scope(||{
        tracing::info!("Cardano Chain-Sync GraphQL Started");
        //tracing::debug!("Saving active GraphQL schema to: {}/schema.sdl",std::env::current_dir().unwrap().display());        
        //std::fs::write("schema.sdl", graphql::create_schema().sdl()).unwrap();
    });
    

    let configuration = match &opt.node_connection {
        NodeToConnect::SocketSync { socket_path, network } => {
            let magic = args::network_as_u64(&network)?;
            let path = socket_path.clone().unwrap_or(std::env::var("CARDANO_NODE_SOCKET_PATH")?);
            tracing::info!("Connecting to socket '{path}' using network magic '{magic}'");
            any_err!(ConfigurationBuilder::new()
                    .with_magic(magic)
                    .with_address(&path)
                    .process_concurrently()
                    .build())?
        }
        NodeToConnect::TcpSync { address, network } => {
            let magic = args::network_as_u64(&network)?;
            let path = address.clone().unwrap_or(resolve_addr(&network).await?);
            tracing::info!("Connecting to address '{path}' using network magic '{magic}'");
            any_err!(ConfigurationBuilder::new()
                   .with_magic(magic)
                   .with_address(&path)
                   .process_concurrently()
                   .build())?
        }
    };


    
    
    let cors = warp::cors()
        .allow_any_origin() 
        .allow_methods(vec!["POST", "GET", "OPTIONS"]) 
        .allow_headers(vec!["Content-Type","origin"]); 

    let graphiql = warp::path::end()
        .and(warp::get())
        // we dont want to match websocket connections
        .and(warp::header::optional("Upgrade").and_then(async move |h:Option<String>|{
            if h.is_some() {
                Err(warp::reject())
            } else {
                Ok(())
            }
        }).boxed())
        .map(|_| {
        HttpResponse::builder()
            .header("content-type", "text/html")
            .body(GraphiQLSource::build().endpoint("/").subscription_endpoint("/").finish())
    }).with(cors.clone());
    
    let marlowe_state = crate::modules::marlowe::state::MarloweState::new();
    marlowe_state.init_mem_cache().await;

    let last_indexed_slot = marlowe_state.last_seen_slot_num();
    let last_indexed_block_hash = marlowe_state.last_seen_block_hash();

    let intersect = match (last_indexed_slot,last_indexed_block_hash) {
        (Some(a),Some(b)) => {
            Some(pallas_network::miniprotocols::Point::new(a,hex::decode(b).unwrap()))
        },
        (None,None) => {
            match &configuration.magic 
            {
                &pallas_network::miniprotocols::PRE_PRODUCTION_MAGIC => 
                    Some(pallas_network::miniprotocols::Point::new(10847427,hex::decode("4194504ed513bedd432301f17738c8cc8c07eb9f5d58d673316344533fabfc23").unwrap())),
                &pallas_network::miniprotocols::MAINNET_MAGIC => 
                    Some(pallas_network::miniprotocols::Point::new(72748995,hex::decode("d86e9f41a81d4e372d9255a07856c46e9026d2eabe847d5404da2bbe5fb7f3c1").unwrap())),
                &pallas_network::miniprotocols::PREVIEW_MAGIC => 
                    Some(pallas_network::miniprotocols::Point::new(832495,hex::decode("641697acd9478e6aaafbdc6f08046ddc758df741d232342c87e2f26025286277").unwrap())),
                _ => None
            }
        },
        (a,b) => panic!("inconsistent state: {:?} --> {:?}",a,b)
    };

    

    let inner_state = ChainSyncState::new(
        intersect.clone(),
        configuration,
        if let crate::args::NodeToConnect::SocketSync { 
            socket_path:_, network :_
        } = opt.node_connection  { 
            crate::lib::ConnectionType::Socket 
        } else { 
            crate::lib::ConnectionType::Tcp 
        });


    let sync_state = std::sync::Arc::new(inner_state);
    
    
    
    let shared_global_state = state::GlobalState::new(
        crate::modules::ModulesStates::new(marlowe_state),
        sync_state.clone()
    );
    

    let shared_global_state_arc = Arc::new(shared_global_state);
    
    let global_state_arc = shared_global_state_arc.clone();


    // Create endpoints and worker instances with state management pre-configured
    let (filters,workers) = 
        crate::modules::get_all(global_state_arc.clone());
        
        
    // Spin up the chain sync worker on a separate thread with its own runtime
    let worker_handle = std::thread::spawn( move ||  {
        let mut ccs = crate::core::lib::CardanoChainSync::new(workers, sync_state);
        let runtime = tokio::runtime::Runtime::new().expect("Failed to create a runtime");
        runtime.block_on(ccs.run())
    });       
    
    // Enable all modules to be reached thru their own gql filters

    //let rest_api_routes = crate::modules::marlowe::restapi::routes(global_state_arc.clone());

    // Combine the routes, REST API routes first
    let routes = 
        //rest_api_routes
        graphiql .or(filters)
        
        .recover(|err: Rejection| async move {
            if let Some(GraphQLBadRequest(err)) = err.find() {
                return Ok::<_, std::convert::Infallible>(warp::reply::with_status(
                    err.to_string(),
                    warp::hyper::StatusCode::BAD_REQUEST,
                ));
            }
            tracing::warn!("Invalid Request: {:?}", err);
            Ok(warp::reply::with_status(
                "INTERNAL_SERVER_ERROR".to_string(),
                warp::hyper::StatusCode::INTERNAL_SERVER_ERROR,
            ))
        });


    tokio::spawn(
        warp::serve(routes)
        .run(([0, 0, 0, 0], opt.graphql_listen_port)));
    
    worker_handle.join().expect("Couldn't join on the associated thread");

    tracing::info!("Application stopping gracefully.");
    
    opentelemetry::global::shutdown_tracer_provider();
    Ok(())

}



#[tracing::instrument]
async fn resolve_addr(network_name:&NetworkArg) -> anyhow::Result<String> {

    let url = match network_name {

        NetworkArg::Mainnet => 
            // "https://explorer.mainnet.cardano.org/relays/topology.json", 
            "https://raw.githubusercontent.com/input-output-hk/cardano-configurations/master/network/mainnet/cardano-node/topology.json",
        
        NetworkArg::Preview => 
            "https://raw.githubusercontent.com/input-output-hk/cardano-configurations/master/network/preview/cardano-node/topology.json",
        
        NetworkArg::Preprod =>
            "https://raw.githubusercontent.com/input-output-hk/cardano-configurations/master/network/preprod/cardano-node/topology.json",
        
        NetworkArg::Sanchonet => 
            "https://raw.githubusercontent.com/input-output-hk/cardano-configurations/master/network/sanchonet/cardano-node/topology.json",

    };

    tracing::info!("fetching topology.json from {}",&url);
    let result = reqwest::get(url).await.map_err(anyhow::Error::new)?;

    if !&result.status().is_success() {
        return Err(anyhow::Error::msg(format!("Failed to fetch topology from {url}. Status code: {:?}",result.status())))
    }

    let body = result.text().await?;
   
    let decoded_data : TopologyData = serde_json::de::from_str(&body)?;
    
    if let Some(p) = decoded_data.bootstrap_peers.first() {
        Ok(format!("{}:{}",p.address,p.port))
    } else {
        Err(anyhow::Error::msg(format!("Found no producers in the response from {url}")))
    }
    
    
} 




