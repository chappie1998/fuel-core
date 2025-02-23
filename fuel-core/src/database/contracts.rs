use crate::{
    database::{
        columns::{BALANCES, CONTRACTS, CONTRACT_UTXO_ID},
        Database,
    },
    state::{Error, IterDirection, MultiKey},
};
use fuel_tx::UtxoId;
use fuel_types::Word;
use fuel_vm::prelude::{AssetId, Contract, ContractId, Storage};
use std::borrow::Cow;

impl Storage<ContractId, Contract> for Database {
    type Error = Error;

    fn insert(&mut self, key: &ContractId, value: &Contract) -> Result<Option<Contract>, Error> {
        Database::insert(self, key.as_ref(), CONTRACTS, value.clone())
    }

    fn remove(&mut self, key: &ContractId) -> Result<Option<Contract>, Error> {
        Database::remove(self, key.as_ref(), CONTRACTS)
    }

    fn get(&self, key: &ContractId) -> Result<Option<Cow<Contract>>, Error> {
        self.get(key.as_ref(), CONTRACTS)
    }

    fn contains_key(&self, key: &ContractId) -> Result<bool, Error> {
        self.exists(key.as_ref(), CONTRACTS)
    }
}

impl Storage<ContractId, UtxoId> for Database {
    type Error = Error;

    fn insert(&mut self, key: &ContractId, value: &UtxoId) -> Result<Option<UtxoId>, Self::Error> {
        Database::insert(self, key.as_ref(), CONTRACT_UTXO_ID, *value)
    }

    fn remove(&mut self, key: &ContractId) -> Result<Option<UtxoId>, Self::Error> {
        Database::remove(self, key.as_ref(), CONTRACT_UTXO_ID)
    }

    fn get<'a>(&'a self, key: &ContractId) -> Result<Option<Cow<'a, UtxoId>>, Self::Error> {
        self.get(key.as_ref(), CONTRACT_UTXO_ID)
    }

    fn contains_key(&self, key: &ContractId) -> Result<bool, Self::Error> {
        self.exists(key.as_ref(), CONTRACT_UTXO_ID)
    }
}

impl Database {
    pub fn contract_balances(
        &self,
        contract: ContractId,
        start_asset: Option<AssetId>,
        direction: Option<IterDirection>,
    ) -> impl Iterator<Item = Result<(AssetId, Word), Error>> + '_ {
        self.iter_all::<Vec<u8>, Word>(
            BALANCES,
            Some(contract.as_ref().to_vec()),
            start_asset.map(|asset_id| MultiKey::new((&contract, &asset_id)).as_ref().to_vec()),
            direction,
        )
        .map(|res| res.map(|(key, balance)| (AssetId::new(key[32..].try_into().unwrap()), balance)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fuel_tx::TxId;

    #[test]
    fn contract_get() {
        let contract_id: ContractId = ContractId::from([1u8; 32]);
        let contract: Contract = Contract::from(vec![32u8]);

        let database = Database::default();

        database
            .insert(contract_id.as_ref().to_vec(), CONTRACTS, contract.clone())
            .unwrap();

        assert_eq!(
            Storage::<ContractId, Contract>::get(&database, &contract_id)
                .unwrap()
                .unwrap()
                .into_owned(),
            contract
        );
    }

    #[test]
    fn contract_put() {
        let contract_id: ContractId = ContractId::from([1u8; 32]);
        let contract: Contract = Contract::from(vec![32u8]);

        let mut database = Database::default();
        Storage::<ContractId, Contract>::insert(&mut database, &contract_id, &contract).unwrap();

        let returned: Contract = database
            .get(contract_id.as_ref(), CONTRACTS)
            .unwrap()
            .unwrap();
        assert_eq!(returned, contract);
    }

    #[test]
    fn contract_remove() {
        let contract_id: ContractId = ContractId::from([1u8; 32]);
        let contract: Contract = Contract::from(vec![32u8]);

        let mut database = Database::default();
        database
            .insert(contract_id.as_ref().to_vec(), CONTRACTS, contract)
            .unwrap();

        Storage::<ContractId, Contract>::remove(&mut database, &contract_id).unwrap();

        assert!(!database.exists(contract_id.as_ref(), CONTRACTS).unwrap());
    }

    #[test]
    fn contract_exists() {
        let contract_id: ContractId = ContractId::from([1u8; 32]);
        let contract: Contract = Contract::from(vec![32u8]);

        let database = Database::default();
        database
            .insert(contract_id.as_ref().to_vec(), CONTRACTS, contract)
            .unwrap();

        assert!(Storage::<ContractId, Contract>::contains_key(&database, &contract_id).unwrap());
    }

    #[test]
    fn contract_utxo_id_get() {
        let contract_id: ContractId = ContractId::from([1u8; 32]);
        let utxo_id: UtxoId = UtxoId::new(TxId::new([2u8; 32]), 4);

        let database = Database::default();

        database
            .insert(contract_id.as_ref().to_vec(), CONTRACT_UTXO_ID, utxo_id)
            .unwrap();

        assert_eq!(
            Storage::<ContractId, UtxoId>::get(&database, &contract_id)
                .unwrap()
                .unwrap()
                .into_owned(),
            utxo_id
        );
    }

    #[test]
    fn contract_utxo_id_put() {
        let contract_id: ContractId = ContractId::from([1u8; 32]);
        let utxo_id: UtxoId = UtxoId::new(TxId::new([2u8; 32]), 4);

        let mut database = Database::default();
        Storage::<ContractId, UtxoId>::insert(&mut database, &contract_id, &utxo_id).unwrap();

        let returned: UtxoId = database
            .get(contract_id.as_ref(), CONTRACT_UTXO_ID)
            .unwrap()
            .unwrap();
        assert_eq!(returned, utxo_id);
    }

    #[test]
    fn contract_utxo_id_remove() {
        let contract_id: ContractId = ContractId::from([1u8; 32]);
        let utxo_id: UtxoId = UtxoId::new(TxId::new([2u8; 32]), 4);

        let mut database = Database::default();
        database
            .insert(contract_id.as_ref().to_vec(), CONTRACT_UTXO_ID, utxo_id)
            .unwrap();

        Storage::<ContractId, UtxoId>::remove(&mut database, &contract_id).unwrap();

        assert!(!database
            .exists(contract_id.as_ref(), CONTRACT_UTXO_ID)
            .unwrap());
    }

    #[test]
    fn contract_utxo_id_exists() {
        let contract_id: ContractId = ContractId::from([1u8; 32]);
        let utxo_id: UtxoId = UtxoId::new(TxId::new([2u8; 32]), 4);

        let database = Database::default();
        database
            .insert(contract_id.as_ref().to_vec(), CONTRACT_UTXO_ID, utxo_id)
            .unwrap();

        assert!(Storage::<ContractId, UtxoId>::contains_key(&database, &contract_id).unwrap());
    }
}
