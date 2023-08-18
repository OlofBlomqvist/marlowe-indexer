#![feature(unsized_fn_params)]

pub use core::fmt;
pub mod configuration;

use configuration::Configuration;
use error_stack::IntoReport;
use pallas_network::miniprotocols::chainsync::NextResponse;
use pallas_traverse::MultiEraBlock;
use tracing::trace;
use tracing::{debug, error, info,warn};
use pallas_network::*;
use crate::facades::*;
use crate::miniprotocols::*;
use error_stack::{ResultExt,Report};
use pallas::ledger::traverse::MultiEraHeader;
use thiserror::Error;
use std::time::Instant;

pub use pallas_network as pallas_network_ccs;

pub struct ProgressTracker {
    start_time: Instant,
    total_items: u64,
    processed_items: u64,
    skipped_items: Option<u64>,
}

impl ProgressTracker {
    
    pub fn new(total_items: u64) -> Self {
        ProgressTracker {
            start_time: Instant::now(),
            total_items,
            processed_items: 0,
            skipped_items: None
        }
    }

    pub fn update_skipped(&mut self, skipped: u64) {
        self.skipped_items = Some(skipped);
    }

    pub fn update_processed(&mut self, processed: u64, total: u64) {
        self.processed_items = processed;
        self.total_items = total;
    }
    
    pub fn percentage_complete(&self) -> f64 {
        if self.total_items == 0 {
            return 0.0; // To avoid division by zero.
        }
        (self.processed_items as f64 / self.total_items as f64) * 100.0
    }

    pub fn estimated_hours_minutes_left(&self) -> (u64,u64) {

        // If we haven't processed items beyond the skipped ones, we can't estimate the time left.
        if let Some(skipped) = &self.skipped_items { 
            if &self.processed_items <= skipped {
                return (0, 0);
            }
        }
    
        // Calculate the elapsed time in seconds.
        let elapsed_seconds = self.start_time.elapsed().as_secs();
    
        // Calculate the rate of processing items per second, accounting for skipped items.
        let effective_processed = 
            if let Some(skipped) = &self.skipped_items {
                self.processed_items - skipped
            } else {
                self.processed_items
            };

        let rate = effective_processed as f64 / elapsed_seconds as f64;
    
        // Calculate the estimated time required for the remaining items.
        let remaining_items = self.total_items - self.processed_items;
        let estimated_seconds_left = (remaining_items as f64 / rate) as u64;
    
        // Convert the estimated time to hours and minutes.
        let hours = estimated_seconds_left / 3600;
        let minutes = (estimated_seconds_left % 3600) / 60;
    
        (hours, minutes)
    }
    
    
    
    

}

pub enum Client {
    TcpClient(PeerClient),
    SocketClient(NodeClient)
}

pub struct CardanoChainSync {
    pub client : Option<Client>,
    pub configuration: Configuration,
    pub worker: Box<(dyn ChainSyncReceiver + Send)>,
    pub intersect : Option<(u64,Vec<u8>)>,
    pub progress : ProgressTracker,
    pub connection_type : ConnectionType
}


pub enum ConnectionType {
    Socket, Tcp
}

