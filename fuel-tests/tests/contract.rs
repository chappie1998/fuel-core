use fuel_core::{
    database::Database,
    service::{Config, FuelService}
};
use fuel_gql_client::client::FuelClient;
use fuel_storage::Storage;
use fuel_vm::prelude::Contract as FuelVmContract;
use fuel_types::ContractId;

#[tokio::test]
async fn contract() {
    // setup test data in the node
    let start_contract = FuelVmContract::default(); // Saved here to compare with returned contract via api
    let id = fuel_types::ContractId::new(Default::default());

    let mut db = Database::default();
    Storage::<fuel_types::ContractId, FuelVmContract>::insert(&mut db, &id, &start_contract).unwrap();
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
    let start_id:ContractId = id.into();

    let unwrapped_return = returned_contract.unwrap();

    let returned_id:ContractId = unwrapped_return.id.into();

    assert_eq!(start_id, returned_id);

    // Test Salt property of contracts
    let salt = unwrapped_return.salt
}
