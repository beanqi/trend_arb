use anyhow::Result;
use log::info;
use trend_arb::trade::binance::{BinanceWsTradeClient, OrderSide, TimeInForce};

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();

    // 从环境变量获取API密钥
    let api_key = std::env::var("BINANCE_API_KEY")
        .expect("Please set BINANCE_API_KEY environment variable");
    let secret_key = std::env::var("BINANCE_SECRET_KEY")
        .expect("Please set BINANCE_SECRET_KEY environment variable");

    // 创建客户端（使用测试网）
    let client = BinanceWsTradeClient::new(api_key, secret_key, false);

    // 启动连接（在实际使用中，这应该在单独的任务中运行）
    let client_clone = client.clone();
    let connection_task = tokio::spawn(async move {
        if let Err(e) = client_clone.start().await {
            log::error!("Connection failed: {}", e);
        }
    });

    // 等待连接建立
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

    // 示例：下限价单
    info!("Placing limit order...");
    match client
        .place_limit_order(
            "TRXUSDT",
            OrderSide::Buy,
            "0.1",  // 价格
            "51",     // 数量
            Some(TimeInForce::Gtc),
            None,        // 客户端订单ID
            None,        // recv_window
        )
        .await
    {
        Ok(response) => {
            info!("Limit order placed successfully: {:?}", response);
        }
        Err(e) => {
            log::error!("Failed to place limit order: {}", e);
        }
    }

    // 示例：下市价单
    info!("Placing market order...");
    match client
        .place_market_order(
            "BTCUSDT",
            OrderSide::Buy,
            None, // 数量
            Some("1"), // 下单金额
            None,          // 客户端订单ID
            None,          // recv_window
        )
        .await
    {
        Ok(response) => {
            info!("Market order placed successfully: {:?}", response);
        }
        Err(e) => {
            log::error!("Failed to place market order: {}", e);
        }
    }

    // 保持程序运行以维持连接
    tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;

    // 取消连接任务
    connection_task.abort();

    Ok(())
}
