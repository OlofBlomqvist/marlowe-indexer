pub use core::fmt;
pub mod configuration;
use configuration::Configuration;
use error_stack::IntoReport;
use pallas_network::miniprotocols::chainsync::{NextResponse, HeaderContent};
use pallas_traverse::MultiEraBlock;
use tracing::{trace};
use tracing::{debug, error, info,warn};
use pallas_network::*;
use crate::facades::*;
use crate::miniprotocols::*;
use hex;
use error_stack::{ResultExt,Report};
use pallas::ledger::traverse::MultiEraHeader;
use thiserror::Error;

pub struct CardanoChainSync {
    pub client : Option<PeerClient>,
    pub configuration: Configuration,
    pub worker: Box<(dyn ChainSyncReceiver + Send)>

}


/// Well-known magic for testnet
pub const TESTNET_MAGIC: u64 = 1097911063;

/// Well-known magic for mainnet
pub const MAINNET_MAGIC: u64 = 764824073;

/// Well-known magic for pre-production
pub const PRE_PRODUCTION_MAGIC: u64 = 1;


impl CardanoChainSync {

    #[tracing::instrument(skip(self))]
    async fn connect(&mut self) -> Result<(),Report<ChainSyncError>> {
        
        debug!("Connecting to {} (using magic: {})",&self.configuration.address,&self.configuration.magic);

        if self.client.is_some() {
            return Ok(())
        }        

        let mut client = 
            pallas_network::facades::PeerClient::connect(
                &self.configuration.address, 
                self.configuration.magic
            ).await.into_report()
                .change_context_lazy(||
                    ChainSyncError::new(&format!("Failed to connect to {} (using magic: {})",
                    &self.configuration.address,
                    &self.configuration.magic)))
                .attach_printable_lazy(||"Is the address correct? Is the node running & online?")?;
                        
                    
        // todo : take pnt as arg
        // PREPROD 10847427 4194504ed513bedd432301f17738c8cc8c07eb9f5d58d673316344533fabfc23
        // PREVIEW 832495 641697acd9478e6aaafbdc6f08046ddc758df741d232342c87e2f26025286277 
        // MAINNET 72748995 d86e9f41a81d4e372d9255a07856c46e9026d2eabe847d5404da2bbe5fb7f3c1
        
        let known_points = 
            match self.configuration.magic {
                PRE_PRODUCTION_MAGIC => vec![Point::Specific(10847427,hex::decode("4194504ed513bedd432301f17738c8cc8c07eb9f5d58d673316344533fabfc23").unwrap())],
                MAINNET_MAGIC => vec![Point::Specific(72748995,hex::decode("d86e9f41a81d4e372d9255a07856c46e9026d2eabe847d5404da2bbe5fb7f3c1").unwrap())],
                PREVIEW_MAGIC => vec![Point::Specific(832495,hex::decode("641697acd9478e6aaafbdc6f08046ddc758df741d232342c87e2f26025286277").unwrap())],
                _ => panic!()
            }
        ;

        // _ = client.chainsync().intersect_tip().await
        //     .into_report().change_context(ChainSyncError::new("failed to find tip point"))?;

        match client.chainsync().find_intersect(known_points).await {
            Ok((Some(p),_t)) => {
                info!("Located specific point: {:?}",p)
            },
            Ok((None,t)) => {
                info!("Failed to located point. Tip is: {:?}",t)
            }
            Err(e) => {
                return Err(e).into_report().change_context(ChainSyncError::new("failed to find intersect point"))
            }
        }   

        self.client = Some(client);
        info!("Connection successfully established!");

        Ok(())
    }

    pub fn new(configuration:Configuration, worker: Box<dyn ChainSyncReceiver + Send>) -> Self {
        CardanoChainSync { 
            configuration: configuration ,
            client: None,
            worker: worker
        }
    }

    
    pub async fn run(&mut self) {
        
        

        loop {
            match self.connect().await {
                Ok(()) => {
                    break;
                },
                Err(e) => {
                    warn!("Failed to connect. {e:?}... \n\nRetrying in 5 seconds.");
                    tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                }
            }
        }
        
        loop {
            match self.next().await {
                Err(e) => {
                    warn!("{:?}... waiting for 1 second before retry",e);
                    _ = tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                },
                Ok(_) => {}
            }
        }
    }

