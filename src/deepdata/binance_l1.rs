use futures::StreamExt;
use log::{error, warn};
use serde_json::json;
use std::sync::{
    Arc,
    atomic::{AtomicI64, AtomicU64, Ordering},
};
use tokio::sync::Mutex;
use tungstenite::Message;

use trend_arb::web_socket::ws_connection::{TungWebSocketReader, TungWsConnection};

const MARKET_CODE: &str = "Binance";
static SOCKET_URL: &'static str = "wss://stream.binance.com:9443/stream";

static NB_CONN: std::sync::LazyLock<Mutex<Option<Arc<TungWsConnection>>>> =
    std::sync::LazyLock::new(|| Mutex::new(None));

static AUTO_INCRE_ID: AtomicU64 = AtomicU64::new(0);

static IDX_LAST_MSG_TIME: [AtomicI64; 50] = [const { AtomicI64::new(0) }; 50]; // 最后一次收到消息的时间

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
                            log::info!(
                                "{}-websocket-{}收到消息: {}",
                                MARKET_CODE,
                                idx,
                                _frame
                            );
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
            "Binance",
            SOCKET_URL,
            true,
            2,
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
}
