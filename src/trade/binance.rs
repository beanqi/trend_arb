use anyhow::{anyhow, Result};
use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use hmac::{Hmac, Mac};
use log::{debug, error, info, warn};
use sha2::Sha256;
use sonic_rs::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, Mutex, RwLock};
use tokio::time::{sleep, timeout};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;

/// Binance WebSocket API base URL
const WS_API_URL: &str = "wss://ws-api.binance.com:443/ws-api/v3";
const WS_API_TESTNET_URL: &str = "wss://ws-api.testnet.binance.vision/ws-api/v3";

/// 订单类型
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum OrderType {
    Limit,
    Market,
}

/// 订单方向
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum OrderSide {
    Buy,
    Sell,
}

/// Time in Force
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum TimeInForce {
    Gtc, // Good Till Cancel
    Ioc, // Immediate or Cancel
    Fok, // Fill or Kill
}

/// 订单响应类型
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum NewOrderRespType {
    Ack,
    Result,
    Full,
}

/// 限价单请求参数
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LimitOrderRequest {
    pub symbol: String,
    pub side: OrderSide,
    #[serde(rename = "type")]
    pub order_type: OrderType,
    pub time_in_force: TimeInForce,
    pub price: String,
    pub quantity: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_client_order_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_order_resp_type: Option<NewOrderRespType>,
    pub api_key: String,
    pub timestamp: u64,
    pub signature: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recv_window: Option<u64>,
}

/// 市价单请求参数
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketOrderRequest {
    pub symbol: String,
    pub side: OrderSide,
    #[serde(rename = "type")]
    pub order_type: OrderType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quote_order_qty: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_client_order_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_order_resp_type: Option<NewOrderRespType>,
    pub api_key: String,
    pub timestamp: u64,
    pub signature: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recv_window: Option<u64>,
}

/// WebSocket请求格式
#[derive(Debug, Clone, Serialize)]
pub struct WebSocketRequest<T> {
    pub id: String,
    pub method: String,
    pub params: T,
}

/// 订单响应基础信息
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrderResponse {
    pub symbol: String,
    pub order_id: u64,
    pub order_list_id: i64,
    pub client_order_id: String,
    pub transact_time: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub price: Option<String>,
    pub orig_qty: String,
    pub executed_qty: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub orig_quote_order_qty: Option<String>,
    pub cummulative_quote_qty: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_in_force: Option<String>,
    #[serde(rename = "type")]
    pub order_type: String,
    pub side: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub working_time: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub self_trade_prevention_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fills: Option<Vec<Fill>>,
}

/// 成交详情
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Fill {
    pub price: String,
    pub qty: String,
    pub commission: String,
    pub commission_asset: String,
    pub trade_id: i64,
}

/// Rate Limit 信息
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RateLimit {
    pub rate_limit_type: String,
    pub interval: String,
    pub interval_num: u32,
    pub limit: u32,
    pub count: u32,
}

/// WebSocket响应格式
#[derive(Debug, Clone, Deserialize)]
pub struct WebSocketResponse<T> {
    pub id: String,
    pub status: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ApiError>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate_limits: Option<Vec<RateLimit>>,
}

/// API错误信息
#[derive(Debug, Clone, Deserialize)]
pub struct ApiError {
    pub code: i32,
    pub msg: String,
}

/// 连接状态
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ConnectionStatus {
    Disconnected,
    Connecting,
    Connected,
    Reconnecting,
}

/// Binance WebSocket交易客户端
#[derive(Clone)]
pub struct BinanceWsTradeClient {
    api_key: String,
    secret_key: String,
    testnet: bool,
    connection_status: Arc<RwLock<ConnectionStatus>>,
    pending_requests: Arc<Mutex<std::collections::HashMap<String, mpsc::UnboundedSender<String>>>>,
    reconnect_attempts: Arc<Mutex<u32>>,
    max_reconnect_attempts: u32,
    reconnect_delay: Duration,
    message_sender: Arc<Mutex<Option<mpsc::UnboundedSender<Message>>>>,
}

impl BinanceWsTradeClient {
    /// 创建新的Binance WebSocket交易客户端
    pub fn new(api_key: String, secret_key: String, testnet: bool) -> Self {
        Self {
            api_key,
            secret_key,
            testnet,
            connection_status: Arc::new(RwLock::new(ConnectionStatus::Disconnected)),
            pending_requests: Arc::new(Mutex::new(std::collections::HashMap::new())),
            reconnect_attempts: Arc::new(Mutex::new(0)),
            max_reconnect_attempts: 10,
            reconnect_delay: Duration::from_secs(5),
            message_sender: Arc::new(Mutex::new(None)),
        }
    }

