mod block;
mod block_height;
mod coin;
mod txpool;

pub use block::{FuelBlock, FuelBlockDb, FuelBlockHeader,FuelBlockConsensus,SealedFuelBlock};
pub use block_height::BlockHeight;
pub use coin::{Coin, CoinStatus};
pub use txpool::{ArcTx, TxInfo};

pub type DaBlockHeight = u64;
pub type ValidatorStake = u64;