#![feature(unsized_fn_params)]

pub use core::fmt;
pub mod configuration;

use configuration::Configuration;
use pallas_network::miniprotocols::Point;
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

pub use pallas_network as pallas_network_ccs;


pub enum Client {
    TcpClient(PeerClient),
    SocketClient(NodeClient)
}

pub struct CardanoChainSync {
    pub client : Option<Client>,
    pub configuration: Configuration,
    pub worker: Box<(dyn ChainSyncReceiver + Send)>,
    pub intersect : Option<pallas_network::miniprotocols::Point>,
    pub connection_type : ConnectionType,
    stored_original_intersect : Option<pallas_network::miniprotocols::Point>
}


pub enum ConnectionType {
    Socket, Tcp
}

pub enum SyncBlock<'a>  {
    N2N(&'a pallas_network::miniprotocols::chainsync::NextResponse<pallas_network::miniprotocols::chainsync::HeaderContent>),
    N2C(&'a pallas_network::miniprotocols::chainsync::NextResponse<pallas_network::miniprotocols::chainsync::BlockContent>)
}

impl CardanoChainSync {

    // we will just create new traces for each handled event rather than connecting them 
    // all to a single parent
    async fn connect(&mut self) -> Result<(),Report<ChainSyncError>> {
        
        debug!("Connecting to {} (using magic: {})",&self.configuration.address,&self.configuration.magic);

        if self.client.is_some() {
            return Ok(())
        }        
        
        // In some cases like mainnet,preprod,testnet we already have known intersect points
        // to use as we know there are no marlowe contracts earlier than that, but for others
        // like sanchonet - we start at the origin.
        let known_points : Vec<pallas_network::miniprotocols::Point> = 
            if let Some(point) = &self.intersect {
                vec![point.clone()]
            } else {
                vec![]
            };
            

        match self.connection_type {
            ConnectionType::Socket => {
                let mut client = 
                pallas_network::facades::NodeClient::connect(
                    &self.configuration.address, 
                    self.configuration.magic
                ).await.map_err(Report::from)
                    .change_context_lazy(||
                        ChainSyncError::new(&format!("Failed to connect to {} (using magic: {})",
                        &self.configuration.address,
                        &self.configuration.magic)))
                    .attach_printable_lazy(||"Is the address correct? Is the node running & online?")?;
                            
                if known_points.is_empty() {

                    // Connect chainsync at origin
                    info!("No intersect point set for this network, fetching origin..");
                    let origin = 
                        client
                            .chainsync()
                            .intersect_origin().await
                            .map_err(Report::from)
                            .change_context_lazy(||
                                ChainSyncError::new(&format!("Failed to fetch origin from {} (using magic: {})",
                                &self.configuration.address,
                                &self.configuration.magic)))
                            .attach_printable_lazy(||"Failed to find origin!")?;

                    let origin_slot = origin.slot_or_default();
                    info!("Origin received: {:?}",origin_slot);

                } else {

                    // Connect chain sync at specific point
                    match client.chainsync().find_intersect(known_points).await {
                        Ok((Some(p),_t)) => {
                            info!("Located specific point: {:?}",p)
                        },
                        Ok((None,t)) => {
                            info!("Failed to located point. Tip is: {:?}",t)
                        }
                        Err(e) => {
                            return Err(e).map_err(Report::from).change_context(ChainSyncError::new("failed to find intersect point"))
                        }
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
                    ).await.map_err(Report::from)
                        .change_context_lazy(||
                            ChainSyncError::new(&format!("Failed to connect to {} (using magic: {})",
                            &self.configuration.address,
                            &self.configuration.magic)))
                        .attach_printable_lazy(||"Is the address correct? Is the node running & online?")?;

                if known_points.is_empty() {
                    
                    // Connect chainsync at origin
                    info!("No intersect point set for this network, fetching origin..");
                    let origin = 
                        client
                            .chainsync()
                            .intersect_origin().await
                            .map_err(Report::from)
                            .change_context_lazy(||
                                ChainSyncError::new(&format!("Failed to fetch origin from {} (using magic: {})",
                                &self.configuration.address,
                                &self.configuration.magic)))
                            .attach_printable_lazy(||"Failed to find origin!")?;

                    let origin_slot = origin.slot_or_default();
                    
                    info!("Origin received: {:?}",origin_slot);

                } else {
                    // Connect at intersect point
                    match client.chainsync().find_intersect(known_points).await {
                        Ok((Some(p),_t)) => {
                            info!("Located specific point: {:?}",p)
                        },
                        Ok((None,t)) => {
                            info!("Failed to located point. Tip is: {:?}",t)
                        }
                        Err(e) => {
                            return Err(e).map_err(Report::from).change_context(ChainSyncError::new("failed to find intersect point"))
                        }
                    }    
                    
                }

                info!("TCP connection successfully established!");
                self.client = Some(Client::TcpClient(client));
      
            }
        }
      
        

        Ok(())
    }

    pub fn new(configuration:Configuration, worker: Box<dyn ChainSyncReceiver + Send>, intersect: Option<pallas_network::miniprotocols::Point>, connection_type: ConnectionType) -> Self {
        CardanoChainSync { 
            configuration ,
            client: None,
            worker,
            intersect: intersect.clone(),
            connection_type,
            // store this to prevent rollback to origin from going further back
            stored_original_intersect: intersect
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
                    
                    match &mut self.client {
                        None => Err(Report::new(ChainSyncError::Message("CardanoChainSync bug: no client".into()))),
                        Some(Client::TcpClient(x)) => {
                            let point_of_this_block: miniprotocols::Point = pallas_network::miniprotocols::Point::Specific(header.slot(), header.hash().to_vec());
                            let bytes = x.blockfetch()
                                .fetch_single(point_of_this_block.clone())
                                .await
                                .map_err(Report::from).change_context(ChainSyncError::new("no block!!"))?.to_vec();

                             // save the last successfully processed slot/block as intersect in case we get disconnected.
                            self.intersect =  Some(point_of_this_block);
                            Ok(ChainSyncEvent::Block(ChainSyncBlock { bytes: bytes.to_vec() }, tip.clone()))

                        },
                        Some(Client::SocketClient(_x)) => todo!()
                    }                  

    
                }
                NextResponse::RollBackward(point, tip) => {
                                        
                    match &point {
                        Point::Origin => {

                            info!("Rollback to origin");
                            
                            // we will refuse to go back further than the original intersect
                            // since we know for a fact there cannot have been marlowe contracts earlier than that
                            self.intersect = self.stored_original_intersect.clone();

                            Ok(ChainSyncEvent::Rollback { 
                                abs_slot_to_go_back_to: point.slot_or_default() , 
                                tip: tip.clone()
                            })
                        }
                        Point::Specific(
                            abs_slot_to_roll_back_to, 
                            block_hash
                        ) => {

                            if let Some(old_intersect) = &self.intersect {
                                
                                let old_abs_slot = old_intersect.slot_or_default();

                                if old_abs_slot < *abs_slot_to_roll_back_to {
                                    panic!("old intersect is lower than rollback: {} < {}",old_abs_slot,abs_slot_to_roll_back_to)
                                }
                            }

                            self.intersect = Some(point.clone());

                            warn!("Rollback to slot '{abs_slot_to_roll_back_to}', block hash: {}",
                                hex::encode(block_hash),
                            );

                            Ok(ChainSyncEvent::Rollback { 
                                abs_slot_to_go_back_to: *abs_slot_to_roll_back_to, 
                                tip: tip.clone()
                            })
                        }
                    }
    
                }
                chainsync::NextResponse::Await => {
                    tracing::debug!("chain-sync reached the tip of the chain. waiting for new data..");
                    Ok(ChainSyncEvent::Noop)
                }
            },

            // PIPE STUFF
            SyncBlock::N2C(next) => {
                
                match next {
                    NextResponse::RollForward(cbor, tip) => {
                        let block = MultiEraBlock::decode(cbor).map_err(Report::from).change_context(ChainSyncError::new("failed to find intersect point"))?;
                            
                        let slot = block.slot();
                        let hash = block.hash();
                        
                        let block_point: miniprotocols::Point = pallas_network::miniprotocols::Point::Specific(slot, hash.to_vec());

                        self.intersect =  Some(block_point);

                        Ok(ChainSyncEvent::Block(ChainSyncBlock { bytes: cbor.0.clone() }, tip.clone()))
                    }
                    NextResponse::RollBackward(point, tip) => {
                        
                        let abs_slot_to_roll_back_to = point.slot_or_default();

                        match &point {
                            Point::Origin => {

                                self.intersect = self.stored_original_intersect.clone();
                                info!("Rollback to origin");
                                Ok(ChainSyncEvent::Rollback { abs_slot_to_go_back_to:point.slot_or_default(), tip: tip.clone()})
                            },
                            Point::Specific(block_abs_slot, block_hash) => {
                                
                                if let Some(old_intersect) = &self.intersect {
                                    let old_abs_slot = old_intersect.slot_or_default();
                                    if old_abs_slot < abs_slot_to_roll_back_to {
                                        panic!("old intersect is lower than rollback: {} < {}",old_abs_slot,abs_slot_to_roll_back_to)
                                    }
                                }
    
                                self.intersect = Some(point.clone());
    
                                warn!("Rollback to slot '{block_abs_slot}', block hash: {}",
                                    hex::encode(block_hash),
                                );
    
                                Ok(ChainSyncEvent::Rollback { 
                                    abs_slot_to_go_back_to: abs_slot_to_roll_back_to, 
                                    tip: tip.clone()
                                })
                            },
                        }
                        
                        
                    }
                    NextResponse::Await => {
                        trace!("chain-sync reached the tip of the chain");
                        Ok(ChainSyncEvent::Noop)
                    }
                }
                
            }

        }
          
    }

    
    pub async fn next(&mut self) ->  Result<(),Report<ChainSyncError>> {
        
        match &mut self.client {
            None => Err(ChainSyncError::new("Not connected.")).map_err(Report::from),
            Some(Client::SocketClient(node_client)) => {
                
                let chain_sync = node_client.chainsync();
                let e : Option<ChainSyncEvent>;
                
                match chain_sync.has_agency() {
                    true => {
                        let next = chain_sync.request_next().await.map_err(Report::from).change_context(ChainSyncError::new("request next failed while we had agency"))?;
                        e = Some(self.handle_sync_response(SyncBlock::N2C(&next)).await?);
                    }
                    false => {
                        trace!("awaiting next block (blocking)");
                        let next = chain_sync.recv_while_must_reply().await.map_err(Report::from).change_context(ChainSyncError::new("recv_while_must_reply failed while they had agency"))?;
                        
                        e = Some(self.handle_sync_response(SyncBlock::N2C(&next)).await?);
                    }
                };
                              
                match e {
                    Some(ee) => {
                        self.worker.handle_event(&ee).await;
                        Ok(())
                    },
                    None => unreachable!(),
                }               
            },
            Some(Client::TcpClient(peer_client)) => {
                
                let must_await = peer_client.chainsync().state() != &chainsync::State::Idle;
                let e : Option<ChainSyncEvent>;
                
                if must_await {
                    let next = &peer_client.chainsync().recv_while_must_reply().await.map_err(Report::from).change_context(ChainSyncError::new("not cool"))?;
                  
                    e = Some(self.handle_sync_response(SyncBlock::N2N(next)).await?);
                } else {
                    let next = &peer_client.chainsync().request_next().await.map_err(Report::from).change_context(ChainSyncError::new("not cool"))?;
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
    Rollback { abs_slot_to_go_back_to:u64, tip:pallas_network::miniprotocols::chainsync::Tip},
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
