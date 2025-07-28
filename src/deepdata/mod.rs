use serde::{Deserialize, Serialize};


pub mod binance_l1;
pub mod gate_l1;


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