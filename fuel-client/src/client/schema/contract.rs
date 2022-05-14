use crate::client::schema::{schema, AssetId, ContractId, HexString, Salt, U64};
#[derive(cynic::FragmentArguments, Debug)]
pub struct ContractByIdArgs {
    pub id: ContractId,
}

#[derive(cynic::FragmentArguments, Debug)]
pub struct ContractBalancesArgs {
    pub id: AssetId,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    schema_path = "./assets/schema.sdl",
    graphql_type = "Query",
    argument_struct = "ContractByIdArgs"
)]
pub struct ContractByIdQuery {
    #[arguments(id = &args.id)]
    pub contract: Option<Contract>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    schema_path = "./assets/schema.sdl",
    graphql_type = "Contract",
    argument_struct = "ContractBalancesArgs"
)]
pub struct Contract {
    pub id: ContractId,
    pub salt: Salt,
    pub bytecode: HexString,
    #[arguments(asset = &args.id)]
    pub balances: U64,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(schema_path = "./assets/schema.sdl", graphql_type = "Contract")]
pub struct ContractIdFragment {
    pub id: ContractId,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contract_by_id_query_gql_output() {
        use cynic::QueryBuilder;
        let operation = ContractByIdQuery::build(ContractByIdArgs {
            id: ContractId::default(),
        });
        insta::assert_snapshot!(operation.query)
    }
}
