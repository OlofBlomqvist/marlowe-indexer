use std::sync::Arc;
pub(crate) mod marlowe;
mod address;
use async_graphql::{EmptyMutation, Schema, Object, Context, SimpleObject}; // InputObject, SchemaBuilder, dynamic};
use async_graphql_warp::GraphQLResponse;
use crate::core::lib::ChainSyncReceiver;
use chrono::{TimeZone, Utc};
use async_graphql::async_stream::stream;

use futures::Stream;

use warp::{Filter, filters::BoxedFilter, reject::Rejection};

use crate::state::GlobalState;

use self::marlowe::graphql::types::{
   MarloweQueryRoot, 
   MarloweSubscriptionRoot
};

#[derive(async_graphql::MergedSubscription, Default)]
struct Subscription(
   MarloweSubscriptionRoot, 
   GlobalSubscriptionType,
   #[cfg(feature="WIP")]AddressSubscriptionType
);

#[derive(async_graphql::MergedObject, Default)]
struct Query(
   MarloweQueryRoot, 
   GlobalQueryType,
   #[cfg(feature="WIP")]AddressQueryType
);

pub struct ModulesStates {
   marlowe_state : Arc<marlowe::state::MarloweState>,
   //address_state : Arc<address::state::AddressState>,
}

impl ModulesStates {
   pub fn new (internal_marlowe_state:marlowe::state::MarloweState) -> Self {
      ModulesStates {
         marlowe_state: Arc::new(internal_marlowe_state),
         // address_state: Arc::new(address::state::AddressState::new(
         //    // todo - use file path 
         //    address::state::PersistanceMode::FileSystem { file_path: "c:/temp/addr.db".to_string() ,
         // }, initial_slot)),
      }
   }
}


type ComboSchema = Schema<Query,EmptyMutation,Subscription>;


#[derive(Default)]
pub struct GlobalQueryType;

#[derive(Default)]
pub struct GlobalSubscriptionType;



#[derive(Clone,serde::Serialize,Debug,serde::Deserialize,SimpleObject)]
pub struct SyncStatus {
   tip_abs_slot : i64,
   local_tip_abs_slot : i64,
   diff : i64,
   marlowe_contract_count : i64,
   time_at_sync_tip : String,
   tip_block_num : i64,
   sync_tip_block_num : i64,
   percentage_synced : i64,
   estimated_time_until_fully_synced : String
}

#[Object]
impl GlobalQueryType {
   async fn stats<'ctx>(
      &self,
      ctx: &Context<'ctx>
  ) -> std::result::Result<SyncStatus,String> {
         
      let state = ctx.data::<std::sync::Arc<GlobalState<ModulesStates>>>().unwrap().clone();
         

      let magic = state.sync_state.configuration.magic;
      let marlowe_contract_count = state.sub_state.marlowe_state.contracts_count();

      let tip_abs_slot = if let Some(t) = state.sync_state.tip_abs_slot.load() { t } else { 0 };
      let tip_block_num = if let Some(t) = state.sync_state.tip_block_number.load() { t } else { 0 };
      
      let sync_tip_abs_slot = state.sync_state.current_block_abs_slot.load().unwrap_or_default();
      let sync_tip_block_num = state.sync_state.current_block_number.load().unwrap_or_default();

      let diff = tip_abs_slot.abs_diff(sync_tip_abs_slot);
      
      let time_at_current_sync_position = crate::core::lib::slot_to_posix_time_with_specific_magic(sync_tip_abs_slot, magic) as i64;
      let timestamp = Utc.timestamp_opt(time_at_current_sync_position/1000, 0).unwrap();
      
      
      let percentage_synced = (sync_tip_block_num as f64 / tip_block_num as f64) * 100.0;


      let json_info = SyncStatus {
         tip_abs_slot : tip_abs_slot as i64,
         local_tip_abs_slot : sync_tip_abs_slot as i64,
         diff : diff as i64,
         marlowe_contract_count : marlowe_contract_count as i64,
         time_at_sync_tip : timestamp.to_string(),
         tip_block_num : tip_block_num as i64,
         sync_tip_block_num : sync_tip_block_num as i64,
         percentage_synced : percentage_synced as i64,
         estimated_time_until_fully_synced: "".to_string()
         
      };

      Ok(json_info)
      

  }
}


