
#![feature(addr_parse_ascii)]
#![feature(async_closure)]

use std::sync::Arc;
use args::{NodeToConnect, NetworkArg};
use cardano_chain_sync::{*, configuration::ConfigurationBuilder};
use clap::Parser;
use opentelemetry::sdk::{Resource, trace::XrayIdGenerator};
use tracing::Level;
use tracing_subscriber::{FmtSubscriber, fmt::{format::FmtSpan, self}, EnvFilter, filter::LevelFilter};
use anyhow::{Result, anyhow};
use warp::{http::Response as HttpResponse, Rejection};
use crate::worker::MarloweSyncWorker;
use warp::Filter;
mod worker;
mod state;

use cardano_chain_sync::pallas_network_ccs as pallas_network;

use async_graphql_warp::GraphQLBadRequest;
use async_graphql::{http::GraphiQLSource, EmptyMutation, Schema};
mod graphql;



macro_rules! any_err {    
    ($expr:expr) => (anyhow::Context::with_context($expr.map_err(|e| anyhow!("{:?}", e)), ||concat!("@ ", file!(), " line ", line!(), " column ", column!())))}


    pub async fn init_logging(level:Level,mut otel_tracing_endpoint:Option<String>) -> Result<()> {

        let (hyper_level,warp_level,pallas_level) = match &level {
            &Level::TRACE => (
                "hyper=debug".parse().unwrap(),
                "warp=trace".parse().unwrap(),
                "pallas_network=debug".parse().unwrap()
            ),
            &Level::DEBUG => (
                "hyper=info".parse().unwrap(),
                "warp=info".parse().unwrap(),
                "pallas_network=info".parse().unwrap()
            ),
            &Level::INFO => (
                "hyper=warn".parse().unwrap(),
                "warp=info".parse().unwrap(),
                "pallas_network=warn".parse().unwrap()
            ),
            &Level::WARN => (
                "hyper=warn".parse().unwrap(),
                "warp=warn".parse().unwrap(),
                "pallas_network=warn".parse().unwrap()
            ),
            &Level::ERROR => (
                "hyper=error".parse().unwrap(),
                "warp=error".parse().unwrap(),
                "pallas_network=error".parse().unwrap()
            ),
        };
        
        let env_filter = 
            tracing_subscriber::EnvFilter::new(LevelFilter::from_level(level).to_string())
                .add_directive(pallas_level)
                .add_directive(warp_level)
                .add_directive(hyper_level);
                

        if otel_tracing_endpoint.is_none() {
            match std::env::var("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT") {
                Ok(endpoint) => {
                    //println!(">>> ENV:OTEL_EXPORTER_OTLP_TRACES_ENDPOINT: {} ::: ", endpoint);
                    otel_tracing_endpoint = Some(endpoint);
                },
                _ => {},
            }
        }

        if otel_tracing_endpoint.is_some()  {
            match std::env::var("OTEL_SDK_DISABLED") {
                Ok(value) => {
                    if value.parse::<bool>().unwrap_or_default() {
                        tracing::info!(">>> ENV:OTEL_SDK_DISABLED: TRUE");
                        otel_tracing_endpoint = None;                   
                    }
                    
                },
                _ => {}
            }
        }

        if let Some(otel_addr) = &otel_tracing_endpoint {
            
            let svc_name = "Marlowe Sync";

            let resource = Resource::new(vec![opentelemetry::KeyValue::new("service.name", svc_name)]);
           
            use tracing_subscriber::layer::SubscriberExt;
            use tracing_subscriber::Registry;     
           
            let exporter = opentelemetry_otlp::WithExportConfig::with_endpoint(opentelemetry_otlp::new_exporter().tonic(), otel_addr.clone());
        
            let tracer = opentelemetry_otlp::new_pipeline()
                .tracing()
                .with_trace_config(
                    opentelemetry::sdk::trace::Config::default()
                        .with_sampler(opentelemetry::sdk::trace::Sampler::AlwaysOn)
                        .with_resource(resource).with_id_generator(XrayIdGenerator::default())
                )
                .with_exporter(exporter).with_batch_config(opentelemetry::sdk::trace::BatchConfig::default())
                .install_batch(opentelemetry::sdk::runtime::Tokio)
                .unwrap();

            let otel_layer = 
                tracing_opentelemetry::OpenTelemetryLayer::default()
                .with_tracer(tracer)
                .with_tracked_inactivity(true)
                .with_exception_fields(true)
                .with_exception_field_propagation(true)
                .with_location(true);
            
            
            let combo_subscriber = Registry::default()
                .with(env_filter)
                .with(otel_layer)
                .with(
                    fmt::Layer::default()
                        .with_line_number(true)
                        .with_span_events(FmtSpan::NONE)
                        .log_internal_errors(false)
                        .pretty()
                );
        
            tracing::subscriber::set_global_default(combo_subscriber).expect("unable to set global subscriber");

            tracing::info!("Logs will be sent to {}. Service name: {:?}. You can disable this using env:OTEL_SDK_DISABLED.",&otel_addr,svc_name);
    
            
            return  Ok(())    
        } 
         
        let console_subscriber = FmtSubscriber::builder()
            .with_env_filter(env_filter)
            .with_line_number(true)
            .with_span_events(FmtSpan::NONE)
            .log_internal_errors(false)
            .pretty().finish();      

            
        tracing::subscriber::set_global_default(console_subscriber)?;

        tracing::info!("No telemetry data will be sent from this application instance, only local logging is enabled.");
           
        Ok(())    
        
    }



