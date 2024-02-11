use crate::modules::marlowe::{graphql::types::*, OutputReference};
use crate::modules::marlowe::state::Contract;
use crate::modules::marlowe::graphql::filters::*;

use async_graphql::connection::{Connection, Edge};

#[derive(Debug, PartialEq)]
pub enum QueryError {
    Generic(String),
    BeforeNotFound,
    AfterNotFound,
    InvalidPagination,
    NoResult
}

impl QueryError {
    pub fn generic(msg:&str) -> Self {
        QueryError::Generic(msg.into())
    }
}

impl std::fmt::Display for QueryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let r = format!("{:?}",self);
        f.write_str(&r)
    }
}

pub(crate) async fn contracts_query_base(magic:u64,marlowe_state: &std::sync::Arc<crate::modules::marlowe::state::MarloweState>, params: QueryParams) -> std::result::Result<Connection<String, Contract, ConnectionFields>, QueryError> {
    
    
    if marlowe_state.contracts_count() == 0 {
        return Ok(Connection::with_additional_fields(
                false,
                false,
                ConnectionFields {
                    total_indexed_contracts: 0,
                    total_number_of_contracts_matching_current_filter: 0,
                    total_number_of_pages_using_current_filter: 0,
                    page_size_used_for_this_result_set: 0,
                    total_contracts_in_requested_range: 0,
                    time_taken_ms: 0.0,
                    current_page: 0.0,
                    log: vec![]
                },
            )
        )
    }

    let start_time = tokio::time::Instant::now();
    let mut log: Vec<String> = vec![];

    let resolved_page_size = match params.page_size {
        Some(n) if (5..=100_000).contains(&n) => n as usize,
        None => 50,
        _ => return Err(QueryError::generic("pageSize is allowed to be between 5 and 100"))
    } as i32;

    let def_end = marlowe_state.contracts_count();



    let start_pos = match params.after {
        Some(a) => {
            a.parse::<usize>().unwrap() + 1
        },
        None => 0,
    };
    
    let end_pos = match params.before {
        Some(b) => {
            b.parse::<usize>().unwrap()
        },
        None => def_end,
    };
    
    
    if start_pos > end_pos {
        return Err(QueryError::generic("Invalid filter: 'after' and 'before' are in the wrong order."))
    }


    // TODO: it is possible to get a range between two keys (as in the min shortid and max shortid)
    
    let all_keys_ever : Vec<String> = marlowe_state.all_indexed_contract_ids().collect();
    let all_keys = all_keys_ever.get(start_pos..end_pos).unwrap();
    
    // TODO: do we really need to keep this log anymore ?
    log.push(format!("Using range: {} to {}", start_pos, end_pos));
    
    //let contracts_in_order_guard = contracts.contracts_in_order.read().await;
    let total_indexed_contracts_len = marlowe_state.contracts_count(); //contracts_in_order_guard.len();

    let mut sliced_contracts = &all_keys[start_pos..end_pos];

    //tracing::trace!("THERE ARE THIS MANY TOTAL CONTRACTS: {}",total_indexed_contracts_len);
    //tracing::trace!("THERE ARE THIS MANY SLICED CONTRACTS: {}",sliced_contracts.len());



    let mut pre_filtered = vec![];
    
    let mut x_filtered_items = vec![];

    // PRE-FILTER FOR SHORT_ID FILTER
    if let Some(filter) = &params.filter {
        match &filter.short_id {
            Some(
                StringFilter::Eq(id)
            ) =>  {
                pre_filtered.push(id.to_owned());
                sliced_contracts = &pre_filtered;
            },
            _ => {},
        }
    }

    // PRE-FILTER FOR ID FILTER
    if let Some(filter) = &params.filter {
        match &filter.id {
            Some(
                StringFilter::Eq(id)
            ) =>  {
                let c = marlowe_state.get_by_long_id_from_mem_cache(id.into()).await;
                if let Some(contract) = c {
                    pre_filtered.push(contract.short_id.to_owned());
                    sliced_contracts = &pre_filtered;
                }

            },
            _ => {},
        }
    }
    
    

    for key in sliced_contracts.iter() {
        
        let mut break_if_found = false;

        if let Some(filter) = &params.filter {
            match &filter.id {
                Some(StringFilter::Eq(_)) => break_if_found = true,
                _ => {}
            }
            match &filter.short_id {
                Some(StringFilter::Eq(id)) if key != id => continue,
                Some(StringFilter::Eq(id)) if key == id => {break_if_found=true},
                Some(StringFilter::Neq(id)) if key == id => continue,
                Some(StringFilter::Contains(id)) if !id.contains(id) => continue,
                Some(StringFilter::NotContains(id)) if id.contains(id) => continue,                
                _ => {},
            }
        }

        //tracing::warn!("LOOKING FOR A CONTRACT: {}",key.clone());

        
        let x = marlowe_state.get_by_shortid_from_mem_cache(key.clone()).await.unwrap(); 
        //tracing::warn!("FOUND A CONTRACT: {}",x.short_id);

        if let Some(f) = &params.filter {

            // Filter by ID
            if !super::filters::str_check_opt(&f.id, &x.id) { continue };

            // Filter by Short_Id - this is done earlier now
            //if !super::filters::str_check_opt(&f.short_id, &x.short_id) { continue };

            // Filter by number of bounds
            if !super::filters::bound_values(&f.number_of_bound_values, &x) { continue };
            
            // Filter by validator
            if !str_check_opt(&f.validator_hash,& x.validator_hash ) {
                continue
            }
            
            // Filter by contract CLOSE'd state
            if let Some(filter_for_closed) = f.is_closed {
                let last_seen_transition = x.transitions.last().expect("There must always be at least one transition in a contract");
                if filter_for_closed != last_seen_transition.end {
                    // end means there will never exist a new transition in this contract - eg. it is completely closed.
                    continue
                }
            }

            // LOCKED AMOUNTS FILTER (if any account matches the filter, this contract should be included)            
            if let Some(fla) = &f.account_state {
                if !super::filters::locked_funds(fla,&x) {continue};                
            }    

            // Filter by meta-data
            if !super::filters::meta(&f.meta_data, &x) {
                continue
            }
            
            // Filter by datum (only with debug feature)
            #[cfg(feature="debug")]
            if !super::filters::datum(&f.datum, &x) {
                continue
            }
            
            // Filter by marlowe_rs_test (only with debug feature)
            #[cfg(feature="debug")]
            if let Some(rstest) = &f.marlowe_rs_status {
                // this MUST be done inside the let check so we dont run marlowe_rs_test when the filter is none
                if !str_check(&rstest,& super::resolvers::contract_debug::marlowe_rs_test(&x,magic) ) {
                    continue
                }            
            }

            x_filtered_items.push(x);
        } else {
            x_filtered_items.push(x);
            if break_if_found {
                break
            }
        }
    };

    let total_matching_contracts_before_first_last = x_filtered_items.len() as i32;

    if total_matching_contracts_before_first_last == 0 {
        return Err(QueryError::NoResult)
    }

    log.push(format!("Total contracts after filtering: {}", total_matching_contracts_before_first_last));



    let first_last_items = match (params.first, params.last) {
        (Some(_),Some(_)) => return Err(QueryError::Generic("cannot use both 'first' and 'last' in the same query.".into())),
        
        (Some(selected_first), _) => {
            if selected_first < 1 {
                return Err(QueryError::InvalidPagination)
            }
            let l = selected_first.min(x_filtered_items.len() as i32) as usize;
            //println!("Selected first {selected_first} resulted in slice: [..{}]", l);
            &x_filtered_items[0..l]
        },

        (_, Some(selected_last)) => {
            if selected_last < 1 {
                return Err(QueryError::InvalidPagination)
            }
            let len_of_filtered_result = x_filtered_items.len();
            if len_of_filtered_result < (selected_last as usize) {
                //println!("[selected last is greater than the result set, so we will just return all items in the slice] Selected last {selected_last} resulted in slice: [..]");
                &x_filtered_items[..]
            } else {
                let start_point = len_of_filtered_result - (selected_last as usize);
                //println!("Selected last {selected_last} resulted in slice: [{}..] ... there are {} results in total",&start_point, len_of_filtered_result);
                
                //println!("this results in a slice with len {}",my_slice.len());
                &x_filtered_items[start_point..]
            }
            
        },

        // WITHOUT SPECIFYING FIRST/LAST
        _ => &x_filtered_items[..]        
    };

    log.push(format!("Items after first/last params: {}", first_last_items.len()));

    let total_matching_contracts = first_last_items.len() as i32;
    
    // Calculate total pages
    let total_pages = (total_matching_contracts as f32 / resolved_page_size as f32).ceil() as usize;
    //println!("The result has {total_pages} pages");

    // If no page parameter is given, set to the last page, otherwise use the provided page
    let current_page: usize = match params.page {
        Some(p) => p as usize,
        None => total_pages
    };

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
            total_indexed_contracts: total_indexed_contracts_len,
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
