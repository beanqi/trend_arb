use anyhow::Result;
use env_logger;
use log::info;
use std::env;
use tokio::time::{sleep, Duration};
use trend_arb::trade::okx::{
    OkxWsTradeClient, OkxOrderSide, OkxTradeMode, ConnectionStatus,
    OkxPlaceOrderRequest, OkxCancelOrderRequest,
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

    // 演示批量下单功能
    info!("演示批量下单");
    let batch_orders = vec![
        OkxPlaceOrderRequest {
            inst_id: "BTC-USDT".to_string(),
            td_mode: "cash".to_string(),
            side: "buy".to_string(),
            ord_type: "limit".to_string(),
            sz: "0.001".to_string(),
            px: Some("39000.0".to_string()),
            pos_side: None,
            cl_ord_id: Some("batch_order_001".to_string()),
            tag: Some("batch".to_string()),
            reduce_only: None,
        },
        OkxPlaceOrderRequest {
            inst_id: "BTC-USDT".to_string(),
            td_mode: "cash".to_string(),
            side: "buy".to_string(),
            ord_type: "limit".to_string(),
            sz: "0.001".to_string(),
            px: Some("38000.0".to_string()),
            pos_side: None,
            cl_ord_id: Some("batch_order_002".to_string()),
            tag: Some("batch".to_string()),
            reduce_only: None,
        },
        OkxPlaceOrderRequest {
            inst_id: "ETH-USDT".to_string(),
            td_mode: "cash".to_string(),
            side: "buy".to_string(),
            ord_type: "limit".to_string(),
            sz: "0.01".to_string(),
            px: Some("2500.0".to_string()),
            pos_side: None,
            cl_ord_id: Some("batch_order_003".to_string()),
            tag: Some("batch".to_string()),
            reduce_only: None,
        },
    ];

    match client.place_multiple_orders(batch_orders).await {
        Ok(response) => {
            info!("批量下单响应: {:?}", response);
            if let Some(data) = response.data {
                for (i, order) in data.iter().enumerate() {
                    info!(
                        "订单 {}: ID={}, 客户端ID={:?}, 状态={}, 消息={}",
                        i + 1,
                        order.ord_id,
                        order.cl_ord_id,
                        order.s_code,
                        order.s_msg
                    );
                }
            }
        }
        Err(e) => {
            log::error!("批量下单失败: {}", e);
        }
    }

    sleep(Duration::from_millis(500)).await;

    // 演示单个撤单
    info!("演示撤单功能");
    match client
        .cancel_order(
            "BTC-USDT",
            None,  // 不使用订单ID
            Some("batch_order_001".to_string()),  // 使用客户端订单ID
        )
        .await
    {
        Ok(response) => {
            info!("撤单成功: {:?}", response);
            if let Some(data) = response.data {
                for order in data {
                    info!(
                        "撤单结果: ID={}, 客户端ID={:?}, 状态={}, 消息={}",
                        order.ord_id,
                        order.cl_ord_id,
                        order.s_code,
                        order.s_msg
                    );
                }
            }
        }
        Err(e) => {
            log::error!("撤单失败: {}", e);
        }
    }

    sleep(Duration::from_millis(500)).await;

    // 演示批量撤单
    info!("演示批量撤单");
    let cancel_orders = vec![
        OkxCancelOrderRequest {
            inst_id: "BTC-USDT".to_string(),
            ord_id: None,
            cl_ord_id: Some("batch_order_002".to_string()),
        },
        OkxCancelOrderRequest {
            inst_id: "ETH-USDT".to_string(),
            ord_id: None,
            cl_ord_id: Some("batch_order_003".to_string()),
        },
    ];

    match client.cancel_multiple_orders(cancel_orders).await {
        Ok(response) => {
            info!("批量撤单成功: {:?}", response);
            if let Some(data) = response.data {
                for (i, order) in data.iter().enumerate() {
                    info!(
                        "撤单 {}: ID={}, 客户端ID={:?}, 状态={}, 消息={}",
                        i + 1,
                        order.ord_id,
                        order.cl_ord_id,
                        order.s_code,
                        order.s_msg
                    );
                }
            }
        }
        Err(e) => {
            log::error!("批量撤单失败: {}", e);
        }
    }

    sleep(Duration::from_millis(500)).await;

    // 演示修改订单
    info!("演示修改订单功能");
    
    // 先下一个订单
    let response = client
        .place_limit_order(
            "BTC-USDT",
            OkxOrderSide::Buy,
            OkxTradeMode::Cash,
            "37000.0",
            "0.001",
            None,
            Some("amend_test_001".to_string()),
            Some("amend".to_string()),
            None,
        )
        .await?;

    if let Some(data) = response.data {
        if let Some(order) = data.first() {
            if order.s_code == "0" {
                info!("订单创建成功，准备修改");
                
                sleep(Duration::from_millis(1000)).await;

                // 修改订单价格和数量
                match client
                    .amend_order(
                        "BTC-USDT",
                        None,
                        Some("amend_test_001".to_string()),
                        Some("0.002".to_string()),  // 新数量
                        Some("36000.0".to_string()), // 新价格
                    )
                    .await
                {
                    Ok(amend_response) => {
                        info!("修改订单成功: {:?}", amend_response);
                        if let Some(amend_data) = amend_response.data {
                            for order in amend_data {
                                info!(
                                    "修改结果: ID={}, 客户端ID={:?}, 状态={}, 消息={}",
                                    order.ord_id,
                                    order.cl_ord_id,
                                    order.s_code,
                                    order.s_msg
                                );
                            }
                        }
                    }
                    Err(e) => {
                        log::error!("修改订单失败: {}", e);
                    }
                }

                sleep(Duration::from_millis(1000)).await;

                // 最后撤销这个订单
                let _ = client
                    .cancel_order(
                        "BTC-USDT",
                        None,
                        Some("amend_test_001".to_string()),
                    )
                    .await;
            }
        }
    }

    // 演示合约批量操作
    info!("演示合约批量下单");
    let contract_orders = vec![
        OkxPlaceOrderRequest {
            inst_id: "BTC-USDT-SWAP".to_string(),
            td_mode: "cross".to_string(),
            side: "buy".to_string(),
            ord_type: "limit".to_string(),
            sz: "1".to_string(),
            px: Some("39000.0".to_string()),
            pos_side: Some("long".to_string()),
            cl_ord_id: Some("contract_batch_001".to_string()),
            tag: Some("contract_batch".to_string()),
            reduce_only: None,
        },
        OkxPlaceOrderRequest {
            inst_id: "ETH-USDT-SWAP".to_string(),
            td_mode: "cross".to_string(),
            side: "buy".to_string(),
            ord_type: "limit".to_string(),
            sz: "1".to_string(),
            px: Some("2500.0".to_string()),
            pos_side: Some("long".to_string()),
            cl_ord_id: Some("contract_batch_002".to_string()),
            tag: Some("contract_batch".to_string()),
            reduce_only: None,
        },
    ];

    match client.place_multiple_orders(contract_orders).await {
        Ok(response) => {
            info!("合约批量下单成功: {:?}", response);
            if let Some(data) = response.data {
                for (i, order) in data.iter().enumerate() {
                    info!(
                        "合约订单 {}: ID={}, 客户端ID={:?}, 状态={}, 消息={}",
                        i + 1,
                        order.ord_id,
                        order.cl_ord_id,
                        order.s_code,
                        order.s_msg
                    );
                }
            }
        }
        Err(e) => {
            log::error!("合约批量下单失败: {}", e);
        }
    }

    sleep(Duration::from_millis(1000)).await;

    // 撤销合约订单
    let contract_cancel_orders = vec![
        OkxCancelOrderRequest {
            inst_id: "BTC-USDT-SWAP".to_string(),
            ord_id: None,
            cl_ord_id: Some("contract_batch_001".to_string()),
        },
        OkxCancelOrderRequest {
            inst_id: "ETH-USDT-SWAP".to_string(),
            ord_id: None,
            cl_ord_id: Some("contract_batch_002".to_string()),
        },
    ];

    let _ = client.cancel_multiple_orders(contract_cancel_orders).await;

    info!("演示完毕，保持连接5秒");
    sleep(Duration::from_secs(5)).await;

    // 取消连接任务
    connection_task.abort();
    
    info!("程序结束");
    Ok(())
}
