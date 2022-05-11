use std::{
    cmp::{max, min},
    sync::Arc,
    time::Duration,
};

use crate::{
    log::EthEventLog,
    pending_events::{PendingDiff, PendingEvents},
    validator_set::CurrentValidatorSet,
    Config,
};
use futures::Future;
use tokio::sync::{broadcast, mpsc};
use tracing::{error, info, trace, warn};

use anyhow::Error;
use ethers_core::types::{BlockId, Filter, Log, TxHash, ValueOrArray, H256};
use ethers_providers::{
    FilterWatcher, Http, Middleware, Provider, ProviderError, StreamExt, SyncingStatus, Ws,
};
use fuel_core_interfaces::{
    block_importer::NewBlockEvent,
    model::DaBlockHeight,
    relayer::{RelayerDb, RelayerError, RelayerEvent, RelayerStatus},
};
use url::Url;

const REPORT_PROGRESS_EVERY_N_BLOCKS: u64 = 500;
const PROVIDER_INTERVAL: u64 = 1000;
pub struct Relayer {
    /// Pending stakes/assets/withdrawals. Before they are finalized
    pending: PendingEvents,
    /// Current validator set
    current_validator_set: CurrentValidatorSet,
    /// db connector to apply stake and token deposit
    db: Box<dyn RelayerDb>,
    /// Relayer Configuration
    config: Config,
    /// state of relayer
    status: RelayerStatus,
    /// new fuel block notifier.
    requests: mpsc::Receiver<RelayerEvent>,
    /// Notification of new block event
    fuel_block_importer: broadcast::Receiver<NewBlockEvent>,
}

impl Relayer {
    pub async fn new(
        config: Config,
        db: Box<dyn RelayerDb>,
        requests: mpsc::Receiver<RelayerEvent>,
        fuel_block_importer: broadcast::Receiver<NewBlockEvent>,
    ) -> Self {
        let private_key =
            hex::decode("79afbf7147841fca72b45a1978dd7669470ba67abbe5c220062924380c9c364b")
                .unwrap();

        let last_commited_finalized_fuel_height =
            db.get_last_commited_finalized_fuel_height().await;

        let pending = PendingEvents::new(
            config.eth_chain_id,
            config.eth_v2_block_commit_contract,
            private_key,
            last_commited_finalized_fuel_height,
        );

        Self {
            config,
            db,
            pending,
            current_validator_set: CurrentValidatorSet::default(),
            status: RelayerStatus::DaClientIsSyncing,
            requests,
            fuel_block_importer,
        }
    }

    /// create provider that we use for communication with ethereum.
    pub async fn provider(uri: &str) -> Result<Provider<Ws>, Error> {
        let ws = Ws::connect(uri).await?;
        let provider =
            Provider::new(ws).interval(std::time::Duration::from_millis(PROVIDER_INTERVAL));
        Ok(provider)
    }

    pub fn provider_http(uri: &str) -> Result<Provider<Http>, Error> {
        let url = Url::parse(uri).unwrap();
        let ws = Http::new(url);
        let provider =
            Provider::new(ws).interval(std::time::Duration::from_millis(PROVIDER_INTERVAL));
        Ok(provider)
    }

    async fn stop_handle<T: Future>(
        &mut self,
        handle: impl Fn() -> T,
    ) -> Result<T::Output, RelayerError> {
        loop {
            tokio::select! {
                biased;
                inner_fuel_event = self.requests.recv() => {
                    tracing::info!("Received event in stop handle:{:?}", inner_fuel_event);
                    match inner_fuel_event {
                        Some(RelayerEvent::Stop) | None =>{
                            self.status = RelayerStatus::Stop;
                            return Err(RelayerError::Stopped);
                        },
                        Some(RelayerEvent::GetValidatorSet {response_channel, .. }) => {
                            let _ = response_channel.send(Err(RelayerError::ValidatorSetEthClientSyncing));
                        },
                        Some(RelayerEvent::GetStatus { response }) => {
                            let _ = response.send(self.status);
                        },
                    }
                }
                o = handle() => {
                    return Ok(o)
                }
            }
        }
    }

