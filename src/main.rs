use crate::{
    deepdata::binance_l1::BinanceL1DeepSocketClient,
    pairs::binance::ALL_BINANCE_SYMBOLS,
};
use chrono::Local;
use std::{io::Write, sync::LazyLock};
use trend_arb::trade::binance::BinanceWsTradeClient;

pub mod deepdata;
pub mod pairs;
pub mod trade;

pub static BINANCE_TRADE_CLIENT: LazyLock<BinanceWsTradeClient> = LazyLock::new(|| {
    env_logger::init();
    // 从环境变量获取API密钥
    let api_key =
        std::env::var("BINANCE_API_KEY").expect("Please set BINANCE_API_KEY environment variable");
    let secret_key = std::env::var("BINANCE_SECRET_KEY")
        .expect("Please set BINANCE_SECRET_KEY environment variable");

    // 创建客户端（使用测试网）
    let client = BinanceWsTradeClient::new(api_key, secret_key, false);

    // 启动连接（在实际使用中，这应该在单独的任务中运行）
    let client_clone = client.clone();
    tokio::spawn(async move {
        if let Err(e) = client_clone.start().await {
            log::error!("Connection failed: {}", e);
        }
    });
    return client;
});

#[tokio::main]
async fn main() {
    unsafe { std::env::set_var("RUST_LOG", "debug,trend_arb=debug") };
    // I. 初始化日志记录器
    env_logger::Builder::from_default_env()
        .format(|buf, record| {
            // 使用本地时间（带微秒）
            let ts = Local::now().format("%Y-%m-%d %H:%M:%S%.6f");
            writeln!(buf, "{} [{}] - {}", ts, record.level(), record.args())
        })
        .init();
    // II. 初始化深度数据连接
    BinanceL1DeepSocketClient::init_conn().await;
    // III. 订阅深度数据
    let _ = BinanceL1DeepSocketClient::subscribe(
        ALL_BINANCE_SYMBOLS
            .iter()
            .filter(|s| s.ends_with("USDT"))
            .map(|s| s.to_lowercase().to_string())
            .collect(),
    )
    .await;
    // IV. 启动心跳任务
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
            BinanceL1DeepSocketClient::heartbeat().await;
        }
    });
    // V. 保持程序运行
    tokio::signal::ctrl_c()
        .await
        .expect("Failed to listen for ctrl_c");
    log::info!("Ctrl-C received, exiting...");
}
