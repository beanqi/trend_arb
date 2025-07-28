use std::{collections::HashMap, sync::{atomic::{AtomicU64, Ordering}, LazyLock}};

use crate::pairs::{binance::ALL_BINANCE_SYMBOLS, gate::ALL_GATEIO_SYMBOLS};
pub struct AtomicF64 {
    storage: AtomicU64,
}
impl AtomicF64 {
    pub fn new(value: f64) -> Self {
        let as_u64 = value.to_bits();
        Self {
            storage: AtomicU64::new(as_u64),
        }
    }
    pub fn store(&self, value: f64, ordering: Ordering) {
        let as_u64 = value.to_bits();
        self.storage.store(as_u64, ordering)
    }
    pub fn load(&self, ordering: Ordering) -> f64 {
        let as_u64 = self.storage.load(ordering);
        f64::from_bits(as_u64)
    }
}
pub static BINANCE_DEPTH: LazyLock<HashMap<&'static str, AtomicF64>> = LazyLock::new(|| {
    let mut map = HashMap::new();
    ALL_BINANCE_SYMBOLS.iter().for_each(|&pair| {
        map.insert(pair, AtomicF64::new(0.0));
    });
    map
});

pub static GATE_DEPTH: LazyLock<HashMap<&'static str, AtomicF64>> = LazyLock::new(|| {
    let mut map = HashMap::new();
    // 假设 Gate.io 的交易对列表与 Binance 相同
    ALL_GATEIO_SYMBOLS.iter().for_each(|&pair| {
        map.insert(pair, AtomicF64::new(0.0));
    });
    map
});
