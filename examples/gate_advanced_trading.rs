use anyhow::Result;
use env_logger;
use log::{info, warn, error};
use std::time::Duration;
use tokio::time::{sleep, interval};
use trend_arb::trade::gate::{GateWsTradeClient, GateOrderSide, GateTimeInForce, ConnectionStatus};

/// 高级Gate.io交易示例
/// 展示如何实现一个简单的交易策略，包括：
/// - 连接管理和心跳
/// - 错误处理和重连
/// - 订单生命周期管理
#[tokio::main]
async fn main() -> Result<()> {
    // 初始化日志
    env_logger::Builder::from_default_env()
        .filter_level(log::LevelFilter::Debug)
        .init();

    info!("Starting Gate.io advanced trading demo...");

    // 创建Gate.io WebSocket客户端
    let client = GateWsTradeClient::new(
        "YOUR_API_KEY".to_string(),
        "YOUR_API_SECRET".to_string(),
        true, // 使用测试网
    );

    // 启动连接任务
    let client_for_connection = client.clone();
    let connection_task = tokio::spawn(async move {
        loop {
            if let Err(e) = client_for_connection.start().await {
                error!("Connection error: {}", e);
                sleep(Duration::from_secs(5)).await;
            }
        }
    });

    // 启动心跳任务
    let client_for_heartbeat = client.clone();
    let heartbeat_task = tokio::spawn(async move {
        let mut heartbeat_interval = interval(Duration::from_secs(30));
        loop {
            heartbeat_interval.tick().await;
            if client_for_heartbeat.is_logged_in().await {
                if let Err(e) = client_for_heartbeat.ping().await {
                    warn!("Heartbeat failed: {}", e);
                } else {
                    info!("Heartbeat sent successfully");
                }
            }
        }
    });

    // 等待连接建立
    wait_for_connection(&client).await?;

    // 执行交易策略
    run_trading_strategy(&client).await?;

    // 清理任务
    connection_task.abort();
    heartbeat_task.abort();

    info!("Demo completed");
    Ok(())
}

/// 等待连接建立
async fn wait_for_connection(client: &GateWsTradeClient) -> Result<()> {
    info!("Waiting for connection...");
    
    for i in 0..30 {
        let status = client.get_connection_status().await;
        let logged_in = client.is_logged_in().await;
        
        info!("Connection attempt {}: status={:?}, logged_in={}", i + 1, status, logged_in);
        
        if status == ConnectionStatus::LoggedIn && logged_in {
            info!("Successfully connected and logged in!");
            return Ok(());
        }
        
        sleep(Duration::from_secs(1)).await;
    }
    
    Err(anyhow::anyhow!("Failed to establish connection within 30 seconds"))
}

/// 运行交易策略
async fn run_trading_strategy(client: &GateWsTradeClient) -> Result<()> {
    info!("Starting trading strategy...");
    
    // 策略参数
    let symbol = "BTC_USDT";
    let trade_amount = "0.001";
    let mut order_counter = 0;

    for round in 1..=3 {
        info!("=== Trading Round {} ===", round);
        
        // 检查连接状态
        if !client.is_logged_in().await {
            warn!("Not logged in, skipping round {}", round);
            sleep(Duration::from_secs(5)).await;
            continue;
        }

        order_counter += 1;
        let order_id = format!("demo-order-{}-{}", round, order_counter);
        
        // 第一步：下限价买单
        info!("Step 1: Placing limit buy order...");
        let buy_order_result = client.place_limit_order(
            symbol,
            GateOrderSide::Buy,
            "50000.0", // 较低的价格，不太可能立即成交
            trade_amount,
            Some(GateTimeInForce::Gtc),
            Some(order_id.clone()),
            None,
        ).await;

        let order_id = match buy_order_result {
            Ok(response) => {
                if response.header.status == "200" {
                    if let Some(order) = response.data.result {
                        info!("Buy order placed successfully: ID={}, Status={}", order.id, order.status);
                        order.id
                    } else {
                        warn!("Buy order response has no result");
                        continue;
                    }
                } else {
                    if let Some(error) = response.data.errs {
                        warn!("Buy order failed: {}: {}", error.label, error.message);
                    } else {
                        warn!("Buy order failed with status: {}", response.header.status);
                    }
                    continue;
                }
            }
            Err(e) => {
                error!("Failed to place buy order: {}", e);
                continue;
            }
        };

        sleep(Duration::from_secs(2)).await;

        // 第二步：查询订单状态
        info!("Step 2: Checking order status...");
        match client.get_order_status(symbol, &order_id, None).await {
            Ok(response) => {
                if response.header.status == "200" {
                    if let Some(order) = response.data.result {
                        info!("Order status: ID={}, Status={}, Filled={}, Left={}", 
                              order.id, order.status, order.filled_total, order.left);
                    }
                } else {
                    if let Some(error) = response.data.errs {
                        warn!("Order status query failed: {}: {}", error.label, error.message);
                    }
                }
            }
            Err(e) => {
                error!("Failed to query order status: {}", e);
            }
        }

        sleep(Duration::from_secs(2)).await;

        // 第三步：取消订单
        info!("Step 3: Cancelling order...");
        match client.cancel_order(symbol, &order_id, None).await {
            Ok(response) => {
                if response.header.status == "200" {
                    if let Some(order) = response.data.result {
                        info!("Order cancelled: ID={}, Status={}", order.id, order.status);
                    } else {
                        info!("Order cancellation request sent");
                    }
                } else {
                    if let Some(error) = response.data.errs {
                        warn!("Order cancellation failed: {}: {}", error.label, error.message);
                    }
                }
            }
            Err(e) => {
                error!("Failed to cancel order: {}", e);
            }
        }

        // 等待下一轮
        if round < 3 {
            info!("Waiting before next round...");
            sleep(Duration::from_secs(5)).await;
        }
    }

    info!("Trading strategy completed");
    Ok(())
}