#[tracing::instrument(skip_all)]
#[cfg_attr(feature = "GraphQL", derive(async_graphql::SimpleObject))]
async fn graphql_handler(
     state: Arc<tokio::sync::RwLock<state::State>>,
     bytes: warp::hyper::body::Bytes,
     src_ip:std::net::IpAddr,
     //headers:std::collections::HashMap<String,String>
) -> Result<impl warp::Reply, Rejection> {
    

    tracing::info!("Got request from {}",src_ip);

    let schema = graphql::create_schema();
    let schema = 
        Schema::build(graphql::types::QueryRoot, EmptyMutation, crate::graphql::types::SubscriptionRoot)
            .data(state.clone())
            .data(schema)
            .finish();    
    let body_string = String::from_utf8_lossy(&bytes);
    let json_value : serde_json::Value = serde_json::from_str(&body_string).unwrap();
    let query_string = json_value["query"].as_str().unwrap_or("");
    let request = async_graphql::Request::new(query_string);
    let response = schema.execute(request).await;
    let response_json = warp::reply::json(&response);

    Ok(warp::reply::with_status(response_json,reqwest::StatusCode::OK))
}



mod args;

#[tokio::main(flavor = "multi_thread", worker_threads = 6)]
async fn main() -> Result<()> {
    
    let opt = crate::args::Opt::parse();

    let log_level = match opt.log_level {
        args::LogLevel::Debug => tracing::Level::DEBUG.into(),
        args::LogLevel::Info => tracing::Level::INFO.into(),
        args::LogLevel::Warn => tracing::Level::WARN.into(),
        args::LogLevel::Trace => tracing::Level::TRACE.into(),
        args::LogLevel::Error => tracing::Level::ERROR.into(),    
    };

    init_logging(log_level,opt.otel_trace_endpoint).await?;

    let span = tracing::error_span!("MAIN");
    span.entered().in_scope(||{
        tracing::info!("Marlowe Indexer Started");
        tracing::debug!("Saving active GraphQL schema to: {}/schema.sdl",std::env::current_dir().unwrap().display());        
        std::fs::write("schema.sdl", graphql::create_schema().sdl()).unwrap();
    });
    

    let configuration = match &opt.node_connection {
        NodeToConnect::SocketSync { socket_path, network } => {
            let magic = get_magic(&network)?;
            let path = socket_path.clone().unwrap_or(std::env::var("CARDANO_NODE_SOCKET_PATH")?);
            tracing::info!("Connecting to socket '{path}' using network magic '{magic}'");
            any_err!(ConfigurationBuilder::new()
                    .with_magic(magic)
                    .with_address(&path)
                    .build())?

        }
        NodeToConnect::TcpSync { address, network } => {
            let magic = get_magic(&network)?;
            let path = address.clone().unwrap_or(resolve_addr(&network).await?);
            tracing::info!("Connecting to address '{path}' using network magic '{magic}'");
            any_err!(ConfigurationBuilder::new()
                   .with_magic(magic)
                   .with_address(&path)
                   .build())?
        }
    };

    let shared_state = state::State::new(opt.force_enable_graphql);
    let locked_share = tokio::sync::RwLock::new(shared_state);
    let shared_state_arc = Arc::new(locked_share);
    
    let ccc = shared_state_arc.clone();

    let intersect = 
        match configuration.magic 
        {
            pallas_network::miniprotocols::PRE_PRODUCTION_MAGIC => 
                Some(pallas_network::miniprotocols::Point::new(10847427,hex::decode("4194504ed513bedd432301f17738c8cc8c07eb9f5d58d673316344533fabfc23").unwrap())),
            pallas_network::miniprotocols::MAINNET_MAGIC => 
                Some(pallas_network::miniprotocols::Point::new(72748995,hex::decode("d86e9f41a81d4e372d9255a07856c46e9026d2eabe847d5404da2bbe5fb7f3c1").unwrap())),
            pallas_network::miniprotocols::PREVIEW_MAGIC => 
                Some(pallas_network::miniprotocols::Point::new(832495,hex::decode("641697acd9478e6aaafbdc6f08046ddc758df741d232342c87e2f26025286277").unwrap())),
            _ => 
                None
        };

        
    let worker_task = tokio::spawn(async move {
        let marlowe_worker = Box::new(MarloweSyncWorker::new(ccc));
        let mut ccs = 
            CardanoChainSync::new(
                configuration,
                marlowe_worker,
                intersect,
                if let crate::args::NodeToConnect::SocketSync { socket_path:_, network :_} = opt.node_connection  { 
                    ConnectionType::Socket 
                } else { 
                    ConnectionType::Tcp 
                }
            );
        
        ccs.run().await; // <-- we no longer create a root span here
                         //     since it was annoying to read the logs when all
                         //     events belonged to a single trace.
                         //     intead we create one trace per handled event from the node.
        std::result::Result::<(),String>::Ok(())

    });

    let cors = warp::cors()
        .allow_any_origin() 
        .allow_methods(vec!["POST", "GET", "OPTIONS"]) 
        .allow_headers(vec!["Content-Type","origin"]); 

    let shared_state_arc_gql_post = shared_state_arc.clone();

    let graphql_post = 
        warp::post()
            .and(warp::body::bytes())
            .and(warp::addr::remote())
            .and_then(move |bytes,remote_addr: Option<std::net::SocketAddr>| {
                let shared_state_arc = shared_state_arc_gql_post.clone();
                let source_ip = remote_addr.unwrap().ip();
                async move {
                    graphql_handler(
                        shared_state_arc, 
                        bytes,
                        source_ip
                    ).await
                }
            }).with(cors.clone());


    let graphiql = warp::path::end().and(warp::get()).map(|| {
        HttpResponse::builder()
            .header("content-type", "text/html")
            .body(GraphiQLSource::build().endpoint("/").subscription_endpoint("/").finish())
    }).with(cors.clone());
    
 
    let routes = async_graphql_warp::graphql_subscription(
                Schema::build(
                    graphql::types::QueryRoot, 
                    EmptyMutation, 
                    crate::graphql::types::SubscriptionRoot
                ).data(shared_state_arc.clone())
                 .data(graphql::create_schema())
                 .finish()
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
            tracing::warn!("Invalid Request: {:?}",err);
            Ok(warp::reply::with_status(
                "INTERNAL_SERVER_ERROR".to_string(),
                warp::hyper::StatusCode::INTERNAL_SERVER_ERROR,
            ))
        }).with(warp::trace::request());


    let warp_task = tokio::spawn(warp::serve(routes).run(([0, 0, 0, 0], opt.graphql_listen_port)));
    
    _ = tokio::try_join!(warp_task,worker_task);
    
    tracing::info!("Application stopping.");

    opentelemetry::global::shutdown_tracer_provider();

    Ok(())
}