#[async_graphql::Subscription]
impl GlobalSubscriptionType {

   
   async fn sync_status<'ctx> (&self,ctx: &Context<'ctx>) -> impl Stream<Item = SyncStatus> {
      
      let state = ctx.data::<std::sync::Arc<GlobalState<ModulesStates>>>().unwrap().clone();
      
      let mut diff_history = vec![0; 10];
      let mut last_diff = 0;
      let mut index = 0;
   

      let magic = state.sync_state.configuration.magic;
      

   
      stream! {
         loop {
            
            let marlowe_contract_count = state.sub_state.marlowe_state.contracts_count();

            let tip_abs_slot = if let Some(t) = state.sync_state.tip_abs_slot.load() { t } else { 0 };
            let tip_block_num = if let Some(t) = state.sync_state.tip_block_number.load() { t } else { 0 };
            
            let sync_tip_abs_slot = state.sync_state.current_block_abs_slot.load().unwrap_or_default();
            let sync_tip_block_num = state.sync_state.current_block_number.load().unwrap_or_default();

            let diff = tip_abs_slot.abs_diff(sync_tip_abs_slot);
            
            let diff_change = diff as isize - last_diff as isize;
            diff_history[index] = diff_change;
            index = (index + 1) % 10;
            last_diff = diff;
   
            let avg_change: f64 = diff_history.iter().map(|&x| x as f64).sum::<f64>() / 10.0;
            let estimated_time_to_sync = if avg_change != 0.0 {
               diff as f64 / avg_change.abs()
            } else {
               0.0
            };
            
            let time_at_current_sync_position = crate::core::lib::slot_to_posix_time_with_specific_magic(sync_tip_abs_slot, magic) as i64;
            let timestamp = Utc.timestamp_opt(time_at_current_sync_position/1000, 0).unwrap();
            

            let total_seconds = estimated_time_to_sync as u64;
            let days = total_seconds / (3600 * 24);
            let hours = (total_seconds % (3600 * 24)) / 3600;
            let minutes = (total_seconds % 3600) / 60;
            
            let formatted_time_left = 
               if diff < 2 {
                  format!("Fully synced!")
               } else {
                  if diff_history[9] == 0 { "estimating, please wait...".into() } else {  format!("{} days, {} hours, {} minutes", days, hours, minutes) }
               };

            let percentage_synced = (sync_tip_block_num as f64 / tip_block_num as f64) * 100.0;

            let json_info = SyncStatus {
               tip_abs_slot : tip_abs_slot as i64,
               local_tip_abs_slot : sync_tip_abs_slot as i64,
               diff : diff as i64, 
               marlowe_contract_count : marlowe_contract_count as i64,
               time_at_sync_tip : timestamp.to_string(),
               tip_block_num : tip_block_num as i64,
               sync_tip_block_num : sync_tip_block_num as i64,
               estimated_time_until_fully_synced : formatted_time_left,
               percentage_synced : percentage_synced as i64
               
            };
            
            yield json_info;
   
            futures_timer::Delay::new(std::time::Duration::from_millis(1000)).await;
         }
      }
   }
   


}


/// Creates filters and modules with all states pre-wired for you
pub fn get_all(global_state: std::sync::Arc<crate::state::GlobalState<ModulesStates>>) -> (BoxedFilter<(impl warp::Reply,)>, Vec<Box<dyn ChainSyncReceiver>>) {
   let combined_filters = combo_gql_filter(global_state.clone());
   
   let all_modules : Vec<Box<dyn ChainSyncReceiver>> =  create_module_instances(global_state.clone());

   (combined_filters,all_modules)
}

fn create_module_instances(global_state: std::sync::Arc<crate::state::GlobalState<ModulesStates>>) -> Vec<Box<dyn ChainSyncReceiver>> {
   let magic = global_state.sync_state.configuration.magic;
   vec![
      Box::new(marlowe::MarloweSyncModule::new(
         global_state.sub_state.marlowe_state.clone(), magic
      )),

      #[cfg(feature="WIP")] 
      Box::new(address::AddressModule::new(global_state.sub_state.address_state.clone()))
   ]
}


