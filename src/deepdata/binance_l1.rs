use futures::StreamExt;
use log::{error, warn};
use moka::sync::Cache;
use serde::Deserialize;
use serde_json::json;
use std::sync::{
    Arc, LazyLock,
    atomic::{AtomicI64, AtomicU64, Ordering},
};
use std::time::Duration;
use tokio::sync::Mutex;
use tungstenite::Message;

use trend_arb::{
    trade::binance::BnbOrderSide,
    web_socket::ws_connection::{TungWebSocketReader, TungWsConnection},
};

use crate::BINANCE_TRADE_CLIENT;
// 被定义为剧烈波动的阈值
const VOLATILE_THRESHOLD: f64 = 0.23; // 23%
const MARKET_CODE: &str = "Binance";
static SOCKET_URL: &'static str = "wss://stream.binance.com:9443/stream";
static NB_CONN: LazyLock<Mutex<Option<Arc<TungWsConnection>>>> = LazyLock::new(|| Mutex::new(None)); // 连接对象管理工具

static AUTO_INCRE_ID: AtomicU64 = AtomicU64::new(0); // 自动递增的ID

static IDX_LAST_MSG_TIME: [AtomicI64; 50] = [const { AtomicI64::new(0) }; 50]; // 最后一次收到消息的时间

// 控制下单的集合，需要控制同一个交易对，5小时内只能下单一次
static ORDER_CONTROL: LazyLock<Cache<String, ()>> = LazyLock::new(|| {
    Cache::builder()
        .max_capacity(1000)
        .time_to_live(std::time::Duration::from_secs(60 * 60 * 5)) // 5小时
        .build()
});

pub struct BinanceL1DeepSocketClient;

impl BinanceL1DeepSocketClient {
    /**
     *  初始化连接对象
     */
    pub async fn init_conn() {
        let build_param_fn = |pair: &str| {
            format!(
                r#"{{"method":"SUBSCRIBE","params":["{}@bookTicker"],"id":{}}}"#,
                pair,
                AUTO_INCRE_ID.fetch_add(1, Ordering::Relaxed)
            )
        };
        let do_read_fn = |reader: TungWebSocketReader, idx: u8| -> tokio::task::JoinHandle<()> {
            tokio::spawn(async move {
                let mut reader = reader;
                loop {
                    let msg = reader.next().await;
                    if msg.is_none() {
                        warn!(
                            "{}-websocket-{}读取消息为空，连接已经关闭，待重新连接",
                            MARKET_CODE, idx
                        );
                        IDX_LAST_MSG_TIME[idx as usize]
                            .store(super::get_current_time_secs() - 60, Ordering::SeqCst);
                        break;
                    }
                    let msg = msg.unwrap();
                    if msg.is_err() {
                        error!(
                            "{}-读取websocket-{}失败,错误信息: {:?}",
                            MARKET_CODE,
                            idx,
                            msg.err()
                        );
                        IDX_LAST_MSG_TIME[idx as usize]
                            .store(super::get_current_time_secs() - 60, Ordering::Release);
                        break;
                    }
                    let msg = msg.unwrap();
                    IDX_LAST_MSG_TIME[idx as usize]
                        .store(super::get_current_time_secs(), Ordering::SeqCst);
                    match msg {
                        Message::Text(_frame) => {
                            // 使用 sonic-rs 快速解析，只提取需要的字段
                            if let Ok(msg) = sonic_rs::from_str::<BinanceBookTickerMessage>(&_frame)
                            {
                                let bid_price: f64 =
                                    lexical_core::parse(msg.data.b.as_bytes()).unwrap_or_default();
                                let symbol = msg.data.s;

                                Self::handle_volatile_price(&symbol, bid_price, idx).await;

                                let old_price =
                                    super::utils::BINANCE_DEPTH.get(symbol.as_str()).unwrap();
                                old_price.store(bid_price, Ordering::Relaxed);
                            }
                        }
                        Message::Close(reason) => {
                            error!(
                                "收到消息:{}-websocket-{}关闭, 原因: {:?}",
                                MARKET_CODE, idx, reason
                            );
                            continue;
                        }
                        Message::Ping(_) => {
                            // 不需要处理
                        }
                        _ => {
                            error!("{}-websocket{}-收到消息{:?}", MARKET_CODE, idx, msg);
                        }
                    }
                }
            })
        };
        let delete_fn = |_pair: &str| {
            super::utils::BINANCE_DEPTH
                .get(_pair)
                .unwrap()
                .store(0.0, Ordering::Relaxed);
        };
        let nb_conn = TungWsConnection::new(
            "Binance",
            SOCKET_URL,
            true,
            150,
            Arc::new(do_read_fn),
            Box::new(build_param_fn),
            Box::new(delete_fn),
        )
        .await;
        nb_conn.set_write_wait_time(300); // 每秒最多接受5个消息，包括ping pong
        let _res = nb_conn.init().await;
        let mut lock = NB_CONN.lock().await;
        *lock = Some(nb_conn);
        drop(lock);
    }

