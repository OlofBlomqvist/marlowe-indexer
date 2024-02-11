{
  contracts(
    filter: {
      id: {
        eq: "48f23b4f32006849a0f5ed76f32229d0f615b0bb2b79a11f329864c8b0c237ae#98",
      }
    }
  ){
    timeTakenMs
    nodes {
      id
      describe
      shortId
      transitions {
        txId
        end
      }
    }
  }
}

{
  contracts(
    filter: {
      isClosed: true,
    }
    ,
    pagination: {
      pageSize:5,
    }
   
  ){
    totalContractsInRequestedRange
    totalNumberOfPagesUsingCurrentFilter
    totalNumberOfContractsMatchingCurrentFilter
    pageSizeUsedForThisResultSet
    currentPage
    pageInfo {
      hasNextPage
      hasPreviousPage
    }
    
    timeTakenMs
    nodes {
      id
      describe
      shortId
      transitions {
        txId
        end
      }
    }
  }
}