    /// Initial syncing from ethereum logs into fuel database. It does overlapping syncronization and returns
    /// logs watcher with assurence that we didnt miss any events.
    #[tracing::instrument(skip_all)]
    async fn initial_sync<'a, P>(
        &mut self,
        provider: &'a P,
    ) -> Result<
        (
            u64,
            FilterWatcher<'a, P::Provider, TxHash>,
            FilterWatcher<'a, P::Provider, Log>,
        ),
        Error,
    >
    where
        P: Middleware<Error = ProviderError>,
    {
        info!("initial sync");
        // loop and wait for eth client to finish syncing
        loop {
            if matches!(
                self.stop_handle(|| provider.syncing()).await??,
                SyncingStatus::IsFalse
            ) {
                break;
            }
            let wait = self.config.eth_initial_sync_refresh();
            self.stop_handle(|| tokio::time::sleep(wait)).await?;
        }

        info!("eth client is synced");

        let last_finalized_da_height = std::cmp::max(
            self.config.eth_v2_contract_deployment(),
            self.db.get_finalized_da_height().await,
        );
        // should be allways more then last finalized_da_heights

        let best_finalized_block =
            provider.get_block_number().await?.as_u64() - self.config.eth_finality_period();

        // 1. sync from HardCoddedContractCreatingBlock->BestEthBlock-100)
        let step = self.config.initial_sync_step(); // do some stats on optimal value
        let contracts = self.config.eth_v2_contract_addresses().to_vec();
        // on start of contract there is possibility of them being overlapping, so we want to skip for loop
        // with next line
        let best_finalized_block = max(last_finalized_da_height, best_finalized_block);
        info!(
            "get logs from:{} to best finalized block:{}",
            last_finalized_da_height, best_finalized_block
        );

        for start in (last_finalized_da_height..best_finalized_block).step_by(step) {
            let end = min(start + step as u64, best_finalized_block);
            if (start - last_finalized_da_height) % REPORT_PROGRESS_EVERY_N_BLOCKS == 0 {
                info!("geting log from:{}", start);
            }

            // TODO  can be parallelized
            let filter = Filter::new()
                .from_block(start)
                .to_block(end)
                .address(ValueOrArray::Array(contracts.clone()));
            let logs = self.stop_handle(|| provider.get_logs(&filter)).await??;

            for eth_event in logs {
                let fuel_event = EthEventLog::try_from(&eth_event);
                if let Err(_err) = fuel_event {
                    // not formated event from contract
                    // just skip it for now.
                    continue;
                }
                let fuel_event = fuel_event.unwrap();
                self.pending
                    .append_eth_events(&fuel_event, eth_event.block_number.unwrap().as_u64())
                    .await;
            }
            // we are sending dummy eth block bcs we are sure that it is finalized
            self.pending.commit_diffs(self.db.as_mut(), end).await;

            self.current_validator_set
                .bump_validators_to_da_height(end, self.db.as_mut())
                .await;
        }

        // TODO probably not needed now. but after some time we will need to do sync to best block here.
        // it depends on how much time it is needed to tranverse first part of this function
        // and how much lag happened in meantime.

        let mut watchers: Option<(FilterWatcher<_, _>, FilterWatcher<_, _>)>;
        let last_included_block = best_finalized_block;

        let mut best_block;

        loop {
            // 1. get best block and its hash sync over it, and push it over
            self.pending.clear();
            //sself.pending.push_back(last_diff.clone());

            best_block = self.stop_handle(|| provider.get_block_number()).await??;
            // there is not get block latest from ethers so we need to do it in two steps to get hash

            let block = self
                .stop_handle(|| provider.get_block(best_block))
                .await??
                .ok_or(RelayerError::InitialSyncAskedForUnknownBlock)?;
            let best_block_hash = block.hash.unwrap(); // it is okay to unwrap

            // 2. sync overlap from LastIncludedEthBlock-> BestEthBlock) they are saved in dequeue.
            let filter = Filter::new()
                .from_block(last_included_block)
                .to_block(best_block)
                .address(ValueOrArray::Array(contracts.clone()));

            let logs = self.stop_handle(|| provider.get_logs(&filter)).await??;

            for eth_event in logs {
                let fuel_event = EthEventLog::try_from(&eth_event);
                if let Err(err) = fuel_event {
                    // not formated event from contract
                    error!(target:"relayer", "Eth Event not formated properly in inital sync:{} log:{:?}",err,eth_event);
                    // just skip it for now.
                    continue;
                }
                let fuel_event = fuel_event.unwrap();
                self.pending
                    .append_eth_events(&fuel_event, eth_event.block_number.unwrap().as_u64())
                    .await;
            }

            // 3. Start listening to eth events
            let eth_log_filter = Filter::new().address(ValueOrArray::Array(contracts.clone()));
            watchers = Some((
                self.stop_handle(|| provider.watch_blocks()).await??,
                self.stop_handle(|| provider.watch(&eth_log_filter))
                    .await??,
            ));

            //let t = watcher.as_mut().expect(()).next().await;
            // sleep for 50ms just to be sure that our watcher is registered and started receiving events
            tokio::time::sleep(Duration::from_millis(50)).await;

            // 4. Check if our LastIncludedEthBlock is same as BestEthBlock
            if best_block == provider.get_block_number().await?
                && best_block_hash
                    == self
                        .stop_handle(|| provider.get_block(best_block))
                        .await??
                        .ok_or(RelayerError::InitialSyncAskedForUnknownBlock)?
                        .hash
                        .unwrap()
            {
                // block number and hash are same as before starting watcher over logs.
                // we are safe to continue.
                break;
            }
            // If not the same, stop listening to events and do 2,3,4 steps again.
            // empty pending and do overlaping sync again.
            // Assume this will not happen very often.
        }

        // 5. Continue to active listen on eth events. and prune(commit to db) dequeue for older finalized events
        let finalized_da_height = best_block.as_u64() - self.config.eth_finality_period();
        self.pending
            .commit_diffs(self.db.as_mut(), finalized_da_height)
            .await;
        self.current_validator_set
            .bump_validators_to_da_height(finalized_da_height, self.db.as_mut())
            .await;

        watchers
            .map(|(w1, w2)| (best_block.as_u64(), w1, w2))
            .ok_or_else(|| RelayerError::ProviderError.into())
    }

    /// Starting point of relayer
    #[tracing::instrument(name = "relayer", skip_all)]
    pub async fn run<P>(mut self, provider: Arc<P>)
    where
        P: Middleware<Error = ProviderError> + 'static,
    {
        //let mut this = self; h
        self.current_validator_set
            .load_get_validators(self.db.as_mut())
            .await;

        loop {
            // initial sync
            let (best_block, mut blocks_watcher, mut logs_watcher) =
                match self.initial_sync(&provider).await {
                    Ok(watcher) => watcher,
                    Err(err) => {
                        if self.status == RelayerStatus::Stop {
                            return;
                        }
                        error!("Error happened while doing initial sync:{:?}", err);
                        continue;
                    }
                };
            info!(
                "Initial syncing finished on block {:?}. Continue to active sync.",
                best_block
            );

            loop {
                tokio::select! {
                    inner_fuel_event = self.requests.recv() => {
                        if inner_fuel_event.is_none() {
                            error!("Inner fuel notification broke and returned err");
                            self.status = RelayerStatus::Stop;
                            return;
                        }
                        self.handle_inner_fuel_event(inner_fuel_event.unwrap()).await;
                    }
                    new_block = self.fuel_block_importer.recv() => {
                        match new_block {
                            Err(e) => {
                                error!("Unexpected error happened in relayer new block event requests:{}",e);
                                self.status = RelayerStatus::Stop;
                                return;
                            },
                            Ok(new_block) => {
                                self.handle_fuel_block_importer(new_block,&provider).await
                            },
                        }
                    }
                    eth_block_hash = blocks_watcher.next() => {
                        let _ = self.handle_eth_block_hash(&provider,eth_block_hash).await;
                    }
                    log = logs_watcher.next() => {
                        self.handle_eth_log(log).await
                    }
                }
            }
        }
    }

    #[tracing::instrument(skip(self, provider))]
    async fn handle_fuel_block_importer<P>(&mut self, new_block: NewBlockEvent, provider: &Arc<P>)
    where
        P: Middleware<Error = ProviderError> + 'static,
    {
        match new_block {
            NewBlockEvent::NewBlockCreated(created_block) => {

                trace!("received new fuel block created event");
                self.pending
                    .handle_created_fuel_block(&created_block, self.db.as_mut(), provider)
                    .await;
                
                //u64::from(created_block.header.number)
            }
            NewBlockEvent::NewBlockIncluded(new_block) => {

                trace!("received new fuel block included event");
                // assume that da_height is checked agains parent block.

                // TODO handle lagging here. compare current_db_height and finalized_db_height and send error notification
                // if we are lagging over data availability event
                self.pending.handle_fuel_block(&new_block);
                //u64::from(new_block.header.number)
            }
        }
    }

    #[tracing::instrument(skip(self))]
    async fn handle_inner_fuel_event(&mut self, inner_event: RelayerEvent) {
        match inner_event {
            RelayerEvent::Stop => {
                self.status = RelayerStatus::Stop;
            }
            RelayerEvent::GetValidatorSet {
                da_height,
                response_channel,
            } => {
                let res = match self.current_validator_set.get_validator_set(da_height) {
                    Some(set) => Ok(set),
                    None => Err(RelayerError::ProviderError),
                };
                let _ = response_channel.send(res);
            }
            RelayerEvent::GetStatus { response } => {
                let _ = response.send(self.status);
            }
        }
    }

    #[tracing::instrument(skip(self, provider, new_eth_block_hash))]
    async fn handle_eth_block_hash<P>(
        &mut self,
        provider: &P,
        new_eth_block_hash: Option<H256>,
    ) -> Result<(), Error>
    where
        P: Middleware<Error = ProviderError>,
    {
        if new_eth_block_hash.is_none() {
            return Ok(());
        }
        let block_hash = new_eth_block_hash.unwrap();
        trace!("Received new block hash:{:x?}",block_hash);
        if let Some(block) = provider.get_block(BlockId::Hash(block_hash)).await? {
            if let Some(da_height) = block.number {
                let finalized_da_height = da_height.as_u64() - self.config.eth_finality_period();

                self.pending
                    .commit_diffs(self.db.as_mut(), finalized_da_height)
                    .await;
                self.current_validator_set
                    .bump_validators_to_da_height(finalized_da_height, self.db.as_mut())
                    .await;
            } else {
                error!("Received block hash does not have block number:block: {:?}", block);
            }
        } else {
            error!("received block hash does not exist:{}",block_hash);
        }

        Ok(())
    }

    #[tracing::instrument(skip(self, eth_event))]
    async fn handle_eth_log(&mut self, eth_event: Option<Log>) {
        // new log
        let eth_event = if let Some(eth_event) = eth_event {
            eth_event
        } else {
            // TODO make proper reconnect options.
            warn!("We broke something. Set state to not eth not connected and do retry");
            return;
        };
        trace!(target:"relayer", "got new log from block:{:?}", eth_event.block_hash);
        let fuel_event = EthEventLog::try_from(&eth_event);
        if let Err(err) = fuel_event {
            warn!(target:"relayer", "Eth Event not formated properly:{}",err);
            return;
        }
        if eth_event.block_number.is_none() {
            error!(target:"relayer", "Block number not found in eth log");
            return;
        }
        let removed = eth_event.removed.unwrap_or(false);
        let block_number = eth_event.block_number.unwrap().as_u64();
        let fuel_event = fuel_event.unwrap();

        self.pending
            .handle_eth_log(fuel_event, block_number, removed)
            .await;
    }
}

