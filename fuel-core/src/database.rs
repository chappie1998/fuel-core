#[cfg(feature = "rocksdb")]
use crate::database::columns::COLUMN_NUM;
use crate::database::transactional::DatabaseTransaction;
use crate::model::FuelBlockDb;
#[cfg(feature = "rocksdb")]
use crate::state::rocks_db::RocksDb;
use crate::state::{
    in_memory::memory_store::MemoryStore, ColumnId, DataSource, Error, IterDirection,
};
use async_trait::async_trait;
pub use fuel_core_interfaces::db::KvStoreError;
use fuel_core_interfaces::relayer::RelayerDb;
use fuel_storage::Storage;
use fuel_vm::prelude::{Address, Bytes32, InterpreterStorage};
use serde::{de::DeserializeOwned, Serialize};
use std::fmt::Debug;
use std::marker::Send;
#[cfg(feature = "rocksdb")]
use std::path::Path;
use std::{collections::HashMap, ops::DerefMut};
use std::{
    fmt::{self, Formatter},
    sync::Arc,
};
#[cfg(feature = "rocksdb")]
use tempfile::TempDir;

use self::columns::METADATA;

pub mod balances;
pub mod block;
pub mod code_root;
pub mod coin;
pub mod contracts;
pub mod deposit_coin;
pub mod metadata;
mod receipts;
pub mod state;
pub mod transaction;
pub mod transactional;
pub mod validator_set;
pub mod validator_set_diffs;

// Crude way to invalidate incompatible databases,
// can be used to perform migrations in the future.
pub const VERSION: u32 = 0;

pub mod columns {
    pub const METADATA: u32 = 0;
    pub const CONTRACTS: u32 = 1;
    pub const CONTRACTS_CODE_ROOT: u32 = 2;
    pub const CONTRACTS_STATE: u32 = 3;
    // Contract Id -> Utxo Id
    pub const CONTRACT_UTXO_ID: u32 = 4;
    pub const BALANCES: u32 = 5;
    pub const COIN: u32 = 6;
    // (owner, coin id) => true
    pub const OWNED_COINS: u32 = 7;
    pub const TRANSACTIONS: u32 = 8;
    // tx id -> current status
    pub const TRANSACTION_STATUS: u32 = 9;
    pub const TRANSACTIONS_BY_OWNER_BLOCK_IDX: u32 = 10;
    pub const RECEIPTS: u32 = 11;
    pub const BLOCKS: u32 = 12;
    // maps block id -> block hash
    pub const BLOCK_IDS: u32 = 13;
    pub const TOKEN_DEPOSITS: u32 = 14;
    pub const VALIDATOR_SET: u32 = 15;
    pub const VALIDATOR_SET_DIFFS: u32 = 16;

    // Number of columns
    #[cfg(feature = "rocksdb")]
    pub const COLUMN_NUM: u32 = 17;
}

#[derive(Clone, Debug)]
pub struct Database {
    data: DataSource,
    // used for RAII
    _drop: Arc<DropResources>,
}

trait DropFnTrait: FnOnce() {}
impl<F> DropFnTrait for F where F: FnOnce() {}
type DropFn = Box<dyn DropFnTrait>;

impl fmt::Debug for DropFn {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "DropFn")
    }
}

#[derive(Debug, Default)]
struct DropResources {
    // move resources into this closure to have them dropped when db drops
    drop: Option<DropFn>,
}

impl<F: 'static + FnOnce()> From<F> for DropResources {
    fn from(closure: F) -> Self {
        Self {
            drop: Option::Some(Box::new(closure)),
        }
    }
}

impl Drop for DropResources {
    fn drop(&mut self) {
        if let Some(drop) = self.drop.take() {
            (drop)()
        }
    }
}

/*** SAFETY: we are safe to do it because DataSource is Send+Sync and there is nowhere it is overwritten
 * it is not Send+Sync by default because Storage insert fn takes &mut self
*/
unsafe impl Send for Database {}
unsafe impl Sync for Database {}

