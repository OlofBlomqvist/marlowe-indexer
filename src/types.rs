#[derive(serde::Deserialize)]
pub struct ProducerInfo {

    #[serde(alias = "Addr")]
    pub addr : String,

    #[serde(alias = "Port")]
    #[allow(dead_code)] pub port : u16,

    #[serde(alias = "Continent")]
    #[allow(dead_code)] pub continent : Option<String>
}

#[derive(serde::Deserialize)]
pub struct TopologyData {
    #[serde(alias = "Producers")]
    pub producers : Vec<ProducerInfo>
}