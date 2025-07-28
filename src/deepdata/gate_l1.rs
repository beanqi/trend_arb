use futures::StreamExt;
use log::{error, warn};
use serde::Deserialize;
use serde_json::json;
use std::sync::{
    atomic::{AtomicI64, Ordering},
    Arc,
};
use tokio::sync::Mutex;
use tungstenite::Message;

use trend_arb::web_socket::ws_connection::{TungWebSocketReader, TungWsConnection};

const MARKET_CODE: &str = "Gate";
static SOCKET_URL: &'static str = "wss://spotws-private.gateapi.io/ws/v4/";

static NB_CONN: std::sync::LazyLock<Mutex<Option<Arc<TungWsConnection>>>> =
    std::sync::LazyLock::new(|| Mutex::new(None));

static IDX_LAST_MSG_TIME: [AtomicI64; 50] = [const { AtomicI64::new(0) }; 50]; // 最后一次收到消息的时间

#[derive(Debug, Deserialize)]
struct GateBookTickerResult {
    s: String, // symbol
    b: String, // bid price
}

#[derive(Debug, Deserialize)]
struct GateBookTickerMessage {
    result: GateBookTickerResult,
}

pub struct GateL1DeepSocketClient;

impl GateL1DeepSocketClient {
    /**
     *  初始化连接对象
     */
    pub async fn init_conn() {
        let build_param_fn =
            |pair: &str| Self::get_sub_or_unsub_data(&vec![pair.to_string()], "subscribe");

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
                            if let Ok(msg) = sonic_rs::from_str::<GateBookTickerMessage>(&_frame) {
                                let bid_price: f64 = lexical_core::parse(msg.result.b.as_bytes())
                                    .unwrap_or_default();
                                let symbol = msg.result.s;

                                let old_price = super::utils::GATE_DEPTH.get(symbol.as_str()).unwrap();
                                let formal_price = old_price.load(Ordering::Relaxed);
                                if formal_price > 0.0 {
                                    let inc = (bid_price - formal_price) / formal_price;
                                    if inc > 0.005 || inc < -0.005 {
                                        warn!(
                                            "{}-websocket-{}-{}价格变动过大: {:.2}%，当前价格: {}, 之前价格: {}",
                                            MARKET_CODE,
                                            idx,
                                            symbol,
                                            inc * 100.0,
                                            bid_price,
                                            formal_price
                                        );
                                    }
                                }
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
                        _ => {
                            error!("{}-websocket{}-收到消息{:?}", MARKET_CODE, idx, msg);
                        }
                    }
                }
            })
        };
        let delete_fn = |_pair: &str| {};
        let nb_conn = TungWsConnection::new(
            MARKET_CODE,
            SOCKET_URL,
            true,
            100,
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

    /**
     * 获取订阅/取消订阅的消息内容
     */
    fn get_sub_or_unsub_data(symbol: &Vec<String>, _evt: &str) -> String {
        let timestamp = super::get_current_time_secs();
        let sub_message = json!({
            "time": timestamp,
            "channel": "spot.book_ticker",
            "event": _evt,
            "payload": symbol,
        });
        match sonic_rs::to_string(&sub_message) {
            Ok(msg) => msg,
            Err(e) => {
                error!("serde json to string error in {}: {:?}", MARKET_CODE, e);
                return String::new();
            }
        }
    }

    pub async fn subscribe(symbols: Vec<String>) -> Result<(), String> {
        let mut lock = NB_CONN.lock().await;
        if let Some(conn) = lock.as_mut() {
            let _res = conn
                .subscribe_vec(symbols, "subscribe", None, |symbols, param| {
                    Self::get_sub_or_unsub_data(symbols, param)
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
}