    /// 启动WebSocket连接
    pub async fn start(&self) -> Result<()> {
        loop {
            match self.connect().await {
                Ok(_) => {
                    info!("WebSocket connection established");
                    // 重置重连计数
                    *self.reconnect_attempts.lock().await = 0;
                }
                Err(e) => {
                    let mut attempts = self.reconnect_attempts.lock().await;
                    *attempts += 1;
                    
                    if *attempts > self.max_reconnect_attempts {
                        error!("Max reconnection attempts reached, giving up");
                        return Err(anyhow!("Failed to establish connection after {} attempts", self.max_reconnect_attempts));
                    }
                    
                    warn!("Connection failed (attempt {}), retrying in {:?}: {}", *attempts, self.reconnect_delay, e);
                    
                    *self.connection_status.write().await = ConnectionStatus::Reconnecting;
                    sleep(self.reconnect_delay).await;
                }
            }
        }
    }

    /// 建立WebSocket连接
    async fn connect(&self) -> Result<()> {
        let url = if self.testnet {
            WS_API_TESTNET_URL
        } else {
            WS_API_URL
        };

        info!("Connecting to Binance WebSocket API: {}", url);
        *self.connection_status.write().await = ConnectionStatus::Connecting;

        let (ws_stream, _) = connect_async(url).await?;
        
        *self.connection_status.write().await = ConnectionStatus::Connected;
        info!("WebSocket connection established successfully");

        // 分离读写流
        let (mut write, mut read) = ws_stream.split();
        
        // 创建通道用于发送消息
        let (tx, mut rx) = mpsc::unbounded_channel::<Message>();
        
        // 保存消息发送器
        *self.message_sender.lock().await = Some(tx.clone());
        
        // 克隆共享状态
        let pending_requests = self.pending_requests.clone();
        let connection_status = self.connection_status.clone();
        let message_sender = self.message_sender.clone();
        
        // 启动写入任务
        let write_task = tokio::spawn(async move {
            while let Some(message) = rx.recv().await {
                if let Err(e) = write.send(message).await {
                    error!("Failed to send WebSocket message: {}", e);
                    break;
                }
            }
            // 清理消息发送器
            *message_sender.lock().await = None;
        });

        // 启动读取任务
        let read_task = tokio::spawn(async move {
            while let Some(message) = read.next().await {
                match message {
                    Ok(Message::Text(text)) => {
                        debug!("Received WebSocket message: {}", text);
                        
                        // 解析响应并分发到对应的请求处理器
                        if let Ok(response) = sonic_rs::from_str::<serde_json::Value>(&text) {
                            if let Some(id) = response.get("id").and_then(|v| v.as_str()) {
                                let mut pending = pending_requests.lock().await;
                                if let Some(sender) = pending.remove(id) {
                                    let _ = sender.send(text.to_string());
                                }
                            }
                        }
                    }
                    Ok(Message::Ping(data)) => {
                        debug!("Received ping, sending pong");
                        if let Err(e) = tx.send(Message::Pong(data)) {
                            error!("Failed to send pong: {}", e);
                            break;
                        }
                    }
                    Ok(Message::Close(_)) => {
                        info!("WebSocket connection closed by server");
                        break;
                    }
                    Err(e) => {
                        error!("WebSocket error: {}", e);
                        break;
                    }
                    _ => {}
                }
            }
            
            *connection_status.write().await = ConnectionStatus::Disconnected;
        });

        // 等待任务完成
        tokio::select! {
            _ = write_task => {
                warn!("Write task completed");
            }
            _ = read_task => {
                warn!("Read task completed");
            }
        }

        Ok(())
    }

    /// 发送WebSocket请求并等待响应
    async fn send_request<T, R>(&self, method: &str, params: T) -> Result<WebSocketResponse<R>>
    where
        T: Serialize,
        R: for<'de> Deserialize<'de>,
    {
        // 检查连接状态
        if *self.connection_status.read().await != ConnectionStatus::Connected {
            return Err(anyhow!("WebSocket not connected"));
        }

        let request_id = Uuid::new_v4().to_string();
        let request = WebSocketRequest {
            id: request_id.clone(),
            method: method.to_string(),
            params,
        };

        let request_json = sonic_rs::to_string(&request)?;
        debug!("Sending WebSocket request: {}", request_json);

        // 创建响应通道
        let (response_tx, mut response_rx) = mpsc::unbounded_channel();
        
        // 注册pending request
        {
            let mut pending = self.pending_requests.lock().await;
            pending.insert(request_id.clone(), response_tx);
        }

        // 发送请求到WebSocket
        {
            let sender_guard = self.message_sender.lock().await;
            if let Some(sender) = sender_guard.as_ref() {
                sender.send(Message::Text(request_json.into()))?;
            } else {
                return Err(anyhow!("WebSocket sender not available"));
            }
        }
        
        // 等待响应
        let response_text = timeout(Duration::from_secs(30), response_rx.recv())
            .await?
            .ok_or_else(|| anyhow!("No response received"))?;

        let response: WebSocketResponse<R> = sonic_rs::from_str(&response_text)?;
        
        if response.status != 200 {
            if let Some(error) = response.error {
                return Err(anyhow!("API Error {}: {}", error.code, error.msg));
            } else {
                return Err(anyhow!("Request failed with status: {}", response.status));
            }
        }

        Ok(response)
    }

    /// 生成签名
    fn generate_signature(&self, query_string: &str) -> Result<String> {
        let mut mac = HmacSha256::new_from_slice(self.secret_key.as_bytes())?;
        mac.update(query_string.as_bytes());
        let result = mac.finalize();
        Ok(hex::encode(result.into_bytes()))
    }