    pub async fn handle_sync_response(peer_client:&mut PeerClient,next:&NextResponse<HeaderContent>) -> Result<ChainSyncEvent,Report<ChainSyncError>> {
        match next {
            NextResponse::RollForward(header, _tip) => {

                // todo : use tip info for logging progress.
                

                let header = to_traverse(header).unwrap();
                let slot = header.slot();
                let hash = header.hash();
                
                //info!("GOT INFO ABOUT NEW BLOCK TO FETCH : {hash:?}");
                let block = peer_client
                    .blockfetch()
                    .fetch_single(Point::Specific(slot, hash.to_vec()))
                    .await
                    .into_report().change_context(ChainSyncError::new("no block! not cool!"))?;
                
                //info!("WE HAVE NOW FETCHED {hash:?}. HANDLING THE BLOCK AS AN EVENT.");
                Ok(ChainSyncEvent::Block(ChainSyncBlock { bytes: block }))
            }
            NextResponse::RollBackward(point, _tip) => {

                match point {
                    Point::Origin => {
                        debug!("Rollback to origin");
                        Ok(ChainSyncEvent::Rollback { block_slot:0, block_hash: None})
                    }
                    Point::Specific(
                        block_slot, 
                        block_hash
                    ) => {
                        debug!("rollback to block '{:?}', slot: {}",block_hash,block_slot);
                        Ok(ChainSyncEvent::Rollback { block_slot:point.slot_or_default().into(), block_hash: Some(block_hash.clone())})
                    }
                }

            }
            chainsync::NextResponse::Await => {
                trace!("chain-sync reached the tip of the chain. waiting for new data..");
                Ok(ChainSyncEvent::Noop)
            }
        }        
    }

    
    pub async fn next(&mut self) -> Result<(),Report<ChainSyncError>> {
        
        match &mut self.client {
            None => Err(ChainSyncError::new("Not connected.")).into_report(),
            Some(peer_client) => {

                let must_await = peer_client.chainsync().state() != &chainsync::State::Idle;

                let e : Option<ChainSyncEvent>;
                if must_await {
                    let next = &peer_client.chainsync().recv_while_must_reply().await.into_report().change_context(ChainSyncError::new("not cool"))?;
                    e = Some(Self::handle_sync_response(peer_client,next).await?);
                } else {
                    let next = &peer_client.chainsync().request_next().await.into_report().change_context(ChainSyncError::new("not cool"))?;
                    e = Some(Self::handle_sync_response(peer_client,next).await?);
                }                
                match e {
                    Some(ee) => self.worker.handle_event(ee).await,
                    None => panic!("Should not be possible to get empty result here."),
                }
                
                Ok(())
            }
        }
    }

}

use async_trait::async_trait;

#[async_trait]
pub trait ChainSyncReceiver{
    async fn handle_event(&mut self,event:ChainSyncEvent);
}


fn to_traverse(header: &HeaderContent) -> Result<MultiEraHeader<'_>, String> {
    let out = match header.byron_prefix {
        Some((subtag, _)) => MultiEraHeader::decode(header.variant, Some(subtag), &header.cbor),
        None => MultiEraHeader::decode(header.variant, None, &header.cbor),
    };

    Ok(out.unwrap())
}


#[derive(Debug)]
pub struct ChainSyncBlock {
    bytes : Vec<u8>
}


#[derive(Debug)]
pub enum ChainSyncEvent {
    Block(ChainSyncBlock),
    Rollback { block_slot:u64, block_hash: Option<Vec<u8>> },
    Noop
}
impl ChainSyncBlock {
    pub fn new(bytes:Vec<u8>) -> ChainSyncBlock {
        ChainSyncBlock { bytes: bytes }
    }
    pub fn decode(&self) -> std::result::Result<MultiEraBlock<'_>, pallas_traverse::Error> {
        pallas_traverse::MultiEraBlock::decode(&self.bytes)
    }
}

#[derive(Error, Debug)]
pub enum ChainSyncError {
    #[error("`{0}`")]
    Message(String)
}
impl ChainSyncError {
    pub fn new(message:&str) -> Self {
        ChainSyncError::Message(message.to_string())
    }
}