#[derive(Debug)]
struct Nope(String);

impl warp::reject::Reject for Nope {

}

impl std::fmt::Display for Nope {
   fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
       f.write_str(&self.0)
   }
}

fn combo_gql_filter(global_state: std::sync::Arc<crate::state::GlobalState<ModulesStates>>) -> BoxedFilter<(impl warp::Reply,)> {

   //let clonely = global_state.clone();
   //let with_state = warp::any().map(move || clonely.clone());
   // async fn some_filter<T>(reply:T,state: std::sync::Arc<crate::state::GlobalState<ModulesStates>>) -> Result<T, Rejection> {
   //    if true {
   //       let j = json!({
   //          "message": "boo!"            
   //       }).to_string();

   //       Err(warp::reject::custom(Nope(j)))
   //    } else {
   //       Ok(reply)
   //    }
   // }

   async fn custom_recover(rej: warp::Rejection) -> Result<impl warp::Reply, warp::Rejection> {
      
      if let Some(Nope(x)) = rej.find::<Nope>() {
          Ok(warp::reply::with_status(
              x.to_string(),
              warp::http::StatusCode::OK,
          ))
      } else {
          Err(rej)
      }
   }

   let schema_builder = 
      Schema::build(
         Query::default(), 
         EmptyMutation, 
         Subscription::default()
      )
      .data(global_state.clone())
      .data(global_state.sub_state.marlowe_state.clone());
      //.data(global_state.sub_state.address_state.clone());
   
   //let schema = create_dynamic_schema();

   let schema = schema_builder.finish();

   let query_filter = 
      async_graphql_warp::graphql(schema.clone())
      // .and(with_state.clone())
      // .and_then(async move |reply,state: std::sync::Arc<crate::state::GlobalState<ModulesStates>>|{
      //    prevent_query_if_not_fully_synced(reply,state).await
      // })
      .and_then(
         |(schema,request) : ( ComboSchema, async_graphql::Request )
         |async move {
            let resp=schema.execute(request).await;
            Ok::<GraphQLResponse, Rejection>(async_graphql_warp::GraphQLResponse::from(resp))
         }).boxed();

   
   let subscription_filter = 
      async_graphql_warp::graphql_subscription(schema.clone())
      // .and(with_state)
      // .and_then(async move |reply,state: std::sync::Arc<crate::state::GlobalState<ModulesStates>>|{
      //    prevent_query_if_not_fully_synced(reply,state).await
      // })
      .map(|reply| Box::new(reply) as Box<dyn warp::Reply>).boxed();

   query_filter.or(subscription_filter).recover(custom_recover).boxed()

}






// DYNAMIC SCHEMA STUFF
// pub fn create_dynamic_schema() -> dynamic::Schema {
   
//    use async_graphql::{dynamic::*, value, Value};

//    let query = Object::new("GlobalQueryType").field(Field::new("value", TypeRef::named_nn(TypeRef::INT), |ctx| {
//       FieldFuture::new(async move { Ok(Some(Value::from(100))) })
//    })).extends();

//    let my_input = InputObject::new("MyInput")
//       .field(InputValue::new("a", TypeRef::named_nn(TypeRef::INT)))
//       .field(InputValue::new("b", TypeRef::named_nn(TypeRef::INT)));

//    let query2 = Object::new("Query").field(
//       Field::new("add", TypeRef::named_nn(TypeRef::INT), |ctx| {
//          FieldFuture::new(async move {
//                let input = ctx.args.try_get("input")?;
//                let input = input.object()?;
//                let a = input.try_get("a")?.i64()?;
//                let b = input.try_get("b")?.i64()?;
//                Ok(Some(Value::from(a + b)))
//          })
//       })
//       .argument(InputValue::new("input", TypeRef::named_nn(my_input.type_name())))
//    );


//    let schema = Schema::build(query.type_name(), None, None)
//     .register(query)
//     .finish().unwrap();  

//    schema
   
// //