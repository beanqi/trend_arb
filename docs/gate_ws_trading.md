# Gate.io WebSocket 交易 API 实现

本文档描述了Gate.io WebSocket交易API的Rust实现，该实现参考了Binance的WebSocket API设计风格。

## 主要特性

### 1. 连接管理
- 自动重连机制
- 连接状态监控
- 心跳保持连接

### 2. 认证机制
- 先登录认证模式
- 登录后无需为每个请求签名
- HMAC-SHA512签名算法

### 3. 订单管理
- 限价单下单
- 市价单下单
- 订单取消
- 订单状态查询

### 4. 错误处理
- 完整的错误信息处理
- 请求超时处理
- 连接异常恢复

## 与Binance API的主要区别

| 特性 | Binance | Gate.io |
|-----|---------|---------|
| 认证方式 | 每个请求都需要签名 | 先登录，后续请求无需签名 |
| 签名算法 | HMAC-SHA256 | HMAC-SHA512 |
| 请求格式 | 直接参数 | API事件格式 |
| 响应格式 | 简单JSON | 带头部信息的结构化响应 |

## 使用示例

### 基本连接和登录

```rust
use trend_arb::trade::gate::{GateWsTradeClient, GateOrderSide};

let client = GateWsTradeClient::new(
    "YOUR_API_KEY".to_string(),
    "YOUR_API_SECRET".to_string(),
    false, // 生产环境
);

// 启动连接（包含自动登录）
let client_clone = client.clone();
tokio::spawn(async move {
    client_clone.start().await
});

// 等待登录完成
tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
```

### 下限价单

```rust
let response = client.place_limit_order(
    "BTC_USDT",                        // 交易对
    GateOrderSide::Buy,                // 买入
    "50000.0",                         // 价格
    "0.001",                           // 数量
    Some(GateTimeInForce::Gtc),        // 有效期类型
    Some("my-order-001".to_string()),  // 自定义订单ID
    None,                              // 账户类型（默认spot）
).await?;
```

### 下市价单

```rust
let response = client.place_market_order(
    "ETH_USDT",                       // 交易对
    GateOrderSide::Sell,              // 卖出
    "0.01",                           // 数量
    Some("my-market-001".to_string()), // 自定义订单ID
    None,                             // 账户类型（默认spot）
).await?;
```

### 取消订单

```rust
let response = client.cancel_order(
    "BTC_USDT",        // 交易对
    "order_id_123",    // 订单ID
    None,              // 账户类型（默认spot）
).await?;
```

### 查询订单状态

```rust
let response = client.get_order_status(
    "BTC_USDT",        // 交易对
    "order_id_123",    // 订单ID
    None,              // 账户类型（默认spot）
).await?;
```

## API方法列表

### 连接管理
- `new()` - 创建客户端实例
- `start()` - 启动连接和登录
- `get_connection_status()` - 获取连接状态
- `is_logged_in()` - 检查登录状态
- `reconnect()` - 手动重连
- `ping()` - 发送心跳

### 交易操作
- `place_limit_order()` - 下限价单
- `place_market_order()` - 下市价单
- `cancel_order()` - 取消订单
- `get_order_status()` - 查询订单状态

## 数据结构

### 订单类型 (GateOrderType)
- `Limit` - 限价单
- `Market` - 市价单

### 订单方向 (GateOrderSide)
- `Buy` - 买入
- `Sell` - 卖出

### 有效期类型 (GateTimeInForce)
- `Gtc` - Good Till Cancel (一直有效直到取消)
- `Ioc` - Immediate or Cancel (立即成交或取消)
- `Fok` - Fill or Kill (全部成交或全部取消)
- `Poc` - Pending or Cancel (被动委托，只挂单不吃单)

### 处理模式 (GateActionMode)
- `Ack` - 异步模式，只返回订单关键字段
- `Result` - 无清算信息
- `Full` - 完整模式（默认）

### 连接状态 (ConnectionStatus)
- `Disconnected` - 已断开
- `Connecting` - 连接中
- `Connected` - 已连接但未登录
- `LoggedIn` - 已登录
- `Reconnecting` - 重连中

## 错误处理

所有API方法都返回`Result<T, anyhow::Error>`，可以使用`?`操作符进行错误传播：

```rust
match client.place_limit_order(...).await {
    Ok(response) => {
        if response.header.status == "200" {
            // 成功
            if let Some(order) = response.data.result {
                println!("Order placed: {}", order.id);
            }
        } else {
            // 业务错误
            if let Some(error) = response.data.errs {
                eprintln!("Business error: {}: {}", error.label, error.message);
            }
        }
    }
    Err(e) => {
        // 系统错误
        eprintln!("System error: {}", e);
    }
}
```

## 重连机制

客户端实现了自动重连功能：

- 连接断开时自动重试
- 指数退避算法控制重试间隔
- 可配置最大重试次数
- 重连成功后自动重新登录

## 心跳机制

建议定期调用`ping()`方法保持连接活跃：

```rust
// 每30秒发送一次心跳
let client_clone = client.clone();
tokio::spawn(async move {
    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(30));
    loop {
        interval.tick().await;
        if let Err(e) = client_clone.ping().await {
            eprintln!("Ping failed: {}", e);
        }
    }
});
```

## 注意事项

1. **API密钥权限**：确保API密钥具有现货交易权限
2. **IP白名单**：如果启用了IP白名单，确保客户端IP在白名单中
3. **连接状态**：下单前检查连接和登录状态
4. **错误处理**：妥善处理网络错误和业务错误
5. **资源清理**：程序退出时正确关闭连接

## 测试

运行基础示例程序：

```bash
# 设置你的API密钥
export GATE_API_KEY="your_api_key"
export GATE_API_SECRET="your_api_secret"

# 运行基础示例
cargo run --example gate_ws_trading
```

运行高级交易示例：

```bash
# 运行高级示例（包含完整的交易流程）
cargo run --example gate_advanced_trading
```

运行单元测试：

```bash
cargo test gate_
```

## 示例程序说明

### gate_ws_trading.rs
基础示例程序，展示了：
- 基本的连接和登录
- 下限价单和市价单
- 查询订单状态
- 取消订单
- 发送心跳

### gate_advanced_trading.rs
高级示例程序，展示了：
- 连接管理和自动重连
- 心跳保持机制
- 完整的交易策略流程
- 错误处理和恢复
- 多轮交易示例

高级示例包含了一个简单的交易策略：
1. 下限价买单（低于市价，不会立即成交）
2. 查询订单状态
3. 取消订单
4. 重复多轮交易

这个示例展示了如何在生产环境中正确使用Gate.io WebSocket API。
