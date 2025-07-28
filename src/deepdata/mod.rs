use serde::{Deserialize, Serialize};
use trend_arb::MarketCode;

pub mod binance_l1;
pub mod gate_l1;
pub mod utils;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DepthSocketCurrencyPairs {
    pub market_code: &'static str,
    pub currency_code: String,
    pub currency_unit: String,
    pub symbol: String,
    pub socket_key: String,
    pub guid: String,
    pub last_data_time: i64,
    /// <summary>
    /// 用于一个交易对一个连接发送ping指令的时间记录
    /// </summary>
    pub send_ping_pong_time: u128,
}

pub fn get_current_time_secs() -> i64 {
    chrono::Utc::now().timestamp()
}

/// 归一化交易对，全部转换成大写，并且默认都是usdt的交易对, 只需要返回币的名称
pub fn normalize_pair(pair: &str, market: &MarketCode) -> String {
    match market {
        MarketCode::Binance => pair.trim_end_matches("USDT").to_string(),
        MarketCode::Gate => pair.trim_end_matches("_USDT").to_string(),
    }
}
