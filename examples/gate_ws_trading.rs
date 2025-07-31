use anyhow::Result;
use env_logger;
use log::info;
use trend_arb::trade::gate::{GateWsTradeClient, GateOrderSide, GateTimeInForce};

#[tokio::main]
async fn main() -> Result<()> {
    // 初始化日志
    env_logger::Builder::from_default_env()
        .filter_level(log::LevelFilter::Info)
        .init();

    // 创建Gate.io WebSocket客户端
    let client = GateWsTradeClient::new(
        "YOUR_API_KEY".to_string(),
        "YOUR_API_SECRET".to_string(),
        true, // 使用测试网
    );

    // 启动连接
    let client_clone = client.clone();
    let connection_task = tokio::spawn(async move {
        if let Err(e) = client_clone.start().await {
            eprintln!("Connection error: {}", e);
        }
    });

    // 等待连接和登录完成
    tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

    // 检查连接状态
    info!("Connection status: {:?}", client.get_connection_status().await);
    info!("Login status: {}", client.is_logged_in().await);

    if client.is_logged_in().await {
        // 示例：下限价单
        let mut order_id = String::new();
        match client.place_limit_order(
            "BTC_USDT",
            GateOrderSide::Buy,
            "50000.0",  // 价格
            "0.001",    // 数量
            Some(GateTimeInForce::Gtc),
            Some("test-order-001".to_string()),
            None, // 使用默认账户类型 "spot"
        ).await {
            Ok(response) => {
                info!("Limit order placed successfully:");
                info!("Request ID: {}", response.request_id);
                info!("Status: {}", response.header.status);
                if let Some(order) = response.data.result {
                    info!("Order ID: {}", order.id);
                    info!("Order Status: {}", order.status);
                    info!("Symbol: {}", order.currency_pair);
                    info!("Side: {}", order.side);
                    info!("Amount: {}", order.amount);
                    info!("Price: {}", order.price);
                    order_id = order.id;
                }
                if let Some(error) = response.data.errs {
                    info!("Error: {}: {}", error.label, error.message);
                }
            }
            Err(e) => {
                eprintln!("Failed to place limit order: {}", e);
            }
        }

        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

        // 示例：查询订单状态
        if !order_id.is_empty() {
            match client.get_order_status(
                "BTC_USDT",
                &order_id,
                None,
            ).await {
                Ok(response) => {
                    info!("Order status query successful:");
                    info!("Request ID: {}", response.request_id);
                    info!("Status: {}", response.header.status);
                    if let Some(order) = response.data.result {
                        info!("Order ID: {}", order.id);
                        info!("Order Status: {}", order.status);
                        info!("Filled: {}", order.filled_total);
                        info!("Left: {}", order.left);
                    }
                    if let Some(error) = response.data.errs {
                        info!("Error: {}: {}", error.label, error.message);
                    }
                }
                Err(e) => {
                    eprintln!("Failed to query order status: {}", e);
                }
            }

            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

            // 示例：取消订单
            match client.cancel_order(
                "BTC_USDT",
                &order_id,
                None,
            ).await {
                Ok(response) => {
                    info!("Order cancellation request sent:");
                    info!("Request ID: {}", response.request_id);
                    info!("Status: {}", response.header.status);
                    if let Some(order) = response.data.result {
                        info!("Order ID: {}", order.id);
                        info!("Order Status: {}", order.status);
                    }
                    if let Some(error) = response.data.errs {
                        info!("Error: {}: {}", error.label, error.message);
                    }
                }
                Err(e) => {
                    eprintln!("Failed to cancel order: {}", e);
                }
            }
        }

        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

        // 示例：下市价单
        match client.place_market_order(
            "ETH_USDT",
            GateOrderSide::Sell,
            "0.01",     // 数量
            Some("test-market-001".to_string()),
            None, // 使用默认账户类型 "spot"
        ).await {
            Ok(response) => {
                info!("Market order placed successfully:");
                info!("Request ID: {}", response.request_id);
                info!("Status: {}", response.header.status);
                if let Some(order) = response.data.result {
                    info!("Order ID: {}", order.id);
                    info!("Order Status: {}", order.status);
                    info!("Symbol: {}", order.currency_pair);
                    info!("Side: {}", order.side);
                    info!("Amount: {}", order.amount);
                }
                if let Some(error) = response.data.errs {
                    info!("Error: {}: {}", error.label, error.message);
                }
            }
            Err(e) => {
                eprintln!("Failed to place market order: {}", e);
            }
        }

        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

        // 示例：发送心跳
        if let Err(e) = client.ping().await {
            eprintln!("Failed to send ping: {}", e);
        } else {
            info!("Ping sent successfully");
        }
    }

    // 等待一段时间后清理
    tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
    
    // 取消连接任务
    connection_task.abort();

    Ok(())
}
