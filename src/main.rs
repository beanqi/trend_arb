use crate::deepdata::{binance_l1::BinanceL1DeepSocketClient, gate_l1::GateL1DeepSocketClient};

pub mod deepdata;
pub mod pairs;

#[tokio::main]
async fn main() {
    unsafe { std::env::set_var("RUST_LOG", "info,trend_arb=debug") };
    env_logger::init();

    GateL1DeepSocketClient::init_conn().await;
    BinanceL1DeepSocketClient::init_conn().await;
    let _ = BinanceL1DeepSocketClient::subscribe(vec!["btcusdt".to_string()]).await;
    // let _ = GateL1DeepSocketClient::subscribe(vec!["BTC_USDT".to_string()]).await;

    tokio::spawn(async move {
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
            GateL1DeepSocketClient::heartbeat().await;
        }
    });
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
            BinanceL1DeepSocketClient::heartbeat().await;
        }
    });

    tokio::signal::ctrl_c()
        .await
        .expect("Failed to listen for ctrl_c");
    log::info!("Ctrl-C received, exiting...");
}
