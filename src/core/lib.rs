use crossbeam::atomic::AtomicCell;
use super::configuration::Configuration;
use pallas_network::miniprotocols::Point;
use pallas_network::miniprotocols::chainsync::NextResponse;
use pallas_traverse::MultiEraBlock;
use tokio::sync::RwLock;
use tracing::trace;
use tracing::{debug, error, info,warn};
use pallas_network::*;
use pallas_network::facades::*;
use pallas_network::miniprotocols::*;
use error_stack::{ResultExt,Report};
use pallas::ledger::traverse::MultiEraHeader;
use thiserror::Error;

pub use pallas_network as pallas_network_ccs;


pub enum Client {
    TcpClient(PeerClient),
    SocketClient(NodeClient)
}

pub struct ChainSyncState {
    pub current_block_number : AtomicCell<Option<u64>>,
    pub tip_block_number : AtomicCell<Option<u64>>,
    pub tip_abs_slot : AtomicCell<Option<u64>>,
    pub tip_block_hash : AtomicCell<Option<String>>,    
    pub current_block_abs_slot :  AtomicCell<Option<u64>>,
    pub configuration: Configuration,
    pub stored_original_intersect : RwLock<Option<Point>>,
    pub connection_type : ConnectionType,
}

#[allow(unused)]
pub struct SlotConfig {
    pub zero_time: u64,
    pub zero_slot: u64,
    pub slot_length: u64,
}


impl ChainSyncState {
    pub fn new(intersect:Option<Point>,configuration:Configuration,connection_type:ConnectionType) -> Self {
        ChainSyncState { 
            current_block_number: AtomicCell::new(None), 
            current_block_abs_slot: AtomicCell::new(None),
            tip_block_number: AtomicCell::new(None), 
            tip_abs_slot : AtomicCell::new(None),
            tip_block_hash: AtomicCell::new(None),
            configuration,
            connection_type,
            stored_original_intersect: RwLock::new(intersect.clone())
        }
    }

    #[cfg(feature="debug")]
    #[allow(dead_code)]
    fn slot_config(&self) -> SlotConfig {
        slot_config_by_magic(self.configuration.magic)
    } 
    
    #[cfg(feature="debug")]
    #[allow(dead_code)]
    pub fn slot_to_posix_time(&self,slot: u64) -> u64 {
        let sc = self.slot_config();
        let ms_after_begin = (slot - sc.zero_slot) * sc.slot_length;
        sc.zero_time + ms_after_begin
    }

    #[cfg(feature="debug")]
    #[allow(dead_code)]
    pub fn posix_time_to_slot(&self,posix_time: u64) -> u64 {
        let sc = self.slot_config();
        ((posix_time - sc.zero_time) / sc.slot_length) + sc.zero_slot
    }


}

// https://book.world.dev.cardano.org/environments/preprod/shelley-genesis.json
// https://book.world.dev.cardano.org/environments/mainnet/shelley-genesis.json
// https://book.world.dev.cardano.org/environments/preview/shelley-genesis.json
// https://book.world.dev.cardano.org/environments/sanchonet/shelley-genesis.json
fn slot_config_by_magic(magic:u64) -> SlotConfig {
    match magic {

        // MAINNET
        764824073 => SlotConfig {
            zero_time: 1596059091000, 
            zero_slot: 4492800, 
            slot_length: 1000
        },

        // PREPROD
        1 => SlotConfig {
            zero_time: 1655683200000,
            zero_slot: 0,
            slot_length: 1000,
        },

        // PREVIEW
        3 => SlotConfig {
            zero_time: 1666656000000, 
            zero_slot: 0, 
            slot_length: 1000
        },

        // SANCHONET
        4 => SlotConfig {
            zero_time: 1686789000000, 
            zero_slot: 0, 
            slot_length: 1000
        },


        x => unimplemented!("slot_config_by_magic for this network: {x}")
    }
}

pub fn slot_to_posix_time_with_specific_magic(slot:u64,magic:u64) -> u64 {
    let sc = slot_config_by_magic(magic);
    let ms_after_begin = (slot - sc.zero_slot) * sc.slot_length;
    sc.zero_time + ms_after_begin
}

#[allow(dead_code)]
pub fn posix_time_to_slot_with_specific_magic(posix_time:u64,magic:u64) -> u64 {
    let sc = slot_config_by_magic(magic);
    ((posix_time - sc.zero_time) / sc.slot_length) + sc.zero_slot
}

pub struct CardanoChainSync  {
    pub client : Option<Client>,
    pub workers: Vec<Box<dyn ChainSyncReceiver>>,
    pub sync_state : std::sync::Arc<ChainSyncState>,
    intersect : Option<pallas_network::miniprotocols::Point>,
}

