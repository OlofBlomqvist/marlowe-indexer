use crate::graphql::types::*;
use async_graphql::connection::{Connection, Edge};
use crate::state::{Contract, OrderedContracts};

use crate::graphql::filters::*;

#[derive(Debug, PartialEq)]
pub enum QueryError {
    Generic(String),
    BeforeNotFound,
    AfterNotFound,
    InvalidPagination,
    NoIndexedContracts,
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

pub(crate) async fn contracts_query_base(contracts: &OrderedContracts, params: QueryParams) -> std::result::Result<Connection<String, Contract, ConnectionFields>, QueryError> {
    
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

            // Filter by ID
            if !super::filters::str_check_opt(&f.id, &x.id) { return false };

            // Filter by Short_Id
            if !super::filters::str_check_opt(&f.short_id, &x.short_id) { return false };

            // Filter by number of bounds
            if !super::filters::bound_values(&f.number_of_bound_values, x) { return false };
            
            // Filter by validator
            if !str_check_opt(&f.validator_hash,& x.validator_hash ) {
                return false;
            }
            
            // Filter by contract CLOSE'd state
            if let Some(filter_for_closed) = f.is_closed {
                let last_seen_transition = x.transitions.last().expect("There must always be at least one transition in a contract");
                if filter_for_closed != last_seen_transition.end {
                    // end means there will never exist a new transition in this contract - eg. it is completely closed.
                    return false;
                }
            }

            // LOCKED AMOUNTS FILTER (if any account matches the filter, this contract should be included)
            if let Some(fla) = &f.account_state {
                if !super::filters::locked_funds(fla,x) { return false };                
            }    
            

            // Filter by meta-data
            if !super::filters::meta(&f.meta_data, x) {
                return false
            }
            
            // Filter by datum (only with debug feature)
            #[cfg(feature="debug")]
            if !super::filters::datum(&f.datum, x) {
                return false
            }
            
            // Filter by marlowe_rs_test (only with debug feature)
            #[cfg(feature="debug")]
            if let Some(rstest) = &f.marlowe_rs_status {
                // this MUST be done inside the let check so we dont run marlowe_rs_test when the filter is none
                if !str_check(&rstest,& crate::graphql::resolvers::contract_debug::marlowe_rs_test(x) ) {
                    return false;
                }            
            }

            true
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