#[cfg(test)]
mod test {

    use std::{sync::Arc, time::Duration};

    use async_trait::async_trait;
    use ethers_core::types::{BlockId, BlockNumber, FilterBlockOption, H256, U256, U64};
    use ethers_providers::SyncingStatus;
    use fuel_core_interfaces::relayer::RelayerEvent;
    use fuel_tx::Address;
    use tokio::sync::mpsc;

    use crate::{
        log,
        test_helpers::{relayer, MockData, MockMiddleware, TriggerHandle, TriggerType},
        Config,
    };

    #[tokio::test]
    pub async fn initial_sync_checks_pending_eth_client_and_handling_stop() {
        let config = Config {
            eth_v2_contract_deployment: 5,
            eth_initial_sync_refresh: Duration::from_millis(10),
            ..Default::default()
        };
        let (relayer, event, _) = relayer(config).await;
        let middle = MockMiddleware::default();
        middle.data.lock().await.is_syncing = SyncingStatus::IsSyncing {
            starting_block: U256::zero(),
            current_block: U256::zero(),
            highest_block: U256::zero(),
        };

        pub struct Handle {
            pub i: u64,
            pub event: mpsc::Sender<RelayerEvent>,
        }
        #[async_trait]
        impl TriggerHandle for Handle {
            async fn run<'a>(&mut self, _: &mut MockData, trigger: TriggerType<'a>) {
                if matches!(trigger, TriggerType::Syncing) {
                    self.i += 1;

                    if self.i == 3 {
                        let _ = self.event.send(RelayerEvent::Stop).await;
                        self.i += 1;
                        return;
                    }
                    if self.i == 4 {
                        panic!("Something is fishy. We should have stopped");
                    }
                } else {
                    panic!("Unknown trigger received");
                }
            }
        }