pub enum SyncBlock<'a>  {
    N2N(&'a pallas_network::miniprotocols::chainsync::NextResponse<pallas_network::miniprotocols::chainsync::HeaderContent>),
    N2C(&'a pallas_network::miniprotocols::chainsync::NextResponse<pallas_network::miniprotocols::chainsync::BlockContent>)
}

// TODO - Should we sync blocks here, or at least allow for enabling that, so that 
// if adding workers to an existing ccs service, we dont need to sync all blocks from a node?

impl CardanoChainSync {

    #[tracing::instrument(skip(self))]
    async fn connect(&mut self) -> Result<(),Report<ChainSyncError>> {
        
        debug!("Connecting to {} (using magic: {})",&self.configuration.address,&self.configuration.magic);

        if self.client.is_some() {
            return Ok(())
        }        
        
        let known_points : Vec<Point> = 
            if let Some(point) = &self.intersect {
                vec![Point::new(point.0, point.1.clone())]
            } else {
                vec![]
            };
            

        match self.connection_type {
            ConnectionType::Socket => {
                let mut client = 
                pallas_network::facades::NodeClient::connect(
                    &self.configuration.address, 
                    self.configuration.magic
                ).await.into_report()
                    .change_context_lazy(||
                        ChainSyncError::new(&format!("Failed to connect to {} (using magic: {})",
                        &self.configuration.address,
                        &self.configuration.magic)))
                    .attach_printable_lazy(||"Is the address correct? Is the node running & online?")?;
                            
                
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
                info!("Socket connection successfully established!");
                self.client = Some(Client::SocketClient(client));
     
            },
            ConnectionType::Tcp => {

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
                self.client = Some(Client::TcpClient(client));
                info!("TCP connection successfully established!");
      
            }
        }
      
        

        Ok(())
    }

    pub fn new(configuration:Configuration, worker: Box<dyn ChainSyncReceiver + Send>, intersect: (u64,Vec<u8>), connection_type: ConnectionType) -> Self {
        let pt = ProgressTracker::new(0);        
        CardanoChainSync { 
            configuration ,
            client: None,
            worker,
            intersect: Some(intersect.clone()),
            progress: pt,
            connection_type
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
            if let Err(e) = self.next().await {

                warn!("{:?}... waiting for 1 second before retry",e);
                _ = tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                
                // attempt to reconnect..
                self.client = None;
                match self.connect().await {
                    Ok(_) => println!("RE-CONNECTED!"),
                    Err(e) => println!("{e:?}"),
                }
            }
        }
    }

    
    pub async fn handle_sync_response<'a>(&mut self,next:SyncBlock<'a>) -> Result<ChainSyncEvent,Report<ChainSyncError>> {
        
        match next {

            // TCP CLIENT

            SyncBlock::N2N(next) => match next {
                NextResponse::RollForward(header, tip) => {
                    
                    

                    let header = to_traverse(header).unwrap();
                    let slot = header.slot();
                    let hash = header.hash();
                    
                    self.intersect =  Some((slot,hash.to_vec()));

                    if self.progress.skipped_items.is_none() {
                        self.progress.update_skipped(header.number())
                    }
                    
                    self.progress.update_processed(header.number(), tip.1);
                    
                    //info!("[{}% DONE] Estimated time until fully synced: {}h {}m. (skipped:{:?} , processed: {})",x,info.0,info.1,self.progress.skipped_items, self.progress.processed_items);

                    //info!("GOT INFO ABOUT NEW BLOCK TO FETCH : {hash:?}");
                    match &mut self.client {
                        None => Err(Report::new(ChainSyncError::Message("CardanoChainSync bug: no client".into()))),
                        Some(Client::TcpClient(x)) => {
                            let bytes = x.blockfetch()
                                .fetch_single(Point::Specific(slot, hash.to_vec()))
                                .await
                                .into_report().change_context(ChainSyncError::new("no block! not cool!"))?.to_vec();
                            Ok(ChainSyncEvent::Block(ChainSyncBlock { bytes: bytes.to_vec() }, tip.clone()))
                        },
                        Some(Client::SocketClient(_x)) => todo!()
                    }
    
                }
                NextResponse::RollBackward(point, _tip) => {
    
                    match point {
                        Point::Origin => {
                            debug!("Rollback to origin");
                            Ok(ChainSyncEvent::Rollback { block_slot:0, tip: _tip.clone()})
                        }
                        Point::Specific(
                            block_slot, 
                            block_hash
                        ) => {
                            self.intersect = Some((*block_slot,block_hash.clone()));
                            debug!("rollback to block '{:?}', slot: {}",block_hash,block_slot);
                            Ok(ChainSyncEvent::Rollback { block_slot:point.slot_or_default(), tip: _tip.clone()})
                        }
                    }
    
                }
                chainsync::NextResponse::Await => {
                    tracing::trace!("chain-sync reached the tip of the chain. waiting for new data..");
                    Ok(ChainSyncEvent::Noop)
                }
            },

            // PIPE STUFF
            SyncBlock::N2C(next) => {
                
                match next {
                    NextResponse::RollForward(cbor, tip) => {
                        let block = MultiEraBlock::decode(cbor).into_report().change_context(ChainSyncError::new("failed to find intersect point"))?;
                            
                        let slot = block.slot();
                        let hash = block.hash();
                        self.intersect =  Some((slot,hash.to_vec()));
                        
                        
                        if self.progress.skipped_items.is_none() {
                            self.progress.update_skipped(block.number())
                        }
                        
                        self.progress.update_processed(block.number(), tip.1);
                        let _info = self.progress.estimated_hours_minutes_left();
                        let _x = self.progress.percentage_complete() as u64;
                        //info!("[{}% DONE] Estimated time until fully synced: {}h {}m. (skipped:{:?} , processed: {})",x,info.0,info.1,self.progress.skipped_items, self.progress.processed_items);


                        Ok(ChainSyncEvent::Block(ChainSyncBlock { bytes: cbor.0.clone() }, tip.clone()))
                    }
                    NextResponse::RollBackward(point, tip) => {
                        match &point {
                            Point::Origin => debug!("Rollback to origin"),
                            Point::Specific(slot, _) => debug!("Rollback to slot {slot}"),
                        };
                        Ok(ChainSyncEvent::Rollback { block_slot:point.slot_or_default(), tip: tip.clone()})
                    }
                    NextResponse::Await => {
                        info!("chain-sync reached the tip of the chain");
                        Ok(ChainSyncEvent::Noop)
                    }
                }
                
            }

        }
          
    }

    
    pub async fn next(&mut self) ->  Result<(),Report<ChainSyncError>> {
        
        match &mut self.client {
            None => Err(ChainSyncError::new("Not connected.")).into_report(),
            Some(Client::SocketClient(node_client)) => {
                
                let chain_sync = node_client.chainsync();
                let e : Option<ChainSyncEvent>;
                
                match chain_sync.has_agency() {
                    true => {
                        let next = chain_sync.request_next().await.into_report().change_context(ChainSyncError::new("not cool"))?;
                        e = Some(self.handle_sync_response(SyncBlock::N2C(&next)).await?);
                    }
                    false => {
                        trace!("awaiting next block (blocking)");
                        let next = chain_sync.recv_while_must_reply().await.into_report().change_context(ChainSyncError::new("not cool"))?;
                        
                        e = Some(self.handle_sync_response(SyncBlock::N2C(&next)).await?);
                    }
                };
                              
                match e {
                    Some(ee) => {
                        self.worker.handle_event(&ee).await;
                        Ok(())
                    },
                    None => panic!("Should not be possible to get empty result here."),
                }               
            },
            Some(Client::TcpClient(peer_client)) => {
                
                let must_await = peer_client.chainsync().state() != &chainsync::State::Idle;
                let e : Option<ChainSyncEvent>;
                
                if must_await {
                    let next = &peer_client.chainsync().recv_while_must_reply().await.into_report().change_context(ChainSyncError::new("not cool"))?;
                  
                    e = Some(self.handle_sync_response(SyncBlock::N2N(next)).await?);
                } else {
                    let next = &peer_client.chainsync().request_next().await.into_report().change_context(ChainSyncError::new("not cool"))?;
                    e = Some(self.handle_sync_response(SyncBlock::N2N(next)).await?);
                }   
                             
                match e {
                    Some(ee) => {
                        self.worker.handle_event(&ee).await;
                        Ok(())
                    },
                    None => panic!("Should not be possible to get empty result here."),
                }
            }
        }
    }

}

use async_trait::async_trait;

#[async_trait]
pub trait ChainSyncReceiver{
    async fn handle_event(&mut self,event:&ChainSyncEvent);
}


fn to_traverse(header: &pallas_network::miniprotocols::chainsync::HeaderContent) -> Result<MultiEraHeader<'_>, String> {
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
    Block(ChainSyncBlock,pallas_network::miniprotocols::chainsync::Tip),
    Rollback { block_slot:u64, tip:pallas_network::miniprotocols::chainsync::Tip},
    Noop
}

impl ChainSyncBlock {
    pub fn new(bytes:Vec<u8>) -> ChainSyncBlock {
        ChainSyncBlock { bytes }
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
