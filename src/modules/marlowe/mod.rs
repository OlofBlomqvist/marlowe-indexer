use std::sync::Arc;
use async_graphql::{SimpleObject, Enum};
use crate::core::lib::slot_to_posix_time_with_specific_magic;
use marlowe_lang::semantics::{ContractSemantics, MachineState};
use tokio::sync::mpsc::UnboundedReceiver;
use tracing::info;
pub mod state;
pub mod worker;
pub mod graphql;

use crate::modules::marlowe::state::*;


#[derive(Debug)]
pub struct MarloweSyncModule{
    pub (crate) state: Arc<MarloweState>,
    pub (crate) tip : crate::pallas_network::miniprotocols::chainsync::Tip,
    pub (crate) evt_main_chan : tokio::sync::mpsc::UnboundedSender<MarloweEvent>
}

impl MarloweSyncModule {

    pub fn new(state:Arc<MarloweState>,magic:u64) -> Self {
        
        info!("Marlowe Sync Worker Initialized");

        let (sender,receiver) = tokio::sync::mpsc::unbounded_channel();

        let state_clone = state.clone();
        tokio::spawn(async move {
            MarloweSyncModule::process_messages(receiver, state_clone,magic).await;
        });


        MarloweSyncModule { 
            state,
            tip:crate::pallas_network::miniprotocols::chainsync::Tip(crate::pallas_network::miniprotocols::Point::Origin,0),
            evt_main_chan: sender
        }
    }

    // This is the message pre-processor.
    // It offloads the worker thread from having to do this processing, and it ensures we only need to do it
    // once, even though there may be many subscribers.
    async fn process_messages(mut receiver: UnboundedReceiver<MarloweEvent>, state: Arc<MarloweState>,magic:u64) {

        while let Some(event) = receiver.recv().await {
            
            // if nobody is subscribed, we do not need to do anything here
            if state.broadcast_tx.0.receiver_count() == 0 {
                continue
            }

            // 0 INIT, 1 UPD, 2 CLOSE
            let (contract,typ) = match &event {
                MarloweEvent::Add(c) => (c.clone(),MarloweSubscriptionEventType::Init),
                MarloweEvent::Update(c) => (c.clone(),MarloweSubscriptionEventType::Update),
                MarloweEvent::Close(c) => (c.clone(),MarloweSubscriptionEventType::Closed),
            };

            let most_recent_tx = contract.transitions.last().expect("there must always exist at least one transition.");

            let machine_state = if let Some(d) = &most_recent_tx.datum {

                let posix_time_at_position = slot_to_posix_time_with_specific_magic(most_recent_tx.abs_slot.u64(), magic) ;

                let instance = marlowe_lang::semantics::ContractInstance::from_datum(&d).with_current_time(posix_time_at_position);
                if let Ok((_a,b)) = instance.step(false) {
                    b
                } else {
                    tracing::warn!("Failed to pre-process machine_state for contract {}",contract.id);
                    continue
                }
            } else {

                let message = format!("A new transition was added to this contract, but no datum was attached.");
                _ = state.broadcast_tx.0.send(
                    MarloweSubscriptionEvent::new(typ, &contract,message,None)
                );
                continue

            };

            let message = format!("Things changed!");
            _ = state.broadcast_tx.0.send(
                MarloweSubscriptionEvent::new(typ, &contract,message, Some(machine_state))
            );

        }

    }


}











#[derive(Clone,Enum,Eq,PartialEq,Copy)]
pub enum MarloweSubscriptionEventType {
    Init,
    Update,
    Closed
}


pub struct WrappedMachineState(MachineState);

#[async_graphql::Object]
impl WrappedMachineState {
    async fn state(&self) -> Result<String,String> {
        Ok(serde_json::to_string_pretty(&self.0).map_err(|e|format!("{e:?}"))?)
    }
}

impl Clone for WrappedMachineState {
    fn clone(&self) -> Self {
        match &self.0 {
            MachineState::Closed => Self(MachineState::Closed),
            MachineState::Faulted(s) => Self(MachineState::Faulted(s.to_string())),
            MachineState::ContractHasTimedOut => Self(MachineState::ContractHasTimedOut),
            MachineState::WaitingForInput { expected, timeout } => {
                Self(MachineState::WaitingForInput {
                    expected: expected.into_iter().map(clone_from_ref).map(|x|x.0).collect(),
                    timeout: timeout.clone()
                })
            },
            MachineState::ReadyForNextStep => {
                Self(MachineState::ReadyForNextStep)
            },
        }
    }
}

pub struct WrappedExpectedInput(marlowe_lang::semantics::ExpectedInput);

fn clone_from_ref(ein:&marlowe_lang::semantics::ExpectedInput) -> WrappedExpectedInput {
    match ein {
        marlowe_lang::semantics::ExpectedInput::Deposit { 
            who_is_expected_to_pay, 
            expected_asset_type, 
            expected_amount, 
            expected_payee, 
            continuation 
        } => WrappedExpectedInput(marlowe_lang::semantics::ExpectedInput::Deposit {
            who_is_expected_to_pay: who_is_expected_to_pay.clone() ,
            expected_asset_type: expected_asset_type.clone() ,
            expected_amount: expected_amount.clone() , 
            expected_payee: expected_payee.clone() ,
            continuation: continuation.clone() 
        }),
        marlowe_lang::semantics::ExpectedInput::Choice { 
            choice_name, 
            who_is_allowed_to_make_the_choice, 
            bounds, 
            continuation 
        } => WrappedExpectedInput(marlowe_lang::semantics::ExpectedInput::Choice {
            choice_name: choice_name.clone(), 
            who_is_allowed_to_make_the_choice: who_is_allowed_to_make_the_choice.clone(), 
            bounds:bounds.clone(), 
            continuation:continuation.clone()
        }),
        marlowe_lang::semantics::ExpectedInput::Notify { 
            obs, 
            continuation 
        } => WrappedExpectedInput(marlowe_lang::semantics::ExpectedInput::Notify {obs: obs.clone(), continuation: continuation.clone()}),
    }
}
impl Clone for WrappedExpectedInput {
    fn clone(&self) -> Self {
        Self(clone_from_ref(&self.0).0)
    }
}

#[derive(Clone,SimpleObject)]
pub struct MarloweSubscriptionEvent {
    pub contract_id : String,
    pub contract_short_id : String,    
    pub evt_type : MarloweSubscriptionEventType,
    pub message : String,
    pub machine_state: Option<WrappedMachineState>
}

impl MarloweSubscriptionEvent {
    pub fn new(event_type:MarloweSubscriptionEventType,contract:&Contract,message:String,machine_state:Option<MachineState>) -> Self {
        Self {
            contract_id : contract.id.to_string(),
            contract_short_id : contract.short_id.to_string(),    
            evt_type : event_type,
            message,
            machine_state: if let Some(m) = machine_state { Some(WrappedMachineState(m)) } else { None }
        }
    }
}