        middle
            .trigger_handle(Box::new(Handle { i: 0, event }))
            .await;

        relayer.run(Arc::new(middle)).await;
    }

    #[tokio::test]
    pub async fn sync_first_n_finalized_blocks() {
        let config = Config {
            eth_v2_contract_deployment: 100, // start from block 1
            eth_finality_period: 30,
            initial_sync_step: 2, // make 2 steps of 2 blocks
            ..Default::default()
        };
        let (relayer, event, _) = relayer(config).await;
        let middle = MockMiddleware::default();
        {
            let mut data = middle.data.lock().await;
            // eth finished syncing
            data.is_syncing = SyncingStatus::IsFalse;
            // best block is 4
            data.best_block.number = Some(U64([134]));
        }
        pub struct Handle {
            pub i: u64,
            pub event: mpsc::Sender<RelayerEvent>,
        }
        #[async_trait]
        impl TriggerHandle for Handle {
            async fn run<'a>(&mut self, _: &mut MockData, trigger: TriggerType<'a>) {
                if let TriggerType::GetLogs(filter) = trigger {
                    if let FilterBlockOption::Range {
                        from_block,
                        to_block,
                    } = filter.block_option
                    {
                        assert_eq!(
                            from_block,
                            Some(BlockNumber::Number(U64([100 + self.i * 2]))),
                            "Start block not matching on i:{:?}",
                            self.i
                        );
                        assert_eq!(
                            to_block,
                            Some(BlockNumber::Number(U64([102 + self.i * 2]))),
                            "Start block not matching on i:{:?}",
                            self.i
                        );
                        self.i += 1;
                    }
                }
                if self.i == 2 {
                    let _ = self.event.send(RelayerEvent::Stop).await;
                    return;
                }
            }
        }
        middle
            .trigger_handle(Box::new(Handle { i: 0, event }))
            .await;

        relayer.run(Arc::new(middle)).await;
    }

    #[tokio::test]
    pub async fn initial_sync() {
        let config = Config {
            eth_v2_contract_deployment: 100, // start from block 1
            eth_finality_period: 30,
            initial_sync_step: 2, // make 2 steps of 2 blocks
            ..Default::default()
        };
        let (relayer, event, _) = relayer(config).await;
        let middle = MockMiddleware::default();
        {
            let mut data = middle.data.lock().await;
            // eth finished syncing
            data.is_syncing = SyncingStatus::IsFalse;
            // best block is 4
            data.best_block.number = Some(U64([134]));

            data.best_block.number = Some(U64([134]));
            data.logs_batch = vec![
                vec![log::tests::eth_log_deposit(136, Address::zeroed(), 10)], //Log::]
            ];
            data.blocks_batch = vec![vec![H256::zero()]];
        }
        pub struct Handle {
            pub i: u64,
            pub event: mpsc::Sender<RelayerEvent>,
        }
        #[async_trait]
        impl TriggerHandle for Handle {
            async fn run<'a>(&mut self, _: &mut MockData, trigger: TriggerType<'a>) {
                match self.i {
                    // check if eth client is in sync.
                    0 => assert_eq!(
                        TriggerType::Syncing,
                        trigger,
                        "We need to check if eth client is synced"
                    ),
                    // get best eth block number so that we know until when to sync
                    1 => assert_eq!(
                        TriggerType::GetBlockNumber,
                        trigger,
                        "We need to get Best eth block number"
                    ),
                    // get first batch of logs.
                    2 => match trigger {
                        TriggerType::GetLogs(filter) => {
                            match filter.block_option {
                                FilterBlockOption::Range {
                                    from_block,
                                    to_block,
                                } => {
                                    assert_eq!(from_block, Some(BlockNumber::Number(U64([100]))));
                                    assert_eq!(to_block, Some(BlockNumber::Number(U64([102]))));
                                }
                                _ => panic!("Expect filter block option range"),
                            };
                        }
                        _ => panic!("wrong trigger:{:?} we expected get logs 1", trigger),
                    },
                    // get second batch of logs. for initialy sync
                    3 => match trigger {
                        TriggerType::GetLogs(filter) => {
                            match filter.block_option {
                                FilterBlockOption::Range {
                                    from_block,
                                    to_block,
                                } => {
                                    assert_eq!(from_block, Some(BlockNumber::Number(U64([102]))));
                                    assert_eq!(to_block, Some(BlockNumber::Number(U64([104]))));
                                }
                                _ => panic!("Expect filter block option range"),
                            };
                        }
                        _ => panic!("wrong trigger:{:?} we expected get logs 1", trigger),
                    },
                    // update our best block
                    4 => {
                        assert_eq!(
                            TriggerType::GetBlockNumber,
                            trigger,
                            "We need to get Best eth block number again"
                        )
                    }
                    // get block hash from best block number
                    5 => {
                        assert_eq!(
                            TriggerType::GetBlock(BlockId::Number(BlockNumber::Number(U64([134])))),
                            trigger,
                            "Get block hash from best block number"
                        )
                    }
                    // get block log from current finalized to best block
                    6 => match trigger {
                        TriggerType::GetLogs(filter) => {
                            match filter.block_option {
                                FilterBlockOption::Range {
                                    from_block,
                                    to_block,
                                } => {
                                    assert_eq!(from_block, Some(BlockNumber::Number(U64([104]))));
                                    assert_eq!(to_block, Some(BlockNumber::Number(U64([134]))));
                                }
                                _ => panic!("Expect filter block option range for 6"),
                            };
                        }
                        _ => panic!("wrong trigger:{:?} we expected get logs 6", trigger),
                    },
                    // get best eth block to syncornize log watcher
                    7 => {
                        assert_eq!(
                            TriggerType::GetBlockNumber,
                            trigger,
                            "We need to get Best eth block number to check that it is not changed"
                        )
                    }
                    // get best eth block hash to syncronize log watcher
                    8 => {
                        assert_eq!(
                            TriggerType::GetBlock(BlockId::Number(BlockNumber::Number(U64([134])))),
                            trigger,
                            "Get block hash from best block number to check that it is not changed"
                        )
                    }
                    _ => panic!("Unknown request, we should have finished until now"),
                }
                self.i += 1;
            }
        }
        middle
            .trigger_handle(Box::new(Handle { i: 0, event }))
            .await;

        relayer.run(Arc::new(middle)).await;
    }
}
