use error_stack::{Report, IntoReport};
use thiserror::Error;
use tracing::debug;

#[derive(Error, Debug)]
pub enum ConfigurationBuilderError {
    #[error("Not cool: `{0}`")]
    Message(String)
}
impl ConfigurationBuilderError {
    pub fn new(message:&str) -> Self {
        ConfigurationBuilderError::Message(message.to_string())
    }
}

#[derive(Debug)]
pub struct Configuration {
    pub magic : u64,
    pub address : String
}

#[derive(Debug)]
pub struct ConfigurationBuilder {
    magic : Option<u64>,
    address : Option<String>
}

impl ConfigurationBuilder {
    pub fn new() -> Self {Self{
        magic: None,
        address: None,
    }}
    pub fn with_preprod(mut self) -> Self {
        self.magic = Some(1);
        self
    }
    pub fn with_magic(mut self,value:u64) -> Self {
        self.magic = Some(value);
        self
    }
    pub fn with_address(mut self,value:String) -> Self {
        self.address = Some(value);
        self
    }

    pub fn build(self) -> Result<Configuration,Report<ConfigurationBuilderError>> {
        
        let config = Ok(Configuration {
            address: self.address
            .ok_or_else(|| ConfigurationBuilderError::new("address is not set.")).into_report()?,
            magic: self.magic
                .ok_or_else(|| ConfigurationBuilderError::new("magic is not set."))
                .into_report()?
        });

        debug!("Configuration OK: {config:?}");

        config
    }
}


impl Default for ConfigurationBuilder {
    fn default() -> Self {
        Self::new()
    }
}