use super::*;
#[derive(Default)]
pub struct AddressSubscriptionType;


// Just a skeleton for a subscription endpoint
#[async_graphql::Subscription]
impl AddressSubscriptionType {
    async fn new_address<'ctx> (&self,_ctx: &Context<'ctx>) -> impl Stream<Item = String> {
        //let state = ctx.data::<std::sync::Arc<AddressState>>().unwrap().clone();
        stream! {
           loop {
                yield format!("mock data");
                futures_timer::Delay::new(std::time::Duration::from_secs(1)).await;
           }
        }
    }
}