    fn get_sub_msg(symbol: &Vec<String>, method: &str) -> String {
        let json = json!({
            "method": method,
            "params": symbol.iter().map(|s| format!("{}@bookTicker", s)).collect::<Vec<_>>(),
            "id": AUTO_INCRE_ID.fetch_add(1, Ordering::Relaxed)
        });
        json.to_string()
    }

    pub async fn subscribe(symbols: Vec<String>) -> Result<(), String> {
        let mut lock = NB_CONN.lock().await;
        if let Some(conn) = lock.as_mut() {
            let _res = conn
                .subscribe_vec(symbols, "SUBSCRIBE", None, |symbols, param| {
                    Self::get_sub_msg(symbols, param)
                })
                .await;
            Ok(())
        } else {
            Err(format!("{}-连接未初始化", MARKET_CODE))
        }
    }

    pub async fn heartbeat() {
        let mut nb_conn = NB_CONN.lock().await;
        if let Some(conn) = nb_conn.as_mut() {
            let conns = conn.get_connected_conn().await;
            for idx in conns {
                if IDX_LAST_MSG_TIME[idx as usize].load(Ordering::Relaxed)
                    < super::get_current_time_secs() - 60
                {
                    warn!("{}-websocket-{}连接超时，尝试重新连接", MARKET_CODE, idx);
                    conn.reconnect(idx).await;
                }
            }
        }
    }

