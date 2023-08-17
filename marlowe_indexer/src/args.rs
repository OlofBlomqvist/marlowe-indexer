use clap::Parser;
use clap::ValueEnum;


#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum,Debug)]
pub enum Mode {
    Tcp,
    Socket,
}

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum,Debug)]
pub enum LogLevel {
    Verbose,
    Info,
    Warn,
}

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum,Debug)]
pub enum Network {
    Mainnet,
    Preview,
    Preprod,
}


#[derive(Parser, Debug,Clone)]
#[clap(version = "1.0", author = "Your Name", about = "Your Application Description")]
pub struct Opt {
    #[clap(short, long)]
    pub mode: Mode,

    #[clap(short, long)]
    pub address: String,

    /// example: https://127.0.0.1:4317/api/traces
    #[clap(short, long)]
    pub otel_endpoint: Option<String>,
    
    #[clap(short, long)]
    pub log_level: LogLevel,

    #[clap(short, long)]
    pub network: Network,
}

