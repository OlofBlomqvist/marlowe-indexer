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

#[derive(Debug,Clone)]
pub struct Configuration {
    pub magic : u64,
    pub address : String,
    pub process_concurrently : bool
}

#[derive(Debug)]
pub struct ConfigurationBuilder {
    magic : Option<u64>,
    address : Option<String>,
    use_async : Option<bool>
}

impl ConfigurationBuilder {
    pub fn new() -> Self {Self{
        magic: None,
        address: None,
        use_async: None
    }}

    /// In some cases, you may want to process blocks with multiple workers concurrently instead of one at the time.
    /// If you only use a single worker, you should not use this.
    #[allow(dead_code)]
    pub fn process_concurrently(mut self) -> Self {
        self.use_async = Some(true);
        self
    }

    #[allow(dead_code)]
    pub fn with_preprod(mut self) -> Self {
        self.magic = Some(1);
        self
    }
    
    #[allow(dead_code)]
    pub fn with_magic(mut self,value:u64) -> Self {
        self.magic = Some(value);
        self
    }

    #[allow(dead_code)]
    pub fn with_address(mut self,value:&str) -> Self {
        self.address = Some(value.to_string());
        self
    }

    #[allow(dead_code)]
    pub fn build(self) -> Result<Configuration,error_stack::Report<ConfigurationBuilderError>> {
        
        let config = Ok(Configuration {
            process_concurrently : if let Some(true) = self.use_async { true } else { false },
            address: self.address
            .ok_or_else(|| ConfigurationBuilderError::new("address is not set.")).map_err(error_stack::Report::from)?,
            magic: self.magic
                .ok_or_else(|| ConfigurationBuilderError::new("magic is not set."))
                .map_err(error_stack::Report::from)?
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