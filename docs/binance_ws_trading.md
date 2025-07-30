# Binance WebSocket Trading Client

这是一个基于Rust的Binance WebSocket API交易客户端，支持限价单和市价单交易，具有自动重连功能。

## 功能特性

- ✅ WebSocket连接管理和自动重连
- ✅ HMAC-SHA256签名认证
- ✅ 限价单下单
- ✅ 市价单下单
- ✅ 使用sonic-rs进行高性能序列化/反序列化
- ✅ 使用tokio-tungstenite的读写分离
- ✅ 完整的错误处理
- ✅ 支持测试网和生产环境

## 依赖要求

在`Cargo.toml`中添加以下依赖：

```toml
[dependencies]
tokio = { version = "1.46.1", features = ["full"] }
tokio-tungstenite = { version = "0.27.0", features = ["native-tls", "rustls-tls-native-roots"] }
sonic-rs = "0.5.3"
anyhow = "1.0.98"
log = "0.4.27"
env_logger = "0.11.8"
chrono = "0.4.41"
hmac = "0.12.1"
sha2 = "0.10.9"
hex = "0.4.3"
uuid = { version = "1.17.0", features = ["v4", "fast-rng"] }
futures-util = "0.3.31"
```

## 使用方法

### 基本设置

```rust
use anyhow::Result;
use log::info;
use trend_arb::trade::binance::{BinanceWsTradeClient, OrderSide, TimeInForce};

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();

    // 设置API密钥
    let api_key = "your_api_key".to_string();
    let secret_key = "your_secret_key".to_string();

    // 创建客户端（true表示使用测试网）
    let client = BinanceWsTradeClient::new(api_key, secret_key, true);
    
    // 启动连接
    let client_clone = client.clone();
    let connection_task = tokio::spawn(async move {
        if let Err(e) = client_clone.start().await {
            log::error!("Connection failed: {}", e);
        }
    });

    // 等待连接建立
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

    Ok(())
}
```

### 下限价单

```rust
// 下买入限价单
let response = client
    .place_limit_order(
        "BTCUSDT",                    // 交易对
        OrderSide::Buy,               // 买入
        "20000.00",                   // 价格
        "0.001",                      // 数量
        Some(TimeInForce::Gtc),       // 有效期（Good Till Cancel）
        None,                         // 客户端订单ID（可选）
        None,                         // recv_window（可选）
    )
    .await?;

println!("Order placed: {:?}", response);
```

### 下市价单

```rust
// 下卖出市价单（指定数量）
let response = client
    .place_market_order(
        "BTCUSDT",                    // 交易对
        OrderSide::Sell,              // 卖出
        Some("0.001"),                // 数量
        None,                         // quote_order_qty（二选一）
        None,                         // 客户端订单ID（可选）
        None,                         // recv_window（可选）
    )
    .await?;

// 下买入市价单（指定金额）
let response = client
    .place_market_order(
        "BTCUSDT",                    // 交易对
        OrderSide::Buy,               // 买入
        None,                         // 数量
        Some("100.00"),               // quote_order_qty（用USDT购买）
        None,                         // 客户端订单ID（可选）
        None,                         // recv_window（可选）
    )
    .await?;
```

## 环境变量配置

为了安全起见，建议使用环境变量来存储API密钥：

```bash
export BINANCE_API_KEY="your_api_key"
export BINANCE_SECRET_KEY="your_secret_key"
```

然后在代码中读取：

```rust
let api_key = std::env::var("BINANCE_API_KEY")
    .expect("Please set BINANCE_API_KEY environment variable");
let secret_key = std::env::var("BINANCE_SECRET_KEY")
    .expect("Please set BINANCE_SECRET_KEY environment variable");
```

## 自动重连

客户端内置了自动重连机制：

- 最大重连次数：10次
- 重连延迟：5秒
- 连接断开时自动尝试重连
- 24小时后连接会自动断开（Binance限制），客户端会自动重连

## 错误处理

所有的API调用都会返回`Result`类型，包含详细的错误信息：

```rust
match client.place_limit_order(...).await {
    Ok(response) => {
        if response.status == 200 {
            println!("Order successful: {:?}", response.result);
        }
    }
    Err(e) => {
        eprintln!("Order failed: {}", e);
    }
}
```

## 连接状态监控

可以查询当前连接状态：

```rust
use trend_arb::trade::binance::ConnectionStatus;

let status = client.get_connection_status().await;
match status {
    ConnectionStatus::Connected => println!("已连接"),
    ConnectionStatus::Connecting => println!("连接中"),
    ConnectionStatus::Disconnected => println!("已断开"),
    ConnectionStatus::Reconnecting => println!("重连中"),
}
```

## 注意事项

1. **测试环境**：在生产环境使用前，请先在测试网（testnet）上进行充分测试
2. **API限制**：注意Binance的API调用频率限制
3. **资金安全**：确保API密钥的安全性，建议只授予交易权限，不要授予提现权限
4. **网络稳定性**：由于WebSocket连接可能不稳定，客户端会自动处理重连，但请确保网络环境相对稳定

## 运行示例

```bash
# 设置环境变量
export BINANCE_API_KEY="your_api_key"
export BINANCE_SECRET_KEY="your_secret_key"

# 运行示例
cargo run --example binance_ws_trading
```

## API文档参考

- [Binance WebSocket API General Information](https://developers.binance.com/docs/binance-spot-api-docs/websocket-api/general-api-information)
- [Binance WebSocket Trading Requests](https://developers.binance.com/docs/binance-spot-api-docs/websocket-api/trading-requests)
