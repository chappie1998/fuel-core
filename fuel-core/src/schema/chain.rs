use crate::model::FuelBlockDb;
use crate::schema::block::Block;
use crate::schema::scalars::U64;
use crate::{database::Database, service::Config};
use async_graphql::{Context, Object};
use fuel_storage::Storage;

pub const DEFAULT_NAME: &str = "Fuel.testnet";

pub struct ChainInfo;

pub struct ConsensusParameters(fuel_tx::ConsensusParameters);

#[Object]
impl ConsensusParameters {
    async fn contract_max_size(&self) -> U64 {
        self.0.contract_max_size.into()
    }

    async fn max_inputs(&self) -> U64 {
        self.0.max_inputs.into()
    }

    async fn max_outputs(&self) -> U64 {
        self.0.max_outputs.into()
    }

    async fn max_witnesses(&self) -> U64 {
        self.0.max_witnesses.into()
    }

    async fn max_gas_per_tx(&self) -> U64 {
        self.0.max_gas_per_tx.into()
    }

    async fn max_script_length(&self) -> U64 {
        self.0.max_script_length.into()
    }

    async fn max_script_data_length(&self) -> U64 {
        self.0.max_script_data_length.into()
    }

    async fn max_static_contracts(&self) -> U64 {
        self.0.max_static_contracts.into()
    }

    async fn max_storage_slots(&self) -> U64 {
        self.0.max_storage_slots.into()
    }

    async fn max_predicate_length(&self) -> U64 {
        self.0.max_predicate_length.into()
    }

    async fn max_predicate_data_length(&self) -> U64 {
        self.0.max_predicate_data_length.into()
    }
}

#[Object]
impl ChainInfo {
    async fn name(&self, ctx: &Context<'_>) -> async_graphql::Result<String> {
        let db = ctx.data_unchecked::<Database>().clone();
        let name = db
            .get_chain_name()?
            .unwrap_or_else(|| DEFAULT_NAME.to_string());
        Ok(name)
    }

    async fn latest_block(&self, ctx: &Context<'_>) -> async_graphql::Result<Block> {
        let db = ctx.data_unchecked::<Database>().clone();
        let height = db.get_block_height()?.unwrap_or_default();
        let id = db.get_block_id(height)?.unwrap_or_default();
        let block = Storage::<fuel_types::Bytes32, FuelBlockDb>::get(&db, &id)?.unwrap_or_default();
        Ok(Block(block.into_owned()))
    }

    async fn base_chain_height(&self) -> U64 {
        0.into()
    }

    async fn peer_count(&self) -> u16 {
        0
    }

    async fn consensus_parameters(
        &self,
        ctx: &Context<'_>,
    ) -> async_graphql::Result<ConsensusParameters> {
        let config = ctx.data_unchecked::<Config>();

        Ok(ConsensusParameters(
            config.chain_conf.transaction_parameters,
        ))
    }
}

#[derive(Default)]
pub struct ChainQuery;

#[Object]
impl ChainQuery {
    async fn chain(&self) -> ChainInfo {
        ChainInfo
    }
}