impl Database {
    #[cfg(feature = "rocksdb")]
    pub fn open(path: &Path) -> Result<Self, Error> {
        let db = RocksDb::open(path, COLUMN_NUM)?;

        Ok(Database {
            data: Arc::new(db),
            _drop: Default::default(),
        })
    }

    pub fn in_memory() -> Self {
        Self {
            data: Arc::new(MemoryStore::default()),
            _drop: Default::default(),
        }
    }

    fn insert<K: Into<Vec<u8>>, V: Serialize + DeserializeOwned>(
        &self,
        key: K,
        column: ColumnId,
        value: V,
    ) -> Result<Option<V>, Error> {
        let result = self.data.put(
            key.into(),
            column,
            bincode::serialize(&value).map_err(|_| Error::Codec)?,
        )?;
        if let Some(previous) = result {
            Ok(Some(
                bincode::deserialize(&previous).map_err(|_| Error::Codec)?,
            ))
        } else {
            Ok(None)
        }
    }

    fn remove<V: DeserializeOwned>(
        &self,
        key: &[u8],
        column: ColumnId,
    ) -> Result<Option<V>, Error> {
        self.data
            .delete(key, column)?
            .map(|val| bincode::deserialize(&val).map_err(|_| Error::Codec))
            .transpose()
    }

    fn get<V: DeserializeOwned>(&self, key: &[u8], column: ColumnId) -> Result<Option<V>, Error> {
        self.data
            .get(key, column)?
            .map(|val| bincode::deserialize(&val).map_err(|_| Error::Codec))
            .transpose()
    }

    fn exists(&self, key: &[u8], column: ColumnId) -> Result<bool, Error> {
        self.data.exists(key, column)
    }

    fn iter_all<K, V>(
        &self,
        column: ColumnId,
        prefix: Option<Vec<u8>>,
        start: Option<Vec<u8>>,
        direction: Option<IterDirection>,
    ) -> impl Iterator<Item = Result<(K, V), Error>> + '_
    where
        K: From<Vec<u8>>,
        V: DeserializeOwned,
    {
        self.data
            .iter_all(column, prefix, start, direction.unwrap_or_default())
            .map(|(key, value)| {
                let key = K::from(key);
                let value: V = bincode::deserialize(&value).map_err(|_| Error::Codec)?;
                Ok((key, value))
            })
    }

    pub fn transaction(&self) -> DatabaseTransaction {
        self.into()
    }
}

impl AsRef<Database> for Database {
    fn as_ref(&self) -> &Database {
        self
    }
}

/// Construct an ephemeral database
/// uses rocksdb when rocksdb features are enabled
/// uses in-memory when rocksdb features are disabled
impl Default for Database {
    fn default() -> Self {
        #[cfg(not(feature = "rocksdb"))]
        {
            Self {
                data: Arc::new(MemoryStore::default()),
                _drop: Default::default(),
            }
        }
        #[cfg(feature = "rocksdb")]
        {
            let tmp_dir = TempDir::new().unwrap();
            Self {
                data: Arc::new(RocksDb::open(tmp_dir.path(), columns::COLUMN_NUM).unwrap()),
                _drop: Arc::new(
                    {
                        move || {
                            // cleanup temp dir
                            drop(tmp_dir);
                        }
                    }
                    .into(),
                ),
            }
        }
    }
}

impl InterpreterStorage for Database {
    type DataError = Error;

    fn block_height(&self) -> Result<u32, Error> {
        let height = self.get_block_height()?.unwrap_or_default();
        Ok(height.into())
    }

    fn block_hash(&self, block_height: u32) -> Result<Bytes32, Error> {
        let hash = self.get_block_id(block_height.into())?.unwrap_or_default();
        Ok(hash)
    }

    fn coinbase(&self) -> Result<Address, Error> {
        let height = self.get_block_height()?.unwrap_or_default();
        let id = self.block_hash(height.into())?;
        let block = Storage::<Bytes32, FuelBlockDb>::get(self, &id)?.unwrap_or_default();
        Ok(block.headers.producer)
    }
}

