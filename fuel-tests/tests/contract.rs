use fuel_core::{
    database::Database,
    service::{Config, FuelService},
};
use fuel_gql_client::client::FuelClient;
use fuel_storage::Storage;
use fuel_types::ContractId;
use fuel_vm::prelude::Contract as FuelVmContract;

#[tokio::test]
async fn contract() {
    // setup test data in the node
    let start_contract = FuelVmContract::default(); // Saved here to compare with returned contract via api
    let id = fuel_types::ContractId::new(Default::default());

    let mut db = Database::default();
    Storage::<fuel_types::ContractId, FuelVmContract>::insert(&mut db, &id, &start_contract)
        .unwrap();
    // setup server & client
    let srv = FuelService::from_database(db, Config::local_node())
        .await
        .unwrap();
    let client = FuelClient::from(srv.bound_address);

    // run test
    let returned_contract = client
        .contract(format!("{:#x}", id).as_str())
        .await
        .unwrap();

    assert!(returned_contract.is_some());

    // Test Id property of Contracts
    let start_id: ContractId = id.into();

    let unwrapped_return = returned_contract.unwrap();

    let returned_id: ContractId = unwrapped_return.id.into();

    assert_eq!(start_id, returned_id);
}

#[tokio::test]
async fn contract_salt() {
    let start_contract = FuelVmContract::default(); // Saved here to compare with returned contract via api
    let id = fuel_types::ContractId::new(Default::default());

    let mut db = Database::default();
    Storage::<fuel_types::ContractId, FuelVmContract>::insert(&mut db, &id, &start_contract)
        .unwrap();
    // setup server & client
    let srv = FuelService::from_database(db, Config::local_node())
        .await
        .unwrap();
    let client = FuelClient::from(srv.bound_address);

    // run test
    let returned_contract = client
        .contract(format!("{:#x}", id).as_str())
        .await
        .unwrap();

    let unwrapped_return = returned_contract.unwrap();

    let salt = unwrapped_return.salt;

    // TODO make assertions here to verify Salt if possible?
}

#[tokio::test]
async fn contract_balances() {
    let start_contract = FuelVmContract::default(); // Saved here to compare with returned contract via api
    let id = fuel_types::ContractId::new(Default::default());

    let mut db = Database::default();
    Storage::<fuel_types::ContractId, FuelVmContract>::insert(&mut db, &id, &start_contract)
        .unwrap();
    // setup server & client
    let srv = FuelService::from_database(db, Config::local_node())
        .await
        .unwrap();
    let client = FuelClient::from(srv.bound_address);

    // run test
    let returned_contract = client
        .contract(format!("{:#x}", id).as_str())
        .await
        .unwrap();

    let unwrapped_return = returned_contract.unwrap();

    let query_asset = fuel_types::AssetId::new([0; 32]); // Should be ETH?

    let balance = unwrapped_return.balances(query_asset).await;

    assert_eq!(balance, 0);
    // Something like unwrapped_return.balances() ?
}
