use async_graphql::*;

use tracing::{info_span, info, Instrument};
use async_graphql::{
    connection::{Connection, Edge},
    Result
};

use crate::state::{Contract, OrderedContracts};

pub struct QueryRoot;

#[derive(InputObject,Debug)]
pub struct MyFilter {
    pub id : Option<StringFilter>,
    pub short_id : Option<StringFilter>,
    pub has_issues : Option<bool>
}

#[derive(InputObject)]
pub struct Pagination {
    after: Option<String>,
    before: Option<String>,
    first: Option<i32>,
    last: Option<i32>,
}

#[derive(OneofObject,Debug)]
pub enum StringFilter {
    Eq(String),
    Neq(String),
    Contains(String),
    NotContains(String)
}

#[derive(OneofObject)]
pub enum NumFilter {
    Eq(f64),
    Gt(f64),
    Lt(f64),
    Lte(f64),
    Gte(f64)
}

/// Additional fields to attach to the connection
#[derive(SimpleObject)]
pub struct ConnectionFields {
    pub(crate) total_indexed_contracts: usize,
    pub(crate) total_number_of_contracts_matching_current_filter: usize,
    pub(crate) total_number_of_pages_using_current_filter: usize,
    pub(crate) page_size_used_for_this_result_set: usize,
    pub(crate) total_contracts_in_requested_range: usize,
    pub(crate) time_taken_ms : f64,
    pub(crate) current_page : f64,
    pub(crate) log : Vec<String>
}

use std::time::Duration;
use futures_util::Stream;

// TODO: Add statistics subscription endpoint
pub struct SubscriptionRoot;
#[Subscription]
impl SubscriptionRoot {
    
    async fn interval(&self, #[graphql(default = 1)] n: i32) -> impl Stream<Item = i32> {
        let mut value = 0;
        async_stream::stream! {
            loop {
                futures_timer::Delay::new(Duration::from_secs(1)).await;
                value += n;
                yield value;
            }
        }
    }
}

#[Object]
impl QueryRoot {

    // TODO: Add some actual statistics.
    // TODO: Make 
    #[tracing::instrument(skip_all)]
    async fn stats<'ctx>(
        &self,
        ctx: &Context<'ctx>
    ) -> Result<String> {
        
        let my_context = ctx.data::<std::sync::Arc<tokio::sync::RwLock<crate::state::State>>>().unwrap();
        let read_guard = my_context.read().await;
        let s = format!("{:?}",read_guard.ordered_contracts.utxo_to_contract_lookup_table);
        
        Ok(s)
    }

    // todo: filter on sub fields ? ie, not in the main filter
    #[tracing::instrument(skip_all)]
    async fn contracts<'ctx>(
        &self,
        ctx: &Context<'ctx>,
        filter: Option<MyFilter>, // APPLIED AFTER HAVING SLICED THE DATA USING "AFTER" AND "BEFORE"
        pagination: Option<Pagination>,
        page_size: Option<u32>, // DEFAULTS TO 50
        page: Option<f64> // DEFAULTS TO THE LAST PAGE (MOST RECENT CONTRACTS MATCHING YOUR FILTER)
    ) -> Result<Connection<String, Contract, ConnectionFields>> {

        let (first,last,before,after) = match pagination {
            Some(p) => (p.first,p.last,p.before,p.after),
            None => (None,None,None,None)
        };

        if first.is_some() && last.is_some() {
            return Err("Cannot use both 'first' and 'last' at the same time.".into());
        }
    
        if let Some(selected_page) = page {
            if selected_page < 1.0 {
                return Err("Minimum selectable page is 1".into())
            }
        }

        let my_context = ctx.data::<std::sync::Arc<tokio::sync::RwLock<crate::state::State>>>().unwrap();
        let read_guard = my_context.read().await;
        let contracts = &read_guard.ordered_contracts;
        
        info!("Handling request to list contracts");
        match testable_query(contracts,QueryParams { filter, after, before, first, last, page_size, page }).await {
            Ok(r) => Ok(r),
            Err(e) => Err(e.into()),
        }
        
    }
    
    
    #[tracing::instrument(skip_all)]
    async fn current_block<'ctx>(&self,ctx: &Context<'ctx>) -> Option<String> {
        let my_context = ctx.data::<std::sync::Arc<tokio::sync::RwLock<crate::state::State>>>().unwrap();
        my_context.read().await.last_seen_block().as_ref().map(|block_id| block_id.clone().to_string())
    }
}