#[async_trait]
impl RelayerDb for Database {
    async fn get_validators(&self) -> HashMap<Address, u64> {
        struct WrapAddress(pub Address);
        impl From<Vec<u8>> for WrapAddress {
            fn from(i: Vec<u8>) -> Self {
                Self(Address::try_from(i.as_ref()).unwrap())
            }
        }
        let mut out = HashMap::new();
        for diff in self.iter_all::<WrapAddress, u64>(columns::VALIDATOR_SET, None, None, None) {
            match diff {
                Ok((address, stake)) => {
                    out.insert(address.0, stake);
                }
                Err(err) => panic!("Database internal error:{:?}", err),
            }
        }
        out
    }

    async fn get_validator_diffs(
        &self,
        from_da_height: u64,
        to_da_height: Option<u64>,
    ) -> Vec<(u64, HashMap<Address, u64>)> {
        let to_da_height = if let Some(to_da_height) = to_da_height {
            if from_da_height > to_da_height {
                return Vec::new();
            }
            to_da_height
        } else {
            u64::MAX
        };
        struct WrapU64Be(pub u64);
        impl From<Vec<u8>> for WrapU64Be {
            fn from(i: Vec<u8>) -> Self {
                use byteorder::{BigEndian, ReadBytesExt};
                use std::io::Cursor;
                let mut i = Cursor::new(i);
                Self(i.read_u64::<BigEndian>().unwrap_or_default())
            }
        }
        let mut out = Vec::new();
        for diff in self.iter_all::<WrapU64Be, HashMap<Address, u64>>(
            columns::VALIDATOR_SET_DIFFS,
            None,
            Some(from_da_height.to_be_bytes().to_vec()),
            None,
        ) {
            match diff {
                Ok((key, diff)) => {
                    let block = key.0;
                    if block >= to_da_height {
                        return out;
                    }
                    out.push((block, diff))
                }
                Err(err) => panic!("get_validator_diffs unexpected error:{:?}", err),
            }
        }
        out
    }

    async fn apply_validator_diffs(&mut self, changes: &HashMap<Address, u64>, da_height: u64) {
        // this is reimplemented inside fuel-core db to assure it is atomic operation in case of poweroff situation.
        let mut db = self.transaction();
        for (address, stake) in changes {
            let _ = Storage::<Address, u64>::insert(db.deref_mut(), address, stake);
        }
        db.set_validators_da_height(da_height).await;
        if let Err(err) = db.commit() {
            panic!("apply_validator_diffs database currupted: {:?}", err);
        }
    }

    async fn get_block_height(&self) -> u64 {
        match self.get_block_height() {
            Ok(res) => {
                return u64::from(
                    res.expect("get_block_height value should be always present and set"),
                );
            }
            Err(err) => {
                panic!("get_block_height database curruption, err:{:?}", err);
            }
        }
    }

    async fn set_finalized_da_height(&self, block: u64) {
        if let Err(err) = self.insert(metadata::FINALIZED_DA_HEIGHT, METADATA, block) {
            panic!("set_finalized_da_height should always succeed: {:?}", err);
        }
    }

    async fn get_finalized_da_height(&self) -> u64 {
        match self.get(metadata::FINALIZED_DA_HEIGHT, METADATA) {
            Ok(res) => {
                return res
                    .expect("get_finalized_da_height value should be always present and set");
            }
            Err(err) => {
                panic!("get_finalized_da_height database curruption, err:{:?}", err);
            }
        }
    }

    async fn set_validators_da_height(&self, block: u64) {
        if let Err(err) = self.insert(metadata::VALIDATORS_DA_HEIGHT, METADATA, block) {
            panic!("set_validators_da_height should always succeed: {:?}", err);
        }
    }

    async fn get_validators_da_height(&self) -> u64 {
        match self.get(metadata::VALIDATORS_DA_HEIGHT, METADATA) {
            Ok(res) => {
                return res
                    .expect("get_validators_da_height value should be always present and set");
            }
            Err(err) => {
                panic!(
                    "get_validators_da_height database curruption, err:{:?}",
                    err
                );
            }
        }
    }
}
