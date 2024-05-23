#[derive(serde::Deserialize)]
pub struct ProducerInfo {

    #[serde(alias = "address")]
    pub address : String,

    #[serde(alias = "port")]
    #[allow(dead_code)] pub port : u16,
}

#[derive(serde::Deserialize)]
pub struct TopologyData {
    #[serde(alias = "bootstrapPeers")]
    pub bootstrap_peers : Vec<ProducerInfo>
}