    /// 处理价格剧烈波动：若满足条件则下单
    async fn handle_volatile_price(
        symbol: &str,
        bid_price: f64,
        idx: u8,
    ) {
        let old_price = super::utils::BINANCE_DEPTH.get(symbol).unwrap();
        let formal_price = old_price.load(Ordering::Relaxed);
        if formal_price <= 0.0 {
            old_price.store(bid_price, Ordering::Relaxed);
            return;
        }

        let inc = (bid_price - formal_price) / formal_price;
        if inc < VOLATILE_THRESHOLD {
            return;
        }

        warn!(
            "{}-websocket-{}-{}价格变动过大: {:.2}%，当前价格: {}, 之前价格: {}",
            MARKET_CODE,
            idx,
            symbol,
            inc * 100.0,
            bid_price,
            formal_price
        );

        if ORDER_CONTROL.get(symbol).is_some() {
            warn!(
                "{}-websocket-{}-{}价格变动过大，但已经下过单，跳过",
                MARKET_CODE, idx, symbol
            );
            return;
        }

        // 记录下单
        ORDER_CONTROL.insert(symbol.to_owned(), ());

        let res = BINANCE_TRADE_CLIENT
            .place_market_order(
                symbol,
                BnbOrderSide::Buy,
                None,
                Some("45.0"),
                None,
                None,
            )
            .await;
        log::warn!(
            "{}-websocket-{}-{}下单结果: {:?}",
            MARKET_CODE,
            idx,
            symbol,
            res
        );

        // 下单成功后处理成交信息，并在1秒后卖出全部仓位
        if let Ok(resp) = &res {
            if let Some(order) = &resp.result {
                // 成交数量（基础币种）
                let executed_qty = order.executed_qty.clone();
                let cumm_quote = order.cummulative_quote_qty.clone();
                if let Some(fills) = &order.fills {
                    for f in fills {
                        log::warn!(
                            "{}-websocket-{}-{}成交明细 price={} qty={} commission={} {} trade_id={}",
                            MARKET_CODE,
                            idx,
                            symbol,
                            f.price,
                            f.qty,
                            f.commission,
                            f.commission_asset,
                            f.trade_id
                        );
                    }
                }
                log::warn!(
                    "{}-websocket-{}-{}买单成交: executed_qty={} quote_spent={}",
                    MARKET_CODE,
                    idx,
                    symbol,
                    executed_qty,
                    cumm_quote
                );

                // 卖出逻辑：如果成交数量有效，则1秒后以市价全部卖出
                // 卖出数量 = 成交数量 * 0.985 （保留原始小数位）
                let symbol_for_sell = symbol.to_string();
                let executed_qty_str = executed_qty.clone();
                // 统计原小数位数
                let decimal_places = executed_qty_str
                    .split('.')
                    .nth(1)
                    .map(|s| s.len())
                    .unwrap_or(0);
                match executed_qty_str.parse::<rust_decimal::Decimal>() {
                    Ok(qty_decimal) => {
                        if qty_decimal > rust_decimal::Decimal::ZERO {
                            // 乘以 0.985
                            let factor = rust_decimal::Decimal::from_str_exact("0.985").unwrap();
                            let mut sell_decimal = qty_decimal * factor;
                            // 量化到原小数位（截断而不是四舍五入，以避免数量超过账户可用数量导致下单失败）
                            sell_decimal = sell_decimal.trunc_with_scale(decimal_places as u32);
                            // 确保在截断后仍然 > 0
                            if sell_decimal > rust_decimal::Decimal::ZERO {
                                let qty_for_sell = sell_decimal.normalize().to_string();
                                tokio::spawn(async move {
                                    tokio::time::sleep(Duration::from_secs(1)).await;
                                    let sell_res = BINANCE_TRADE_CLIENT
                                        .place_market_order(
                                            &symbol_for_sell,
                                            BnbOrderSide::Sell,
                                            Some(&qty_for_sell),
                                            None,
                                            None,
                                            None,
                                        )
                                        .await;
                                    log::warn!(
                                        "{}-websocket-{}-{}卖出结果: {:?} (sell_qty={})",
                                        MARKET_CODE,
                                        idx,
                                        symbol_for_sell,
                                        sell_res,
                                        qty_for_sell
                                    );
                                });
                            } else {
                                log::warn!(
                                    "{}-websocket-{}-{} 计算后的卖出数量<=0，跳过 (executed_qty={}, decimal_places={})",
                                    MARKET_CODE, idx, symbol, executed_qty_str, decimal_places
                                );
                            }
                        } else {
                            log::warn!(
                                "{}-websocket-{}-{}成交数量无效(<=0)，不执行卖出: {}",
                                MARKET_CODE,
                                idx,
                                symbol,
                                executed_qty_str
                            );
                        }
                    }
                    Err(e) => {
                        log::warn!(
                            "{}-websocket-{}-{}解析成交数量失败，字符串: {}, err={}",
                            MARKET_CODE,
                            idx,
                            symbol,
                            executed_qty_str,
                            e
                        );
                    }
                }
            }
        }
    }
}

#[derive(Debug, Deserialize)]
struct BinanceBookTickerData {
    s: String, // symbol
    b: String, // bid price
}

#[derive(Debug, Deserialize)]
struct BinanceBookTickerMessage {
    data: BinanceBookTickerData,
}
