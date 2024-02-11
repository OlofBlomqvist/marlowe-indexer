subscription {
  syncStatus {
    timeAtSyncTip
    tipAbsSlot
    localTipAbsSlot
    
    tipBlockNum
    syncTipBlockNum
    
    diff
    marloweContractCount
    timeAtSyncTip
    percentageSynced
    estimatedTimeUntilFullySynced
  }
}

subscription {
  event(filter: {
    closings:true,
    inits: true,
    updates: true
  }) {
    evtType
    contractShortId
    contractId
    message
    machineState {
      state 
    }
  }
}