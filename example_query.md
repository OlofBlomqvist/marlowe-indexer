{
  stats {
    percentageSynced
    timeAtSyncTip
    tipAbsSlot
    localTipAbsSlot
    
  }
  contracts{
    totalIndexedContracts
    totalNumberOfContractsMatchingCurrentFilter
    timeTakenMs
    log
    nodes {
      id
      describe
      transitions {
        txId
        end
      }
    }
  }
}