    /// 下限价单
    pub async fn place_limit_order(
        &self,
        symbol: &str,
        side: OrderSide,
        price: &str,
        quantity: &str,
        time_in_force: Option<TimeInForce>,
        client_order_id: Option<String>,
        recv_window: Option<u64>,
    ) -> Result<WebSocketResponse<OrderResponse>> {
        let timestamp = Utc::now().timestamp_millis() as u64;
        let time_in_force = time_in_force.unwrap_or(TimeInForce::Gtc);
        
        // 构建签名参数
        let mut params = vec![
            ("symbol", symbol.to_string()),
            ("side", sonic_rs::to_string(&side)?),
            ("type", "LIMIT".to_string()),
            ("timeInForce", sonic_rs::to_string(&time_in_force)?),
            ("price", price.to_string()),
            ("quantity", quantity.to_string()),
            ("apiKey", self.api_key.clone()),
            ("timestamp", timestamp.to_string()),
        ];

        if let Some(client_id) = &client_order_id {
            params.push(("newClientOrderId", client_id.clone()));
        }

        if let Some(recv_win) = recv_window {
            params.push(("recvWindow", recv_win.to_string()));
        }

        // 生成查询字符串并签名
        let query_string = params
            .iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect::<Vec<_>>()
            .join("&");

        let signature = self.generate_signature(&query_string)?;

        let request = LimitOrderRequest {
            symbol: symbol.to_string(),
            side,
            order_type: OrderType::Limit,
            time_in_force,
            price: price.to_string(),
            quantity: quantity.to_string(),
            new_client_order_id: client_order_id,
            new_order_resp_type: Some(NewOrderRespType::Full),
            api_key: self.api_key.clone(),
            timestamp,
            signature,
            recv_window,
        };

        self.send_request("order.place", request).await
    }

    /// 下市价单
    pub async fn place_market_order(
        &self,
        symbol: &str,
        side: OrderSide,
        quantity: Option<&str>,
        quote_order_qty: Option<&str>,
        client_order_id: Option<String>,
        recv_window: Option<u64>,
    ) -> Result<WebSocketResponse<OrderResponse>> {
        if quantity.is_none() && quote_order_qty.is_none() {
            return Err(anyhow!("Either quantity or quoteOrderQty must be specified for market order"));
        }

        let timestamp = Utc::now().timestamp_millis() as u64;
        
        // 构建签名参数
        let mut params = vec![
            ("symbol", symbol.to_string()),
            ("side", sonic_rs::to_string(&side)?),
            ("type", "MARKET".to_string()),
            ("apiKey", self.api_key.clone()),
            ("timestamp", timestamp.to_string()),
        ];

        if let Some(qty) = quantity {
            params.push(("quantity", qty.to_string()));
        }

        if let Some(quote_qty) = quote_order_qty {
            params.push(("quoteOrderQty", quote_qty.to_string()));
        }

        if let Some(client_id) = &client_order_id {
            params.push(("newClientOrderId", client_id.clone()));
        }

        if let Some(recv_win) = recv_window {
            params.push(("recvWindow", recv_win.to_string()));
        }

        // 生成查询字符串并签名
        let query_string = params
            .iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect::<Vec<_>>()
            .join("&");

        let signature = self.generate_signature(&query_string)?;

        let request = MarketOrderRequest {
            symbol: symbol.to_string(),
            side,
            order_type: OrderType::Market,
            quantity: quantity.map(|s| s.to_string()),
            quote_order_qty: quote_order_qty.map(|s| s.to_string()),
            new_client_order_id: client_order_id,
            new_order_resp_type: Some(NewOrderRespType::Full),
            api_key: self.api_key.clone(),
            timestamp,
            signature,
            recv_window,
        };

        self.send_request("order.place", request).await
    }

    /// 获取连接状态
    pub async fn get_connection_status(&self) -> ConnectionStatus {
        *self.connection_status.read().await
    }

    /// 手动重连
    pub async fn reconnect(&self) -> Result<()> {
        info!("Manually triggering reconnection");
        *self.connection_status.write().await = ConnectionStatus::Disconnected;
        self.start().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_signature_generation() {
        let client = BinanceWsTradeClient::new(
            "test_api_key".to_string(),
            "test_secret_key".to_string(),
            true,
        );

        let query_string = "symbol=BTCUSDT&side=BUY&type=LIMIT&timeInForce=GTC&quantity=1&price=23416.10000000&apiKey=test_api_key&timestamp=1660801715431";
        let signature = client.generate_signature(query_string).unwrap();
        
        // 验证签名不为空
        assert!(!signature.is_empty());
        assert_eq!(signature.len(), 64); // HMAC-SHA256 produces 64 character hex string
    }

    #[tokio::test]
    async fn test_client_creation() {
        let client = BinanceWsTradeClient::new(
            "test_api_key".to_string(),
            "test_secret_key".to_string(),
            true,
        );

        assert_eq!(client.get_connection_status().await, ConnectionStatus::Disconnected);
    }
}