#[derive(Clone)]
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
        
        
        debug!("Connecting to {} (using magic: {})--> {:?}.. intersect: {:?}",&self.sync_state.configuration.address,&self.sync_state.configuration.magic,&self.sync_state.current_block_number,&self.intersect);

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
            

        match &self.sync_state.connection_type {
            ConnectionType::Socket => {
                
                let mut client = 
                pallas_network::facades::NodeClient::connect(
                    &self.sync_state.configuration.address, 
                    self.sync_state.configuration.magic
                ).await.map_err(Report::from)
                    .change_context_lazy(||
                        ChainSyncError::new(&format!("Failed to connect to {} (using magic: {})",
                        &self.sync_state.configuration.address,
                        &self.sync_state.configuration.magic)))
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
                                &self.sync_state.configuration.address,
                                &self.sync_state.configuration.magic)))
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
                        &self.sync_state.configuration.address, 
                        self.sync_state.configuration.magic
                    ).await.map_err(Report::from)
                        .change_context_lazy(||
                            ChainSyncError::new(&format!("Failed to connect to {} (using magic: {})",
                            &self.sync_state.configuration.address,
                            &self.sync_state.configuration.magic)))
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
                                &self.sync_state.configuration.address,
                                &self.sync_state.configuration.magic)))
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
                
                warn!("Using TCP is often slower than socket connections");

                self.client = Some(Client::TcpClient(client));
      
            }
        }
      
        

        Ok(())
    }

    pub fn new(
        workers: Vec<Box<dyn ChainSyncReceiver>>, 
        state: std::sync::Arc<ChainSyncState>  
    ) -> Self {
        CardanoChainSync { 
            intersect: state.clone().stored_original_intersect.blocking_read().clone(),
            client: None,
            workers,
            sync_state: state
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
                    Err(e) => error!("{e:?}"),
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
                            self.intersect = Some(point_of_this_block.clone());
                            
                            self.sync_state.tip_block_number.store(Some(tip.1.clone()));
                            self.sync_state.tip_abs_slot.store(Some(tip.0.slot_or_default()));

                            let tip_hash = match &tip.0 {
                                Point::Origin => None,
                                Point::Specific(_abs_slot,hash) => Some(hex::encode(hash)),
                            };
                            
                            self.sync_state.tip_block_hash.store(tip_hash);
                            self.sync_state.current_block_abs_slot.store(Some(point_of_this_block.slot_or_default()));
                            self.sync_state.current_block_number.store(Some(header.number()));
                            
                            Ok(ChainSyncEvent::Block(ChainSyncBlock { bytes: bytes.to_vec() }, tip.clone()))

                        },
                        Some(Client::SocketClient(_x)) => unreachable!()
                    }                  

    
                }
                NextResponse::RollBackward(point, tip) => {
                                        
                    match &point {
                        Point::Origin => {

                            info!("Rollback to origin");
                            
                            let tip_hash = match &tip.0 {
                                Point::Origin => None,
                                Point::Specific(_abs_slot,hash) => Some(hex::encode(hash)),
                            };
                            self.sync_state.tip_block_number.store(Some(tip.1));
                            self.sync_state.tip_abs_slot.store(Some(tip.0.slot_or_default()));
                            self.sync_state.tip_block_hash.store(tip_hash);

                            self.intersect = None;
                            // we will refuse to go back further than the original intersect
                            // since we know for a fact there cannot have been marlowe contracts earlier than that
                            self.sync_state.current_block_abs_slot.store(None);
                            self.sync_state.current_block_number.store(None);
                            Ok(ChainSyncEvent::Rollback { 
                                abs_slot_to_go_back_to: point.slot_or_default() , 
                                tip: tip.clone(),
                                block_hash: None
                            })
                        }
                        Point::Specific(
                            abs_slot_to_roll_back_to, 
                            block_hash
                        ) => {
                            let block_hash_hex = hex::encode(block_hash);

                            if let Some(old_intersect) = &self.intersect {
                                
                                let old_abs_slot = old_intersect.slot_or_default();

                                if old_abs_slot < *abs_slot_to_roll_back_to {
                                    panic!("old intersect is lower than rollback: {} < {}",old_abs_slot,abs_slot_to_roll_back_to)
                                }
                            }

                            self.intersect = Some(point.clone());
                            self.sync_state.current_block_abs_slot.store(Some(abs_slot_to_roll_back_to.clone()));
                            self.sync_state.current_block_number.store(None);

                             let tip_hash = match &tip.0 {
                                Point::Origin => None,
                                Point::Specific(_abs_slot,hash) => Some(hex::encode(hash)),
                            };
                            self.sync_state.tip_block_number.store(Some(tip.1));
                            self.sync_state.tip_abs_slot.store(Some(tip.0.slot_or_default()));
                            self.sync_state.tip_block_hash.store(tip_hash);

                            warn!("Rollback to slot '{abs_slot_to_roll_back_to}', block hash: {}",
                                hex::encode(block_hash),
                            );

                            Ok(ChainSyncEvent::Rollback { 
                                abs_slot_to_go_back_to: *abs_slot_to_roll_back_to, 
                                tip: tip.clone(),
                                block_hash: Some(block_hash_hex)
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
                        self.sync_state.current_block_abs_slot.store(Some(slot.clone()));
                        self.sync_state.current_block_number.store(Some(block.number()));
                        let tip_hash = match &tip.0 {
                            Point::Origin => None,
                            Point::Specific(_abs_slot,hash) => Some(hex::encode(hash)),
                        };

                        self.sync_state.tip_block_number.store(Some(tip.1));
                        self.sync_state.tip_abs_slot.store(Some(tip.0.slot_or_default()));
                        self.sync_state.tip_block_hash.store(tip_hash);

                        Ok(ChainSyncEvent::Block(ChainSyncBlock { bytes: cbor.0.clone() }, tip.clone()))
                    }
                    NextResponse::RollBackward(point, tip) => {
                        
                        let abs_slot_to_roll_back_to = point.slot_or_default();

                        match &point {
                            Point::Origin => {

                                self.intersect = None;
                                self.sync_state.current_block_abs_slot.store(Some(abs_slot_to_roll_back_to));
                                self.sync_state.current_block_number.store(None);
                               
                                let tip_hash = match &tip.0 {
                                    Point::Origin => None,
                                    Point::Specific(_abs_slot,hash) => Some(hex::encode(hash)),
                                };
                                self.sync_state.tip_block_number.store(Some(tip.1));
                                self.sync_state.tip_abs_slot.store(Some(tip.0.slot_or_default()));
                                self.sync_state.tip_block_hash.store(tip_hash);

                                info!("Rollback to origin");
                                Ok(ChainSyncEvent::Rollback { abs_slot_to_go_back_to:point.slot_or_default(), tip: tip.clone(), block_hash:None})
                            },
                            Point::Specific(block_abs_slot, block_hash) => {
                                
                                if let Some(old_intersect) = &self.intersect {
                                    let old_abs_slot = old_intersect.slot_or_default();
                                    if old_abs_slot < abs_slot_to_roll_back_to {
                                        panic!("old intersect is lower than rollback: {} < {}",old_abs_slot,abs_slot_to_roll_back_to)
                                    }
                                }
    
                                self.intersect = Some(point.clone());
                                self.sync_state.tip_block_number.store(Some(tip.1));
                                self.sync_state.tip_abs_slot.store(Some(tip.0.slot_or_default()));
                                self.sync_state.current_block_abs_slot.store(Some(abs_slot_to_roll_back_to));
                                self.sync_state.current_block_number.store(None);
                               
                                let tip_hash = match &tip.0 {
                                    Point::Origin => None,
                                    Point::Specific(_abs_slot,hash) => Some(hex::encode(hash)),
                                };

                                self.sync_state.tip_block_number.store(Some(tip.1));
                                self.sync_state.tip_abs_slot.store(Some(tip.0.slot_or_default()));
                                self.sync_state.tip_block_hash.store(tip_hash);

                                warn!("Rollback to slot '{block_abs_slot}', block hash: {}",
                                    hex::encode(block_hash),
                                );
                                
                                let block_hash_hex = hex::encode(block_hash);
                                Ok(ChainSyncEvent::Rollback { 
                                    abs_slot_to_go_back_to: abs_slot_to_roll_back_to, 
                                    tip: tip.clone(),
                                    block_hash: Some(block_hash_hex)
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

                        let process_concurrently = self.sync_state.configuration.process_concurrently;
                        
                        if process_concurrently {
                            let tasks = self.workers.iter_mut().map(|w|w.handle_event(&ee));
                            _ = futures::future::select_all(tasks).await;
                        } else {
                            for w in &mut self.workers {
                                w.handle_event(&ee).await;
                            }
                        }    
                                         
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
                        
                        let process_concurrently = self.sync_state.configuration.process_concurrently;
                        
                        if process_concurrently {
                            let tasks = self.workers.iter_mut().map(|w|w.handle_event(&ee));
                            _ = futures::future::select_all(tasks).await;
                        } else {
                            for w in &mut self.workers {
                                w.handle_event(&ee).await;
                            }
                        }
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
pub trait ChainSyncReceiver : Send {
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
    Rollback { abs_slot_to_go_back_to:u64, tip:pallas_network::miniprotocols::chainsync::Tip, block_hash: Option<String>},
    Noop
}

impl ChainSyncBlock {
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
