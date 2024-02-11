use clap::Parser;
use clap::ValueEnum;



#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum,Debug)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error    
}


#[derive(Clone, PartialEq, Eq, PartialOrd, Ord,Debug, clap::Subcommand)]
pub enum NodeToConnect {

    #[command(arg_required_else_help = true)]
    SocketSync {

        #[arg(last = true,help="this will be resolved automatically using environment variable 'CARDANO_NODE_SOCKET_PATH' if not provided")]
        socket_path : Option<String>,

        /// mainnet, preprod, preview, sanchonet
        #[arg(
            long,
            require_equals = true,
            value_enum
        )]
        network : NetworkArg
    },
    #[command(arg_required_else_help = true)]
    TcpSync {

        #[arg(last = true,help="this will be resolved automatically if not provided")]
        address : Option<String>,

        #[arg(
            long,
            require_equals = true,
            value_enum
        )]
        network : NetworkArg
    }
}


#[derive(ValueEnum, Copy, Clone, Debug, PartialEq, Eq,PartialOrd,Ord)]
pub enum NetworkArg {
    Mainnet,
    Preprod,
    Sanchonet,
    Preview
}

#[derive(Parser, Debug,Clone)]
#[command(version = "0.0.1-alpha", author = "Olof Blomqvist", about = "Marlowe Sync")]
pub struct Opt {

    /// Enable telemetry data to be sent to jaeger or other endpoints
    /// that support otel data.
    /// Example: https://127.0.0.1:4317/api/traces
    #[clap(short, long)]
    pub otel_trace_endpoint: Option<String>,
    
    #[clap(short, long, default_value="info")]
    pub log_level: LogLevel,

    #[clap(subcommand)]
    pub node_connection: NodeToConnect,

    #[clap(short, long, default_value="8000")]
    pub graphql_listen_port: u16,

}



pub fn network_as_u64(network:&crate::args::NetworkArg) -> anyhow::Result<u64> {
    match network {
        crate::args::NetworkArg::Mainnet => Ok(pallas::network::miniprotocols::MAINNET_MAGIC),
        crate::args::NetworkArg::Preview => Ok(pallas::network::miniprotocols::PREVIEW_MAGIC),
        crate::args::NetworkArg::Preprod => Ok(pallas::network::miniprotocols::PRE_PRODUCTION_MAGIC),
        crate::args::NetworkArg::Sanchonet=> Ok(4)
    }
}

