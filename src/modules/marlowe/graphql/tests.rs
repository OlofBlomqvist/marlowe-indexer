use std::collections::HashMap;
use crate::modules::marlowe::Contract;
use crate::modules::marlowe::ContractShortId;
use crate::modules::marlowe::graphql::types::{
    ContractsFilter,
    QueryParams,
    StringFilter
};

#[cfg(test)]
async fn setup_ordered_contracts(contract_count:usize,test_name:&str) -> std::sync::Arc<crate::modules::marlowe::state::MarloweState> {
   
    let ids : Vec<usize> = (1..contract_count + 1).collect();
    let contracts : Vec<Contract> = ids.iter().map(|x| Contract {
        id: format!("id_{}", x),
        short_id: format!("short_{}", x),
        transitions: vec![],
        validator_hash: "".into()
    }).collect();

    let state = crate::modules::marlowe::state::MarloweState::new_mock(contracts,test_name);
    state.init_mem_cache().await;
    std::sync::Arc::new(state)
}

#[tokio::test]
async fn test_valid_filter() {
    
    let contracts = setup_ordered_contracts(50,"test_valid_filter").await;
    let filter = Some(ContractsFilter {
        ..Default::default()
    });
    
    let result = crate::modules::marlowe::graphql::query::contracts_query_base(42,&contracts,QueryParams {
        filter,
        ..Default::default()
    }).await;
    
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_pagination_after_and_first() {
    let contracts = setup_ordered_contracts(55,"test_pagination_after_and_first").await;;

    let result =  crate::modules::marlowe::graphql::query::contracts_query_base(42,&contracts,QueryParams {
        after: Some("short_20".to_string()),
        first: Some(5),
        ..Default::default()
    }).await;

    // [1,2,3.....  15,16,17,18,19,20,21,24,23,24,25|47,48,49,50,51,52,53,54,55]
    //                               |^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^| After applying "after 20" 
    //                               |^^^^^^^^^^^^^^| After applying "first 5"

    let r = result.as_ref().unwrap();
    assert!(&result.is_ok());
    assert_eq!(r.additional_fields.current_page,1.0); // 5 items should fit in a single page
    assert_eq!(r.additional_fields.page_size_used_for_this_result_set,50); // default page size should be 50
    assert_eq!(r.additional_fields.total_contracts_in_requested_range,35); // there should be 35 items after "id_20" ( 55-20 = 35 )
    assert_eq!(r.additional_fields.total_indexed_contracts,55); // in total we have 55 items in the db
            
    let expected_last_id = "id_25";
    let expected_first_id = "id_21";
    assert_eq!(r.edges[0].node.id, expected_first_id);
    assert_eq!(r.edges[4].node.id, expected_last_id);

    assert_eq!(&r.edges.len(), &5);

}


#[tokio::test]
async fn test_pagination_before_and_last() {
    let contracts = setup_ordered_contracts(55,"test_pagination_before_and_last").await;;

    let result =  crate::modules::marlowe::graphql::query::contracts_query_base(42,&contracts,QueryParams {
        before: Some("short_20".to_string()),
        last: Some(5),
        ..Default::default()
    }).await;

    // [1,2,3.....  15,16,17,18,19,20,21...47,48,49,50,51,52,53,54,55]
    // |^^^^^^^^^^^^^^^^^^^^^^^^^^| After applying "before 20" 
    //             |^^^^^^^^^^^^^^| After applying "last 5"

    let expected_last_id = "id_19";
    let expected_first_id = "id_15";
    assert!(&result.is_ok());
    assert_eq!(result.as_ref().unwrap().additional_fields.current_page,1.0); // 5 items should fit in a single page
    assert_eq!(result.as_ref().unwrap().additional_fields.page_size_used_for_this_result_set,50); // default page size should be 50
    assert_eq!(result.as_ref().unwrap().additional_fields.total_contracts_in_requested_range,19); // there should be 19 items before "id_20"
    assert_eq!(result.as_ref().unwrap().additional_fields.total_indexed_contracts,55); // in total we have 55 items in the db
    
    
    assert_eq!(result.as_ref().unwrap().edges[0].node.id, expected_first_id);
    assert_eq!(result.as_ref().unwrap().edges[4].node.id, expected_last_id);
    assert_eq!(&result.unwrap().edges.len(), &5);
}


#[tokio::test]
async fn test_filter_by_id() {
    let contracts = setup_ordered_contracts(50,"test_filter_by_id").await;;
    
    let filter = Some(ContractsFilter {
        id: Some(StringFilter::Eq("id_25".to_string())),
        ..Default::default()
    });

    let result =  crate::modules::marlowe::graphql::query::contracts_query_base(42,&contracts,QueryParams {
        filter,
        ..Default::default()
    }).await;
    
    match result {
        Ok(r) => {
            assert_eq!(r.edges[0].node.id, "id_25");
            assert!(!r.has_next_page);
            assert!(!r.has_previous_page);
        },
        Err(e) => panic!("{e:?}"),
    }
    
}


#[tokio::test]
async fn test_invalid_before_arg() {
    let contracts = setup_ordered_contracts(50,"test_invalid_before_arg").await;;

    let result =  crate::modules::marlowe::graphql::query::contracts_query_base(42,&contracts,QueryParams {
        before: Some("short_1000".to_string()),  // This does not exist
        ..Default::default()
    }).await;

    assert!(result.is_err());
    
    assert_eq!(result.err().unwrap(),  crate::modules::marlowe::graphql::query::QueryError::BeforeNotFound);
}

#[tokio::test]
async fn test_invalid_after_arg() {
    let contracts = setup_ordered_contracts(50,"test_invalid_after_arg").await;;

    let result =  crate::modules::marlowe::graphql::query::contracts_query_base(42,&contracts,QueryParams {
        after: Some("short_1000".to_string()),
        ..Default::default()
    }).await;

    assert!(result.is_err());
    assert_eq!(result.err().unwrap(),  crate::modules::marlowe::graphql::query::QueryError::AfterNotFound);
}

#[tokio::test]
async fn test_no_results_filter() {
    let contracts = setup_ordered_contracts(50,"test_no_results_filter").await;;
    let filter = Some(ContractsFilter {
        id: Some(StringFilter::Eq("id_1000".to_string())),
        ..Default::default()
    });

    let result =  crate::modules::marlowe::graphql::query::contracts_query_base(42,&contracts, QueryParams { filter, ..Default::default() }).await;
    match result {
        Ok(_) => panic!("should not yield any results"),
        Err(e) => assert_eq!( crate::modules::marlowe::graphql::query::QueryError::NoResult,e),
    }
}

#[tokio::test]
async fn test_negative_first_value() {
    let contracts = setup_ordered_contracts(50,"test_negative_first_value").await;;

    let result =  crate::modules::marlowe::graphql::query::contracts_query_base(42,&contracts, QueryParams { first: Some(-5), ..Default::default() }).await;

    assert!(result.is_err());
    assert_eq!(result.err().unwrap(),  crate::modules::marlowe::graphql::query::QueryError::InvalidPagination); // assuming you have this error type
}

#[tokio::test]
async fn test_multiple_filters() {
    let contracts = setup_ordered_contracts(50,"test_multiple_filters").await;;

    let filter = Some(ContractsFilter {
        id: Some(StringFilter::Eq("id_25".to_string())),
        short_id: Some(StringFilter::Eq("short_25".to_string())),
        ..Default::default()
    });

    let result =  crate::modules::marlowe::graphql::query::contracts_query_base(42,&contracts, QueryParams { filter, ..Default::default() }).await;

    assert!(result.is_ok());
    assert_eq!(result.unwrap().edges.len(), 1);
}

#[tokio::test]
async fn test_max_results() {
    let contracts = setup_ordered_contracts(50,"test_max_results").await;;

    let result =  crate::modules::marlowe::graphql::query::contracts_query_base(42,&contracts, QueryParams { first: Some(100), ..Default::default() }).await;

    assert!(result.is_ok());
    assert_eq!(result.unwrap().edges.len(), 50); // assuming 50 is the maximum
}

#[tokio::test]
async fn test_ordered_results() {
    let contracts = setup_ordered_contracts(50,"test_ordered_results").await;;

    let result =  crate::modules::marlowe::graphql::query::contracts_query_base(42,&contracts, QueryParams { ..Default::default() }).await;

    assert!(result.is_ok());
    let edges = result.unwrap().edges;
    for i in 0..edges.len() - 1 {
        let current_id_num: usize = edges[i].node.id.trim_start_matches("id_").parse().unwrap();
        let next_id_num: usize = edges[i + 1].node.id.trim_start_matches("id_").parse().unwrap();
        assert!(current_id_num < next_id_num); 
    }
    
}

#[tokio::test]
async fn test_overlap_of_before_and_after() {
    let contracts = setup_ordered_contracts(50,"test_overlap_of_before_and_after").await;;

    let result =  crate::modules::marlowe::graphql::query::contracts_query_base(42,&contracts, 
        QueryParams {
            before: Some("short_20".to_string()),
            after: Some("short_15".to_string()),
            ..Default::default()
        }).await;

    assert!(result.is_ok());
    let edges = result.unwrap().edges;
    assert_eq!(edges.first().unwrap().node.id, "id_16");
    assert_eq!(edges.last().unwrap().node.id, "id_19");
}


#[tokio::test]
async fn test_no_contracts() {
    let contracts = setup_ordered_contracts(0,"test_no_contracts").await;;
    let result =  crate::modules::marlowe::graphql::query::contracts_query_base(42,&contracts, 
        QueryParams { ..Default::default() }).await;
    match result {
        Ok(r) => assert_eq!(r.edges.len(),0),
        Err(_) => panic!("This should not fail")
    }
}

#[tokio::test]
async fn test_filter_non_existent_value() {
    let contracts = setup_ordered_contracts(50,"test_filter_non_existent_value").await;;
    let filter = Some(ContractsFilter {
        id: Some(StringFilter::Eq("id_1000".to_string())),
        ..Default::default()

    });
    let result =  crate::modules::marlowe::graphql::query::contracts_query_base(42,&contracts, 
        QueryParams { filter, ..Default::default() }).await;
        match result {
            Ok(_) => panic!("should not yield any results"),
            Err(e) => assert_eq!( crate::modules::marlowe::graphql::query::QueryError::NoResult,e),
        }
}

#[tokio::test]
async fn test_multiple_different_filters() {
    let contracts = setup_ordered_contracts(50,"test_multiple_different_filters").await;;
    let filter = Some(ContractsFilter {
        id: Some(StringFilter::Eq("id_25".to_string())),
        short_id: Some(StringFilter::Eq("short_26".to_string())),
        ..Default::default()
    });
    let result =  crate::modules::marlowe::graphql::query::contracts_query_base(42,&contracts, 
        QueryParams { filter, ..Default::default() }).await;
    match result {
        Ok(_) => panic!("should not yield any results"),
        Err(e) => assert_eq!( crate::modules::marlowe::graphql::query::QueryError::NoResult,e),
    }
    
}

#[tokio::test]
async fn test_has_next_page_false_at_end() {
    let contracts = setup_ordered_contracts(50,"test_has_next_page_false_at_end").await;;
    let result =  crate::modules::marlowe::graphql::query::contracts_query_base(42,&contracts, 
        QueryParams {..Default::default() }).await;
    assert!(result.is_ok());
    assert!(!result.unwrap().has_next_page);
}

#[tokio::test]
async fn test_has_previous_page_true() {
    let contracts = setup_ordered_contracts(500,"test_has_previous_page_true").await;;
    let result =  crate::modules::marlowe::graphql::query::contracts_query_base(42,&contracts, 
        QueryParams { page:Some(4.0), ..Default::default() }).await;
    assert!(result.is_ok());
    let r = result.unwrap();
    assert!(r.has_previous_page);
    assert!(r.has_next_page);
    assert_eq!(r.additional_fields.current_page,4.0);
    assert_eq!(r.edges[0].cursor,"short_151".to_string());
    assert_eq!(r.edges.len(),50);
    assert_eq!(r.edges[49].cursor,"short_200".to_string());
    
    assert_eq!(r.additional_fields.page_size_used_for_this_result_set,50);
    assert_eq!(r.additional_fields.total_number_of_pages_using_current_filter,500/50);
    
}


#[tokio::test]
async fn test_has_previous_page_false_at_start() {
    let contracts = setup_ordered_contracts(50,"test_has_previous_page_false_at_start").await;;
    let result =  crate::modules::marlowe::graphql::query::contracts_query_base(42,&contracts, 
        QueryParams { first: Some(50), ..Default::default() }).await;
    assert!(result.is_ok());
    assert!(!result.unwrap().has_previous_page);

}

#[tokio::test]
async fn test_invalid_combination_first_and_last() {
    let contracts = setup_ordered_contracts(50,"test_invalid_combination_first_and_last").await;;
    let result =  crate::modules::marlowe::graphql::query::contracts_query_base(42,&contracts, 
        QueryParams { 
            first: Some(5), 
            last: Some(5), 
            ..Default::default() 
        }
    ).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_full_page() {
    
    let contracts = setup_ordered_contracts(150,"test_full_page").await;;

    let result =  crate::modules::marlowe::graphql::query::contracts_query_base(42,&contracts, 
        QueryParams { page: Some(2.0), ..Default::default() }).await;

    if let Err(e) = &result {
        println!("Error: {:?}", e);
    }
    
    assert!(result.is_ok());
    let x = result.unwrap();
    assert_eq!(x.edges.len(), 50);
    assert_eq!(x.additional_fields.current_page, 2.0);
    assert_eq!(x.additional_fields.total_contracts_in_requested_range, 150);
    assert_eq!(x.additional_fields.total_indexed_contracts, 150);
    assert_eq!(x.additional_fields.total_number_of_pages_using_current_filter, 3);
    assert!(x.has_next_page);
    assert!(x.has_previous_page);
    assert_eq!(x.additional_fields.current_page, 2.0);
    assert_eq!(x.additional_fields.page_size_used_for_this_result_set, 50);
}


#[tokio::test]
async fn test_partial_page() {
    let contracts = setup_ordered_contracts(55,"test_partial_page").await;;
    let result =  crate::modules::marlowe::graphql::query::contracts_query_base(42,&contracts, 
        QueryParams { page:Some(1.0), first: Some(55), ..Default::default() }).await;
    assert!(result.is_ok());
    let x = result.unwrap();
    println!("first: {:?}",x.edges.first().unwrap().cursor);
    println!("last: {:?}",x.edges.last().unwrap().cursor);
    assert_eq!(x.edges.len(), 50);  // we selected the first page here manually
    assert!(x.has_next_page);
    assert!(!x.has_previous_page);
    assert_eq!(x.additional_fields.total_number_of_contracts_matching_current_filter, 55);
}


#[tokio::test]
async fn test_partial_page_using_latest() {
    let contracts = setup_ordered_contracts(55,"test_partial_page_using_latest").await;;
    let result =  crate::modules::marlowe::graphql::query::contracts_query_base(42,&contracts, 
        QueryParams { last: Some(55), ..Default::default() }).await;
    assert!(result.is_ok());
    let x = result.unwrap();

    // we default to returning the last page (most recent items) and since
    // the default page size is 50 we will have the first 50 oldest items on the first page
    // and the remaining 5 on the second (last) page.
    assert_eq!(x.edges.len(), 5); 

    assert_eq!(x.additional_fields.total_number_of_pages_using_current_filter,2);
    assert!(!x.has_next_page);
    assert!(x.has_previous_page);
    assert_eq!(x.additional_fields.current_page,2.0);
    assert_eq!(x.additional_fields.total_number_of_contracts_matching_current_filter, 55);
}

#[tokio::test]
async fn test_partial_page_with_specific_page_size() {
    let contracts = setup_ordered_contracts(55,"test_partial_page_with_specific_page_size").await;;
    let result =  crate::modules::marlowe::graphql::query::contracts_query_base(42,&contracts, 
        QueryParams { first: Some(55), page_size: Some(55), ..Default::default() }).await;
    assert!(result.is_ok());
    let x = result.unwrap();
    println!("LOG: {:?}",x.additional_fields.log);
    println!("first: {:?}",x.edges.first().unwrap().cursor);
    println!("last: {:?}",x.edges.last().unwrap().cursor);
    
    assert_eq!(x.edges.len(), 55);
    assert!(!x.has_next_page);
    assert!(!x.has_previous_page);
    assert_eq!(x.additional_fields.total_number_of_contracts_matching_current_filter, 55);
}

#[tokio::test]
async fn test_specific_page() {
    let contracts = setup_ordered_contracts(150,"test_specific_page").await;;
    let result =  crate::modules::marlowe::graphql::query::contracts_query_base(42,&contracts, 
        QueryParams { page: Some(3.0), ..Default::default() }).await;
    assert!(result.is_ok());
    let x = result.unwrap();
    assert_eq!(x.edges.len(), 50);
    assert!(!x.has_next_page);
    assert!(x.has_previous_page);
    assert_eq!(x.additional_fields.current_page, 3.0);
}

