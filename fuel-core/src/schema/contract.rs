use crate::database::{Database, KvStoreError};
use crate::schema::scalars::Bytes32 as FuelBytes;
use crate::schema::scalars::ContractId;
use crate::schema::scalars::HexString;
use crate::schema::scalars::Salt;
use async_graphql::{Context, Object};
use fuel_storage::{MerkleStorage, Storage};
use fuel_types::{Bytes32, ContractId as FuelContractId};
use fuel_vm::prelude::Contract as FuelVmContract;

pub struct Contract(pub(crate) fuel_types::ContractId);

impl From<fuel_types::ContractId> for Contract {
    fn from(id: fuel_types::ContractId) -> Self {
        Self(id)
    }
}

#[Object]
impl Contract {
    async fn id(&self) -> ContractId {
        self.0.into()
    }

    async fn bytecode(&self, ctx: &Context<'_>) -> async_graphql::Result<HexString> {
        let db = ctx.data_unchecked::<Database>().clone();
        let contract = Storage::<fuel_types::ContractId, FuelVmContract>::get(&db, &self.0)?
            .ok_or(KvStoreError::NotFound)?
            .into_owned();
        Ok(HexString(contract.into()))
    }

    async fn salt(&self, ctx: &Context<'_>) -> async_graphql::Result<Salt> {
        let contract_id = self.0;

        let db = ctx.data_unchecked::<Database>().clone();
        let salt = fuel_vm::storage::InterpreterStorage::storage_contract_root(&db, &contract_id)
            .unwrap()
            .expect("Contract does not exist");

        let cleaned_salt: Salt = salt.clone().0.into();

        Ok(cleaned_salt)
    }

    async fn balances(&self, ctx: &Context<'_>) -> async_graphql::Result<FuelBytes> {
        let contract_id: FuelContractId = self.0.into(); // Would calling id be more correct?

        let db = ctx.data_unchecked::<Database>().clone();

        let balance = Bytes32::new([4; 32]);

        let result =
            MerkleStorage::<FuelContractId, Bytes32, Bytes32>::get(&db, &contract_id, &balance)
                .unwrap()
                .unwrap();

        Ok((*result).into())
    }
}

#[derive(Default)]
pub struct ContractQuery;

#[Object]
impl ContractQuery {
    async fn contract(
        &self,
        ctx: &Context<'_>,
        #[graphql(desc = "ID of the Contract")] id: ContractId,
    ) -> async_graphql::Result<Option<Contract>> {
        let id: fuel_types::ContractId = id.0;
        let db = ctx.data_unchecked::<Database>().clone();
        let contract_exists =
            Storage::<fuel_types::ContractId, FuelVmContract>::contains_key(&db, &id)?;
        if !contract_exists {
            return Ok(None);
        }
        let contract = Contract(id);
        Ok(Some(contract))
    }
}