pub enum AutoResolvedConnectionAddress{
    SocketFromEnvironment(String),
    TCPDefaultForNetwork(String)
}

#[derive(serde::Deserialize)]
struct ProducerInfo {

    #[serde(alias = "Addr")]
    pub addr : String,

    #[serde(alias = "Port")]
    #[allow(dead_code)] pub port : u16,

    #[serde(alias = "Continent")]
    #[allow(dead_code)] pub continent : Option<String>
}

#[derive(serde::Deserialize)]
struct TopologyData {
    #[serde(alias = "Producers")]
    pub producers : Vec<ProducerInfo>
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
    
    if let Some(p) = decoded_data.producers.first() {
        Ok(format!("{}:{}",p.addr,p.port))
    } else {
        Err(anyhow::Error::msg(format!("Found no producers in the response from {url}")))
    }
    
    
} 

#[tracing::instrument]
pub fn get_magic(network:&crate::args::NetworkArg) -> Result<u64> {
    match network {
        crate::args::NetworkArg::Mainnet => Ok(pallas_network::miniprotocols::MAINNET_MAGIC),
        crate::args::NetworkArg::Preview => Ok(pallas_network::miniprotocols::PREVIEW_MAGIC),
        crate::args::NetworkArg::Preprod => Ok(pallas_network::miniprotocols::PRE_PRODUCTION_MAGIC),
        crate::args::NetworkArg::Sanchonet=> Ok(4)
    }
    
}


