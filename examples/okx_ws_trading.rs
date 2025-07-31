use anyhow::Result;
use env_logger;
use log::info;
use std::env;
use tokio::time::{sleep, Duration};
use trend_arb::trade::okx::{
    OkxWsTradeClient, OkxOrderSide, OkxTradeMode, OkxPositionSide, ConnectionStatus,
};

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();

    // 从环境变量获取API凭证
    let api_key = env::var("OKX_API_KEY").unwrap_or_else(|_| "your_api_key".to_string());
    let secret_key = env::var("OKX_SECRET_KEY").unwrap_or_else(|_| "your_secret_key".to_string());
    let passphrase = env::var("OKX_PASSPHRASE").unwrap_or_else(|_| "your_passphrase".to_string());
    let testnet = env::var("OKX_TESTNET")
        .unwrap_or_else(|_| "true".to_string())
        .parse::<bool>()
        .unwrap_or(true);

    info!("创建OKX WebSocket交易客户端");
    let client = OkxWsTradeClient::new(api_key, secret_key, passphrase, testnet);

    info!("启动WebSocket连接并登录");
    // 在后台启动连接
    let client_clone = client.clone();
    let connection_task = tokio::spawn(async move {
        if let Err(e) = client_clone.start().await {
            log::error!("连接失败: {}", e);
        }
    });

    // 等待连接和登录完成
    let mut attempts = 0;
    while attempts < 100 {  // 最多等待10秒
        let status = client.get_connection_status().await;
        match status {
            ConnectionStatus::LoggedIn => {
                info!("已成功登录，可以开始交易");
                break;
            }
            ConnectionStatus::Disconnected => {
                log::error!("连接失败");
                return Ok(());
            }
            _ => {
                info!("当前状态: {:?}", status);
                sleep(Duration::from_millis(100)).await;
            }
        }
        attempts += 1;
    }

    if client.get_connection_status().await != ConnectionStatus::LoggedIn {
        log::error!("登录超时");
        return Ok(());
    }

    // 演示下单功能
    info!("演示限价单下单");
    match client
        .place_limit_order(
            "BTC-USDT",                    // 交易对
            OkxOrderSide::Buy,             // 买入
            OkxTradeMode::Cash,            // 现货模式
            "40000.0",                     // 价格
            "0.001",                       // 数量
            None,                          // 持仓方向（现货不需要）
            Some("test_order_001".to_string()), // 客户端订单ID
            Some("demo".to_string()),      // 标签
            None,                          // 只减仓
        )
        .await
    {
        Ok(response) => {
            info!("限价单下单成功: {:?}", response);
            if let Some(data) = response.data {
                for order in data {
                    info!(
                        "订单ID: {}, 客户端订单ID: {:?}, 状态: {}, 消息: {}",
                        order.ord_id,
                        order.cl_ord_id,
                        order.s_code,
                        order.s_msg
                    );
                }
            }
        }
        Err(e) => {
            log::error!("限价单下单失败: {}", e);
        }
    }

    // 演示市价单
    info!("演示市价单下单");
    match client
        .place_market_order(
            "BTC-USDT",                    // 交易对
            OkxOrderSide::Sell,            // 卖出
            OkxTradeMode::Cash,            // 现货模式
            "0.001",                       // 数量
            None,                          // 持仓方向（现货不需要）
            Some("test_order_002".to_string()), // 客户端订单ID
            Some("demo".to_string()),      // 标签
            None,                          // 只减仓
        )
        .await
    {
        Ok(response) => {
            info!("市价单下单成功: {:?}", response);
            if let Some(data) = response.data {
                for order in data {
                    info!(
                        "订单ID: {}, 客户端订单ID: {:?}, 状态: {}, 消息: {}",
                        order.ord_id,
                        order.cl_ord_id,
                        order.s_code,
                        order.s_msg
                    );
                }
            }
        }
        Err(e) => {
            log::error!("市价单下单失败: {}", e);
        }
    }

    // 演示POST_ONLY单（只做maker）
    info!("演示POST_ONLY单下单");
    match client
        .place_post_only_order(
            "BTC-USDT",                    // 交易对
            OkxOrderSide::Buy,             // 买入
            OkxTradeMode::Cash,            // 现货模式
            "39000.0",                     // 价格（比市价低，确保是maker）
            "0.001",                       // 数量
            None,                          // 持仓方向（现货不需要）
            Some("test_order_003".to_string()), // 客户端订单ID
            Some("demo".to_string()),      // 标签
            None,                          // 只减仓
        )
        .await
    {
        Ok(response) => {
            info!("POST_ONLY单下单成功: {:?}", response);
            if let Some(data) = response.data {
                for order in data {
                    info!(
                        "订单ID: {}, 客户端订单ID: {:?}, 状态: {}, 消息: {}",
                        order.ord_id,
                        order.cl_ord_id,
                        order.s_code,
                        order.s_msg
                    );
                }
            }
        }
        Err(e) => {
            log::error!("POST_ONLY单下单失败: {}", e);
        }
    }

    // 演示合约交易
    info!("演示合约交易 - 开多单");
    match client
        .place_limit_order(
            "BTC-USDT-SWAP",               // 永续合约
            OkxOrderSide::Buy,             // 买入开多
            OkxTradeMode::Cross,           // 全仓模式
            "40000.0",                     // 价格
            "1",                           // 数量（张数）
            Some(OkxPositionSide::Long),   // 开多
            Some("test_swap_001".to_string()), // 客户端订单ID
            Some("demo".to_string()),      // 标签
            None,                          // 只减仓
        )
        .await
    {
        Ok(response) => {
            info!("合约开多单下单成功: {:?}", response);
            if let Some(data) = response.data {
                for order in data {
                    info!(
                        "订单ID: {}, 客户端订单ID: {:?}, 状态: {}, 消息: {}",
                        order.ord_id,
                        order.cl_ord_id,
                        order.s_code,
                        order.s_msg
                    );
                }
            }
        }
        Err(e) => {
            log::error!("合约开多单下单失败: {}", e);
        }
    }

    info!("演示完毕，保持连接5秒");
    sleep(Duration::from_secs(5)).await;

    // 取消连接任务
    connection_task.abort();
    
    info!("程序结束");
    Ok(())
}
