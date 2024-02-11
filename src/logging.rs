use opentelemetry_sdk::{Resource, trace::XrayIdGenerator};
use tracing::Level;
use tracing_subscriber::{FmtSubscriber, fmt::{format::FmtSpan, self}, filter::LevelFilter};

pub async fn init(level:Level,mut otel_tracing_endpoint:Option<String>) -> anyhow::Result<()> {

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
        
        let svc_name = "Cardano Chain Sync";

        let resource = Resource::new(vec![opentelemetry::KeyValue::new("service.name", svc_name)]);
        
        use tracing_subscriber::layer::SubscriberExt;
        use tracing_subscriber::Registry;     
        
        let exporter = opentelemetry_otlp::WithExportConfig::with_endpoint(opentelemetry_otlp::new_exporter().tonic(), otel_addr.clone());
    
        let tracer = opentelemetry_otlp::new_pipeline()
            .tracing()
            .with_trace_config(
                opentelemetry_sdk::trace::Config::default()
                    .with_sampler(opentelemetry_sdk::trace::Sampler::AlwaysOn)
                    .with_resource(resource).with_id_generator(XrayIdGenerator::default())
            )
            .with_exporter(exporter).with_batch_config(opentelemetry_sdk::trace::BatchConfig::default())
            .install_batch(opentelemetry_sdk::runtime::Tokio)
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


