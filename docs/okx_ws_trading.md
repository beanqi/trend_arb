# OKX WebSocket 交易 API 使用指南

## 概述

本项目实现了OKX交易所的WebSocket交易API，提供了完整的下单功能和重连机制。与Binance API不同，OKX API需要先登录认证，登录后的交易请求不需要签名。

## 主要特性

- 自动重连机制
- 支持现货和合约交易
- 支持多种订单类型（限价、市价、POST_ONLY等）
- 完整的错误处理
- 线程安全的设计
- 详细的日志记录

## 快速开始

### 1. 设置环境变量

```bash
export OKX_API_KEY="your_api_key"
export OKX_SECRET_KEY="your_secret_key"
export OKX_PASSPHRASE="your_passphrase"
export OKX_TESTNET="true"  # 设置为false使用实盘
```

### 2. 基本使用

```rust
use trend_arb::trade::okx::{
    OkxWsTradeClient, OkxOrderSide, OkxTradeMode, ConnectionStatus,
};

#[tokio::main]
async fn main() -> Result<()> {
    // 创建客户端
    let client = OkxWsTradeClient::new(
        api_key,
        secret_key,
        passphrase,
        true // testnet
    );

    // 启动连接（在后台）
    let client_clone = client.clone();
    tokio::spawn(async move {
        client_clone.start().await
    });

    // 等待登录完成
    while client.get_connection_status().await != ConnectionStatus::LoggedIn {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // 现在可以开始交易
    let response = client.place_limit_order(
        "BTC-USDT",
        OkxOrderSide::Buy,
        OkxTradeMode::Cash,
        "40000.0",
        "0.001",
        None,
        Some("my_order_001".to_string()),
        Some("demo".to_string()),
        None,
    ).await?;

    Ok(())
}
```

## API参考

### 创建客户端

```rust
pub fn new(
    api_key: String,
    secret_key: String,
    passphrase: String,
    testnet: bool
) -> Self
```

- `api_key`: OKX API Key
- `secret_key`: OKX Secret Key
- `passphrase`: OKX API Passphrase
- `testnet`: 是否使用测试网

### 订单类型

#### 限价单

```rust
pub async fn place_limit_order(
    &self,
    inst_id: &str,                          // 交易对，如 "BTC-USDT"
    side: OkxOrderSide,                     // 买卖方向
    td_mode: OkxTradeMode,                  // 交易模式
    price: &str,                            // 价格
    size: &str,                             // 数量
    pos_side: Option<OkxPositionSide>,      // 持仓方向（合约）
    client_order_id: Option<String>,        // 客户端订单ID
    tag: Option<String>,                    // 订单标签
    reduce_only: Option<bool>,              // 只减仓
) -> Result<OkxWebSocketResponse<OkxOrderResponse>>
```

#### 市价单

```rust
pub async fn place_market_order(
    &self,
    inst_id: &str,                          // 交易对
    side: OkxOrderSide,                     // 买卖方向
    td_mode: OkxTradeMode,                  // 交易模式
    size: &str,                             // 数量
    pos_side: Option<OkxPositionSide>,      // 持仓方向（合约）
    client_order_id: Option<String>,        // 客户端订单ID
    tag: Option<String>,                    // 订单标签
    reduce_only: Option<bool>,              // 只减仓
) -> Result<OkxWebSocketResponse<OkxOrderResponse>>
```

#### POST_ONLY单

```rust
pub async fn place_post_only_order(
    &self,
    inst_id: &str,                          // 交易对
    side: OkxOrderSide,                     // 买卖方向
    td_mode: OkxTradeMode,                  // 交易模式
    price: &str,                            // 价格
    size: &str,                             // 数量
    pos_side: Option<OkxPositionSide>,      // 持仓方向（合约）
    client_order_id: Option<String>,        // 客户端订单ID
    tag: Option<String>,                    // 订单标签
    reduce_only: Option<bool>,              // 只减仓
) -> Result<OkxWebSocketResponse<OkxOrderResponse>>
```

### 枚举类型

#### 订单方向 (OkxOrderSide)
- `Buy`: 买入
- `Sell`: 卖出

#### 交易模式 (OkxTradeMode)
- `Cash`: 现货模式
- `Cross`: 全仓模式
- `Isolated`: 逐仓模式

