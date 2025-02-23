use super::scalars::U64;
use crate::service::Config;
use async_graphql::{Context, Object};

pub struct NodeInfo {
    utxo_validation: bool,
    predicates: bool,
    vm_backtrace: bool,
    min_gas_price: U64,
    min_byte_price: U64,
    max_tx: U64,
    max_depth: U64,
    node_version: String,
}

#[Object]
impl NodeInfo {
    async fn utxo_validation(&self) -> bool {
        self.utxo_validation
    }

    async fn predicates(&self) -> bool {
        self.predicates
    }

    async fn vm_backtrace(&self) -> bool {
        self.vm_backtrace
    }

    async fn min_gas_price(&self) -> U64 {
        self.min_gas_price
    }

    async fn min_byte_price(&self) -> U64 {
        self.min_byte_price
    }

    async fn max_tx(&self) -> U64 {
        self.max_tx
    }

    async fn max_depth(&self) -> U64 {
        self.max_depth
    }

    async fn node_version(&self) -> String {
        self.node_version.to_owned()
    }
}

#[derive(Default)]
pub struct NodeQuery {}

#[Object]
impl NodeQuery {
    async fn node_info(&self, ctx: &Context<'_>) -> async_graphql::Result<NodeInfo> {
        let Config {
            utxo_validation,
            predicates,
            vm,
            tx_pool_config,
            ..
        } = ctx.data_unchecked::<Config>();

        const VERSION: &str = env!("CARGO_PKG_VERSION");

        Ok(NodeInfo {
            utxo_validation: *utxo_validation,
            predicates: *predicates,
            vm_backtrace: vm.backtrace,
            min_gas_price: tx_pool_config.min_gas_price.into(),
            min_byte_price: tx_pool_config.min_byte_price.into(),
            max_tx: (tx_pool_config.max_tx as u64).into(),
            max_depth: (tx_pool_config.max_depth as u64).into(),
            node_version: VERSION.to_owned(),
        })
    }
}