pub fn create_schema() -> async_graphql::Schema<QueryRoot, async_graphql::EmptyMutation, SubscriptionRoot> {
    Schema::build(QueryRoot, EmptyMutation, SubscriptionRoot).finish()
}

#[derive(Debug)]
#[derive(Default)]
pub struct QueryParams {
    pub(crate) filter: Option<MyFilter>,
    pub(crate) after: Option<String>,
    pub(crate) before: Option<String>,
    pub(crate) first: Option<i32>,
    pub(crate) last: Option<i32>,
    pub(crate) page_size: Option<u32>,
    pub(crate) page: Option<f64>
}
impl std::fmt::Display for QueryParams {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let r = format!("{:?}",self);
        f.write_str(&r)
    }
}




#[derive(Debug, PartialEq)]
pub enum QueryError {
    Generic(String),
    BeforeNotFound,
    AfterNotFound,
    InvalidPagination,
    NoIndexedContracts,
    NoResult
}

impl std::fmt::Display for QueryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let r = format!("{:?}",self);
        f.write_str(&r)
    }
}
impl QueryError {
    pub fn generic(msg:&str) -> Self {
        QueryError::Generic(msg.into())
    }
}



pub(crate) async fn testable_query(contracts: &OrderedContracts, params: QueryParams) -> std::result::Result<Connection<String, Contract, ConnectionFields>, QueryError> {

    if contracts.contract_count() == 0 {
        return Err(QueryError::NoIndexedContracts)
    }

    let start_time = tokio::time::Instant::now();
    let mut log: Vec<String> = vec![];

    let resolved_page_size = match params.page_size {
        Some(n) if (5..=100_000).contains(&n) => n as usize,
        None => 50,
        _ => return Err(QueryError::generic("pageSize is allowed to be between 5 and 100"))
    } as i32;

    let def_end = contracts.contract_count();

    let start_pos = match params.after {
        Some(a) => {
            match contracts.lookup_table.get(&a) {
                Some(&i) => i + 1, // we dont want to include the "after" item in the result,
                None => return Err(QueryError::AfterNotFound),
            }
        },
        None => 0,
    };

    let end_pos = match params.before {
        Some(b) => {
            match contracts.lookup_table.get(&b) {
                Some(&i) => i,
                None => return Err(QueryError::BeforeNotFound),
            }
        },
        None => def_end,
    };

    if start_pos > end_pos {
        return Err(QueryError::generic("Invalid filter: 'after' and 'before' are in the wrong order."))
    }

    log.push(format!("Using range: {} to {}", start_pos, end_pos));
    let sliced_contracts = &contracts.contracts_in_order[start_pos..end_pos];
    
    let x_filtered_items: Vec<&Contract> = sliced_contracts.iter().filter(|&x| {
        if let Some(f) = &params.filter {

            let issue_filter = 
                if let Some(b) = &f.has_issues {

                    let mut has_issues = false;
                    
                    for t in &x.transitions {
                        if t.invalid {
                            has_issues = true;
                            break
                        }
                    }

                    &has_issues == b
                    
                } else {
                    true
                };

            let id_matches = match &f.id {
                Some(StringFilter::Eq(id)) => &x.id == id,
                Some(StringFilter::Neq(id)) => &x.id != id,
                Some(StringFilter::Contains(id)) => x.id.contains(id),
                Some(StringFilter::NotContains(id)) => !x.id.contains(id),
                None => true,
            };
            let short_id_matches = match &f.short_id {
                Some(StringFilter::Eq(sid)) => &x.short_id == sid,
                Some(StringFilter::Neq(sid)) => &x.short_id != sid,
                Some(StringFilter::Contains(sid)) => x.short_id.contains(sid),
                Some(StringFilter::NotContains(sid)) => !x.short_id.contains(sid),
                None => true,
            };
            short_id_matches && id_matches && issue_filter
        } else {
            true
        }
    }).collect();

    let total_matching_contracts_before_first_last = x_filtered_items.len() as i32;

    if total_matching_contracts_before_first_last == 0 {
        return Err(QueryError::NoResult)
    }

    log.push(format!("Total contracts after filtering: {}", total_matching_contracts_before_first_last));



    let first_last_items = match (params.first, params.last) {
        (Some(_),Some(_)) => return Err(QueryError::Generic("cannot use both 'first' and 'last' in the same query.".into())),
        (Some(f), _) => {
            if f < 1 {
                return Err(QueryError::InvalidPagination)
            }
            &x_filtered_items[0..f.min(x_filtered_items.len() as i32) as usize]
        },
        (_, Some(l)) => {
            if l < 1 {
                return Err(QueryError::InvalidPagination)
            }
            let s = x_filtered_items.len().saturating_sub(l as usize);
            &x_filtered_items[s..]
        },
        _ => &x_filtered_items[..],
    };

    log.push(format!("Items after first/last params: {}", first_last_items.len()));

    let total_matching_contracts = first_last_items.len() as i32;
    
    // Calculate total pages
    let total_pages = (total_matching_contracts as f32 / resolved_page_size as f32).ceil() as usize;

    // If no page parameter is given, set to the last page, otherwise use the provided page
    let current_page: usize = match params.page {
        Some(p) => p as usize,
        None => total_pages
    };

    // todo : this will be true if there are no items indexed!
    if current_page < 1 || current_page > total_pages {
        return Err(QueryError::InvalidPagination)
    }

    let (page_start, page_end) = {
        let s = (current_page - 1) * resolved_page_size as usize;
        let e = s + resolved_page_size as usize;
        (s, e)
    };

    let start = page_start.min(first_last_items.len());
    let end: usize = page_end.min(first_last_items.len());

    log.push(format!("Selected range after page: {} to {}", start, end));

    let paged_items = &first_last_items[start..end];

    let has_next_page = match (params.first, params.last) {
        (Some(_), _) => end < x_filtered_items.len(),
        (_, Some(_l)) => end < first_last_items.len(),
        _ => end < x_filtered_items.len()
    };
    

    let has_previous_page = match (params.first, params.last) {
        (Some(_), _) => start_pos != 0,
        (_, Some(_)) => start > 0,
        _ => start > 0
    };

    let elapsed_time = start_time.elapsed();
    let time_taken_ms = elapsed_time.as_millis() as f64;

    let mut connection = Connection::with_additional_fields(
        has_previous_page,
        has_next_page,
        ConnectionFields {
            total_indexed_contracts: contracts.contracts_in_order.len(),
            total_number_of_contracts_matching_current_filter: total_matching_contracts as usize,
            total_number_of_pages_using_current_filter: total_pages,
            page_size_used_for_this_result_set: resolved_page_size  as usize,
            total_contracts_in_requested_range: sliced_contracts.len(),
            time_taken_ms,
            current_page: current_page as f64,
            log
        },
    );

    connection.edges = 
        paged_items.iter().map(|node| 
            Edge::new(node.short_id.clone(), (*node).clone())
        ).collect();

    Ok(connection)
}



// ====================== NOTES =======================================================================

// ---------------------------------------------------
// :: GET SELECTED FIELDS OF QUERY
// :: So that we can create a more efficient db call?
// ---------------------------------------------------
//  
//  let fields = ctx.field().selection_set();
//  for f in fields {
//      // top level field F name:
//      _ = f.name();
//      // sub fields selected from F:
//      _ = f.selection_set() // <-- recurse
//  }

