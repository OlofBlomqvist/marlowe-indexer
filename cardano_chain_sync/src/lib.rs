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

pub use pallas_network as pallas_network_ccs;


pub enum Client {
    TcpClient(PeerClient),
    SocketClient(NodeClient)
}

pub struct CardanoChainSync {
    pub client : Option<Client>,
    pub configuration: Configuration,
    pub worker: Box<(dyn ChainSyncReceiver + Send)>,
    pub intersect : Option<(u64,Vec<u8>)>,
    pub connection_type : ConnectionType
}


pub enum ConnectionType {
    Socket, Tcp
}

pub enum SyncBlock<'a>  {
    N2N(&'a pallas_network::miniprotocols::chainsync::NextResponse<pallas_network::miniprotocols::chainsync::HeaderContent>),
    N2C(&'a pallas_network::miniprotocols::chainsync::NextResponse<pallas_network::miniprotocols::chainsync::BlockContent>)
}

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
        CardanoChainSync { 
            configuration ,
            client: None,
            worker,
            intersect: Some(intersect.clone()),
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
