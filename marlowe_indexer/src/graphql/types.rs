use async_graphql::*;

pub struct QueryRoot;

#[derive(InputObject,Default,Debug)]
pub struct ContractsFilter {

    /// The transaction hash and output index (hash#i) of the tx that created the contract
    pub id : Option<StringFilter>,

    /// Custom cursor id used by marlowe-indexer, based on block,tx and utxo position on chain, converted to base58.
    pub short_id : Option<StringFilter>,

    /// NOTE: This filter is only available because the debug feature is active
    #[cfg(feature="debug")] pub has_issues : Option<bool>,
    pub account_state: Option<LockedAmountFilter>,
    
    /// The contract is fully closed: no further transitions are possible.
    pub is_closed: Option<bool>,

    /// Filter by meta data attached to any tx of a contract
    pub meta_data: Option<StringFilter>,

    pub validator_hash: Option<StringFilter>,

    /// NOTE: This filter is only available because the debug feature is active
    #[cfg(feature="debug")]
    pub marlowe_rs_status: Option<StringFilter>,

    /// Filter by number of bound values in current state
    pub number_of_bound_values: Option<NumFilter>,

    /// NOTE: This filter is only available because the debug feature is active
    #[cfg(feature="debug")]
    pub datum: Option<StringFilter>,


    // ----- TODO : ADD SUPPORT FOR THESE--------------------------------------------

    /// NOTE: This filter is not yet implemented.
    pub uses_merkleization : Option<bool>,

    /// NOTE: This filter is not yet implemented.
    /// >> Find all contracts that this tx affected
    pub tx_id: Option<StringFilter>,

    /// NOTE: This filter is not yet implemented.
    pub role_token: Option<Vec<StringFilter>>,

    /// NOTE: This filter is not yet implemented.
    pub role_policy: Option<StringFilter>,

    /// NOTE: This filter is not yet implemented.
    pub participant_address: Option<Vec<StringFilter>>


}

#[derive(InputObject)]
pub struct Pagination {
    pub after: Option<String>,
    pub before: Option<String>,
    pub first: Option<i32>,
    pub last: Option<i32>,
    pub page_size: Option<u32>, // DEFAULTS TO 50
    pub page: Option<f64> // DEFAULTS TO THE LAST PAGE (MOST RECENT CONTRACTS MATCHING YOUR FILTER)
}

#[derive(OneofObject,Debug)]
pub enum StringFilter {
    Eq(String),
    Neq(String),
    Contains(String),
    NotContains(String)
}

#[derive(OneofObject,Debug)]
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


// TODO: Add statistics subscription endpoint
pub struct SubscriptionRoot;



#[derive(Default,Debug)]
pub struct QueryParams {
    pub(crate) filter: Option<ContractsFilter>,
    pub(crate) after: Option<String>,
    pub(crate) before: Option<String>,
    pub(crate) first: Option<i32>,
    pub(crate) last: Option<i32>,
    pub(crate) page_size: Option<u32>,
    pub(crate) page: Option<f64>
}

impl std::fmt::Display for QueryParams {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str(&format!("{:?}",&self))
    }
}


#[derive(OneofObject,Debug)]
pub enum AccountOwnerFilter {
    Role(StringFilter),
    Address(StringFilter)
}
#[derive(InputObject,Debug)]
pub struct LockedAmountFilter {
    pub amount : Option<NumFilter>,
    pub currency_symbol : Option<StringFilter>,
    pub token_name : Option<StringFilter>,
    pub account_owner : Option<AccountOwnerFilter>,
    pub number_of_accounts: Option<NumFilter>
}

#[derive(SimpleObject)]
pub struct LockedAmountsResult {
    pub amount : f64,
    pub currency_symbol : String,
    pub token_name : String,
    pub account_owner_role : Option<String>,
    pub account_owner_addr : Option<String>
}




#[derive(InputObject)]
pub struct TransitionFilter {

    /// Only show the most recent transition
    pub last: Option<bool>,
    
    pub end: Option<bool>,
    pub issues: Option<bool>,
    pub tx_id: Option<String>,
    pub slot: Option<f64>,
    pub block_hash: Option<String>,
    pub block_num: Option<f64>,
    pub marlowe_scan_status: Option<String>,  
    pub issues_match: Option<String>,  
      
}

#[derive(Enum,Copy,Clone,Eq,PartialEq)]
pub enum MarlowePartyType {
    Role,
    Address
}
#[derive(Enum,Copy,Clone,Eq,PartialEq)]
pub enum MarlowePayeeType {
    Account,
    Party
}

#[derive(Clone, Eq, PartialEq)]
pub struct MarlowePayee {
    pub typ : MarlowePartyType,
    pub value : String
}

#[derive(Clone, Eq, PartialEq)]
pub struct MarloweParty {
    pub typ : MarlowePartyType,
    pub value : String
}

#[derive(Clone, Eq, PartialEq)]
pub struct ExpectedDeposit {
    pub by : MarloweParty,
    pub to : MarlowePayee,
    pub amount : String,
    pub token : String
}

#[derive(Clone, Eq, PartialEq)]
pub struct ExpectedChoice {
    pub by : MarloweParty,
    pub bounds : String,
    pub choice_name : String
}

#[derive(Clone, Eq, PartialEq)]
pub struct ExpectedNotification {
    pub observation : String,
    pub is_currently_true : bool
}

#[derive(Clone, Eq, PartialEq)]
pub struct ExpectedInputActions {
    pub deposits : Vec<ExpectedDeposit>,
    pub choices : Vec<ExpectedChoice>,
    pub notifications : Vec<ExpectedNotification>
}

#[allow(unused)]
pub struct SlotConfig {
    pub zero_time: u64,
    pub zero_slot: u64,
    pub slot_length: u64,
}
