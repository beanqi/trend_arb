use crate::{
    deepdata::{binance_l1::BinanceL1DeepSocketClient, gate_l1::GateL1DeepSocketClient},
    pairs::{binance::ALL_BINANCE_SYMBOLS, gate::ALL_GATEIO_SYMBOLS},
};
use chrono::Local;
use std::io::Write;

pub mod deepdata;
pub mod pairs;

#[tokio::main]
async fn main() {
    unsafe { std::env::set_var("RUST_LOG", "info,trend_arb=debug") };

    env_logger::Builder::from_default_env()
        .format(|buf, record| {
            // 使用本地时间（带微秒）
            let ts = Local::now().format("%Y-%m-%d %H:%M:%S%.6f");
            writeln!(buf, "{} [{}] - {}", ts, record.level(), record.args())
        })
        .init();

    GateL1DeepSocketClient::init_conn().await;
    BinanceL1DeepSocketClient::init_conn().await;
    let _ = BinanceL1DeepSocketClient::subscribe(
        ALL_BINANCE_SYMBOLS.iter().map(|s| s.to_lowercase().to_string()).collect(),
    )
    .await;
    let _ = GateL1DeepSocketClient::subscribe(
        ALL_GATEIO_SYMBOLS.iter().map(|s| s.to_string()).collect(),
    )
    .await;

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