#### 持仓方向 (OkxPositionSide)
- `Net`: 买卖模式（现货）
- `Long`: 开多/平空
- `Short`: 开空/平多

#### 连接状态 (ConnectionStatus)
- `Disconnected`: 未连接
- `Connecting`: 连接中
- `Connected`: 已连接但未登录
- `LoggingIn`: 登录中
- `LoggedIn`: 已登录，可以交易
- `Reconnecting`: 重连中

## 使用示例

### 现货交易

```rust
// 现货限价买单
let response = client.place_limit_order(
    "BTC-USDT",
    OkxOrderSide::Buy,
    OkxTradeMode::Cash,
    "40000.0",
    "0.001",
    None,  // 现货不需要持仓方向
    Some("spot_buy_001".to_string()),
    Some("spot".to_string()),
    None,
).await?;

// 现货市价卖单
let response = client.place_market_order(
    "BTC-USDT",
    OkxOrderSide::Sell,
    OkxTradeMode::Cash,
    "0.001",
    None,
    Some("spot_sell_001".to_string()),
    Some("spot".to_string()),
    None,
).await?;
```

### 合约交易

```rust
// 永续合约开多单
let response = client.place_limit_order(
    "BTC-USDT-SWAP",
    OkxOrderSide::Buy,
    OkxTradeMode::Cross,
    "40000.0",
    "1",  // 1张合约
    Some(OkxPositionSide::Long),  // 开多
    Some("swap_long_001".to_string()),
    Some("futures".to_string()),
    None,
).await?;

// 永续合约平多单（只减仓）
let response = client.place_market_order(
    "BTC-USDT-SWAP",
    OkxOrderSide::Sell,
    OkxTradeMode::Cross,
    "1",
    Some(OkxPositionSide::Long),  // 平多
    Some("swap_close_001".to_string()),
    Some("futures".to_string()),
    Some(true),  // 只减仓
).await?;
```

### 错误处理

```rust
match client.place_limit_order(...).await {
    Ok(response) => {
        if response.code == "0" {
            println!("下单成功");
            if let Some(data) = response.data {
                for order in data {
                    if order.s_code == "0" {
                        println!("订单提交成功: {}", order.ord_id);
                    } else {
                        println!("订单失败: {}", order.s_msg);
                    }
                }
            }
        } else {
            println!("API错误: {}", response.msg);
        }
    }
    Err(e) => {
        println!("请求失败: {}", e);
    }
}
```

## 重连机制

客户端内置了自动重连机制：

- 连接断开时自动重连
- 最大重连次数：6000次
- 重连间隔：1秒
- 重连成功后自动重新登录

```rust
// 手动触发重连
client.reconnect().await?;

// 检查连接状态
let status = client.get_connection_status().await;
println!("当前状态: {:?}", status);
```

## 注意事项

1. **登录认证**: OKX需要先登录，登录后的请求不需要签名
2. **现货vs合约**: 现货交易不需要设置持仓方向，合约交易需要
3. **交易模式**: 现货使用`Cash`，合约使用`Cross`或`Isolated`
4. **数量单位**: 现货以币为单位，合约以张为单位
5. **客户端订单ID**: 建议使用唯一的客户端订单ID便于追踪
6. **错误处理**: 需要检查API级别和订单级别的错误

## 运行示例

```bash
# 设置环境变量
export OKX_API_KEY="your_api_key"
export OKX_SECRET_KEY="your_secret_key"
export OKX_PASSPHRASE="your_passphrase"
export OKX_TESTNET="true"

# 运行示例
cargo run --example okx_ws_trading
```

## 与Binance API的对比

| 特性 | Binance | OKX |
|------|---------|-----|
| 认证方式 | 每个请求签名 | 登录一次，后续请求不签名 |
| 重连处理 | 需要重新签名 | 需要重新登录 |
| 现货交易 | 支持 | 支持 |
| 合约交易 | 需要不同端点 | 统一端点，通过参数区分 |
| 订单类型 | 限价/市价/FOK/IOC | 限价/市价/POST_ONLY/FOK/IOC |

## 技术细节

- 使用`tokio-tungstenite`进行WebSocket通信
- 使用`sonic_rs`进行JSON序列化/反序列化
- 使用`hmac`和`sha2`进行签名计算
- 使用`base64`进行签名编码
- 线程安全设计，支持并发访问
