use anyhow::Result;
use log::{error, info, warn};
use std::time::Duration;
use tokio::time::sleep;
use trend_arb::trade::binance::{BinanceWsTradeClient, ConnectionStatus, OrderSide, TimeInForce};

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();

    // 从环境变量获取API密钥
    let api_key = std::env::var("BINANCE_API_KEY")
        .unwrap_or_else(|_| "your_api_key".to_string());
    let secret_key = std::env::var("BINANCE_SECRET_KEY")
        .unwrap_or_else(|_| "your_secret_key".to_string());

    info!("Starting Binance WebSocket Trading Client Demo");

    // 创建客户端（使用测试网）
    let client = BinanceWsTradeClient::new(api_key, secret_key, true);

    // 在后台启动连接任务
    let client_clone = client.clone();
    let connection_task = tokio::spawn(async move {
        loop {
            if let Err(e) = client_clone.start().await {
                error!("Connection failed: {}", e);
                sleep(Duration::from_secs(5)).await;
                warn!("Retrying connection...");
            }
        }
    });

    // 等待连接建立
    info!("Waiting for connection to establish...");
    let mut connected = false;
    for i in 1..=30 {
        let status = client.get_connection_status().await;
        info!("Connection attempt {}: Status = {:?}", i, status);
        
        if status == ConnectionStatus::Connected {
            connected = true;
            break;
        }
        
        sleep(Duration::from_secs(1)).await;
    }

    if !connected {
        error!("Failed to establish connection within 30 seconds");
        connection_task.abort();
        return Ok(());
    }

    info!("Connection established successfully!");

    // 演示不同类型的订单
    demo_trading_operations(&client).await?;

    // 保持程序运行以维持连接
    info!("Demo completed. Keeping connection alive for 30 seconds...");
    sleep(Duration::from_secs(30)).await;

    // 清理
    connection_task.abort();
    info!("Demo finished");

    Ok(())
}

async fn demo_trading_operations(client: &BinanceWsTradeClient) -> Result<()> {
    info!("=== Starting Trading Operations Demo ===");

    // 1. 限价买单
    info!("1. Placing limit buy order...");
    match client
        .place_limit_order(
            "BTCUSDT",
            OrderSide::Buy,
            "15000.00",  // 远低于市价，避免实际成交
            "0.001",
            Some(TimeInForce::Gtc),
            Some("demo-limit-buy-001".to_string()),
            Some(5000),  // 5秒接收窗口
        )
        .await
    {
        Ok(response) => {
            info!("✅ Limit buy order response: Status = {}", response.status);
            if let Some(result) = response.result {
                info!("   Order ID: {}", result.order_id);
                info!("   Client Order ID: {}", result.client_order_id);
                info!("   Status: {}", result.status);
            }
        }
        Err(e) => {
            error!("❌ Failed to place limit buy order: {}", e);
        }
    }

    sleep(Duration::from_secs(2)).await;

    // 2. 限价卖单
    info!("2. Placing limit sell order...");
    match client
        .place_limit_order(
            "BTCUSDT",
            OrderSide::Sell,
            "100000.00",  // 远高于市价，避免实际成交
            "0.001",
            Some(TimeInForce::Ioc), // Immediate or Cancel
            Some("demo-limit-sell-001".to_string()),
            None,
        )
        .await
    {
        Ok(response) => {
            info!("✅ Limit sell order response: Status = {}", response.status);
            if let Some(result) = response.result {
                info!("   Order ID: {}", result.order_id);
                info!("   Status: {}", result.status);
                info!("   Time in Force: {:?}", result.time_in_force);
            }
        }
        Err(e) => {
            error!("❌ Failed to place limit sell order: {}", e);
        }
    }

    sleep(Duration::from_secs(2)).await;

    // 3. 市价买单（指定金额）
    info!("3. Placing market buy order (quote quantity)...");
    match client
        .place_market_order(
            "BTCUSDT",
            OrderSide::Buy,
            None,
            Some("10.00"), // 用10 USDT购买BTC
            Some("demo-market-buy-quote-001".to_string()),
            None,
        )
        .await
    {
        Ok(response) => {
            info!("✅ Market buy order (quote) response: Status = {}", response.status);
            if let Some(result) = response.result {
                info!("   Order ID: {}", result.order_id);
                info!("   Executed Qty: {}", result.executed_qty);
                info!("   Cumulative Quote Qty: {}", result.cummulative_quote_qty);
                
                if let Some(fills) = result.fills {
                    info!("   Fills count: {}", fills.len());
                    for (i, fill) in fills.iter().enumerate() {
                        info!("     Fill {}: Price = {}, Qty = {}", i + 1, fill.price, fill.qty);
                    }
                }
            }
        }
        Err(e) => {
            error!("❌ Failed to place market buy order: {}", e);
        }
    }

    sleep(Duration::from_secs(2)).await;

    // 4. 市价卖单（指定数量）
    info!("4. Placing market sell order (base quantity)...");
    match client
        .place_market_order(
            "BTCUSDT",
            OrderSide::Sell,
            Some("0.0001"), // 卖出0.0001 BTC
            None,
            Some("demo-market-sell-base-001".to_string()),
            None,
        )
        .await
    {
        Ok(response) => {
            info!("✅ Market sell order (base) response: Status = {}", response.status);
            if let Some(result) = response.result {
                info!("   Order ID: {}", result.order_id);
                info!("   Executed Qty: {}", result.executed_qty);
                info!("   Cumulative Quote Qty: {}", result.cummulative_quote_qty);
            }
        }
        Err(e) => {
            error!("❌ Failed to place market sell order: {}", e);
        }
    }

    sleep(Duration::from_secs(2)).await;

    // 5. 测试连接状态
    info!("5. Checking connection status...");
    let status = client.get_connection_status().await;
    info!("   Current connection status: {:?}", status);

    info!("=== Trading Operations Demo Completed ===");
    Ok(())
}
