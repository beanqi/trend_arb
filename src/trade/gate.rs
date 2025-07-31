use anyhow::{anyhow, Result};
use futures_util::{SinkExt, StreamExt};
use hmac::{Hmac, Mac};
use log::{debug, error, info, warn};
use sha2::Sha512;
use sonic_rs::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, Mutex, RwLock};
use tokio::time::{sleep, timeout};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use uuid::Uuid;

type HmacSha512 = Hmac<Sha512>;

/// Gate.io WebSocket API base URL
const WS_API_URL: &str = "wss://api.gateio.ws/ws/v4/";
const WS_API_TESTNET_URL: &str = "wss://ws-testnet.gate.com/v4/ws/spot";

/// 订单类型
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GateOrderType {
    Limit,
    Market,
}

/// 订单方向
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GateOrderSide {
    Buy,
    Sell,
}

impl GateOrderSide {
    pub fn as_str(&self) -> &str {
        match self {
            GateOrderSide::Buy => "buy",
            GateOrderSide::Sell => "sell",
        }
    }
}

/// Time in Force
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GateTimeInForce {
    Gtc, // Good Till Cancel
    Ioc, // Immediate or Cancel
    Fok, // Fill or Kill
    Poc, // Pending or Cancel (被动委托)
}

impl GateTimeInForce {
    pub fn as_str(&self) -> &str {
        match self {
            GateTimeInForce::Gtc => "gtc",
            GateTimeInForce::Ioc => "ioc",
            GateTimeInForce::Fok => "fok",
            GateTimeInForce::Poc => "poc",
        }
    }
}

/// 处理模式
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum GateActionMode {
    Ack,    // 异步模式，只返回订单关键字段
    Result, // 无清算信息
    Full,   // 完整模式（默认）
}

/// 登录请求参数
#[derive(Debug, Clone, Serialize)]
pub struct GateLoginRequest {
    pub req_id: String,
    pub api_key: String,
    pub signature: String,
    pub timestamp: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub req_header: Option<serde_json::Value>,
}

/// 限价单请求参数
#[derive(Debug, Clone, Serialize)]
pub struct GateLimitOrderRequest {
    pub text: String,                    // 自定义订单ID
    pub currency_pair: String,
    #[serde(rename = "type")]
    pub order_type: GateOrderType,
    pub account: String,                 // "spot", "margin", "cross_margin"
    pub side: GateOrderSide,
    pub amount: String,
    pub price: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_in_force: Option<GateTimeInForce>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iceberg: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_borrow: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_repay: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stp_act: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action_mode: Option<GateActionMode>,
}

/// 市价单请求参数
#[derive(Debug, Clone, Serialize)]
pub struct GateMarketOrderRequest {
    pub text: String,
    pub currency_pair: String,
    #[serde(rename = "type")]
    pub order_type: GateOrderType,
    pub account: String,
    pub side: GateOrderSide,
    pub amount: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_in_force: Option<GateTimeInForce>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_borrow: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_repay: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stp_act: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action_mode: Option<GateActionMode>,
}

/// WebSocket API请求格式
#[derive(Debug, Clone, Serialize)]
pub struct GateApiRequest<T> {
    pub time: u64,
    pub channel: String,
    pub event: String, // 固定为 "api"
    pub payload: T,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

/// API请求负载
#[derive(Debug, Clone, Serialize)]
pub struct GateApiPayload<T> {
    pub req_id: String,
    pub req_param: T,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub req_header: Option<serde_json::Value>,
}

/// 订单响应基础信息
#[derive(Debug, Clone, Deserialize)]
pub struct GateOrderResponse {
    pub id: String,
    pub text: String,
    pub amend_text: String,
    pub create_time: String,
    pub update_time: String,
    pub create_time_ms: u64,
    pub update_time_ms: u64,
    pub status: String,
    pub currency_pair: String,
    #[serde(rename = "type")]
    pub order_type: String,
    pub account: String,
    pub side: String,
    pub amount: String,
    pub price: String,
    pub time_in_force: String,
    pub iceberg: String,
    pub left: String,
    pub filled_total: String,
    pub fee: String,
    pub fee_currency: String,
    pub point_fee: String,
    pub gt_fee: String,
    pub gt_maker_fee: String,
    pub gt_taker_fee: String,
    pub gt_discount: bool,
    pub rebated_fee: String,
    pub rebated_fee_currency: String,
    pub stp_act: String,
    pub finish_as: String,
}

/// 登录响应
#[derive(Debug, Clone, Deserialize)]
pub struct GateLoginResponse {
    pub api_key: String,
    pub uid: String,
}

/// WebSocket响应头部
#[derive(Debug, Clone, Deserialize)]
pub struct GateResponseHeader {
    pub response_time: String,
    pub status: String,
    pub channel: String,
    pub event: String,
    pub client_id: String,
    pub x_in_time: u64,
    pub x_out_time: u64,
    pub conn_id: String,
    pub conn_trace_id: String,
    pub trace_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub x_gate_ratelimit_requests_remain: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub x_gate_ratelimit_limit: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub x_gat_ratelimit_reset_timestamp: Option<u64>,
}

/// WebSocket响应数据
#[derive(Debug, Clone, Deserialize)]
pub struct GateResponseData<T> {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub errs: Option<GateApiError>,
}

/// WebSocket响应格式
#[derive(Debug, Clone, Deserialize)]
pub struct GateWebSocketResponse<T> {
    pub request_id: String,
    pub header: GateResponseHeader,
    pub data: GateResponseData<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ack: Option<bool>,
}

/// API错误信息
#[derive(Debug, Clone, Deserialize)]
pub struct GateApiError {
    pub label: String,
    pub message: String,
}

/// 连接状态
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ConnectionStatus {
    Disconnected,
    Connecting,
    Connected,
    LoggedIn,
    Reconnecting,
}

/// Gate.io WebSocket交易客户端
#[derive(Clone)]
pub struct GateWsTradeClient {
    api_key: String,
    secret_key: String,
    testnet: bool,
    connection_status: Arc<RwLock<ConnectionStatus>>,
    pending_requests: Arc<Mutex<std::collections::HashMap<String, mpsc::UnboundedSender<String>>>>,
    reconnect_attempts: Arc<Mutex<u32>>,
    max_reconnect_attempts: u32,
    reconnect_delay: Duration,
    message_sender: Arc<Mutex<Option<mpsc::UnboundedSender<Message>>>>,
    login_status: Arc<RwLock<bool>>,
}

impl GateWsTradeClient {
    /// 创建新的Gate.io WebSocket交易客户端
    pub fn new(api_key: String, secret_key: String, testnet: bool) -> Self {
        Self {
            api_key,
            secret_key,
            testnet,
            connection_status: Arc::new(RwLock::new(ConnectionStatus::Disconnected)),
            pending_requests: Arc::new(Mutex::new(std::collections::HashMap::new())),
            reconnect_attempts: Arc::new(Mutex::new(0)),
            max_reconnect_attempts: 6000,
            reconnect_delay: Duration::from_secs(1),
            message_sender: Arc::new(Mutex::new(None)),
            login_status: Arc::new(RwLock::new(false)),
        }
    }

    /// 启动WebSocket连接
    pub async fn start(&self) -> Result<()> {
        loop {
            match self.connect_and_login().await {
                Ok(_) => {
                    info!("WebSocket connection established and logged in");
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
                    *self.login_status.write().await = false;
                    sleep(self.reconnect_delay).await;
                }
            }
        }
    }

    /// 建立WebSocket连接并登录
    async fn connect_and_login(&self) -> Result<()> {
        let url = if self.testnet {
            WS_API_TESTNET_URL
        } else {
            WS_API_URL
        };

        info!("Connecting to Gate.io WebSocket API: {}", url);
        *self.connection_status.write().await = ConnectionStatus::Connecting;

        let (ws_stream, _) = connect_async(url).await?;
        
        *self.connection_status.write().await = ConnectionStatus::Connected;
        info!("WebSocket connection established");

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
        let login_status = self.login_status.clone();
        
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
                            if let Some(request_id) = response.get("request_id").and_then(|v| v.as_str()) {
                                let mut pending = pending_requests.lock().await;
                                if let Some(sender) = pending.remove(request_id) {
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
            *login_status.write().await = false;
        });

        // 执行登录
        self.login().await?;

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

    /// 执行登录
    async fn login(&self) -> Result<()> {
        let timestamp = chrono::Local::now().timestamp() as u64;
        let req_id = format!("{}-login", chrono::Local::now().timestamp_millis());
        
        // 生成登录签名
        let signature = self.generate_login_signature(timestamp)?;
        
        let login_request = GateLoginRequest {
            req_id: req_id.clone(),
            api_key: self.api_key.clone(),
            signature,
            timestamp: timestamp.to_string(),
            req_header: None,
        };

        let request = GateApiRequest {
            time: timestamp,
            channel: "spot.login".to_string(),
            event: "api".to_string(),
            payload: login_request,
            id: None,
        };

        let response: GateWebSocketResponse<GateLoginResponse> = self.send_api_request(request).await?;
        
        if response.header.status != "200" {
            if let Some(error) = response.data.errs {
                return Err(anyhow!("Login failed: {}: {}", error.label, error.message));
            } else {
                return Err(anyhow!("Login failed with status: {}", response.header.status));
            }
        }

        *self.login_status.write().await = true;
        *self.connection_status.write().await = ConnectionStatus::LoggedIn;
        info!("Successfully logged in to Gate.io WebSocket API");

        Ok(())
    }

    /// 发送API请求并等待响应
    async fn send_api_request<T, R>(&self, request: GateApiRequest<T>) -> Result<GateWebSocketResponse<R>>
    where
        T: Serialize,
        R: for<'de> Deserialize<'de>,
    {
        // 检查连接状态
        if *self.connection_status.read().await == ConnectionStatus::Disconnected {
            return Err(anyhow!("WebSocket not connected"));
        }

        // 为请求生成唯一ID
        let request_id = if let Some(ref payload) = request.id {
            payload.clone()
        } else {
            Uuid::new_v4().to_string()
        };

        let request_json = sonic_rs::to_string(&request)?;
        debug!("Sending WebSocket API request: {}", request_json);

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

        let response: GateWebSocketResponse<R> = sonic_rs::from_str(&response_text)?;
        
        Ok(response)
    }

    /// 生成登录签名
    fn generate_login_signature(&self, timestamp: u64) -> Result<String> {
        // 登录时req_param为空字符串
        let signature_string = format!("api\nspot.login\n\n{}", timestamp);
        
        let mut mac = HmacSha512::new_from_slice(self.secret_key.as_bytes())?;
        mac.update(signature_string.as_bytes());
        let result = mac.finalize();
        Ok(hex::encode(result.into_bytes()))
    }

    /// 下限价单
    pub async fn place_limit_order(
        &self,
        symbol: &str,
        side: GateOrderSide,
        price: &str,
        quantity: &str,
        time_in_force: Option<GateTimeInForce>,
        client_order_id: Option<String>,
        account: Option<&str>,
    ) -> Result<GateWebSocketResponse<GateOrderResponse>> {
        // 检查登录状态
        if !*self.login_status.read().await {
            return Err(anyhow!("Not logged in"));
        }

        let timestamp = chrono::Local::now().timestamp() as u64;
        let req_id = format!("{}-limit-order", chrono::Local::now().timestamp_millis());
        
        let order_request = GateLimitOrderRequest {
            text: client_order_id.unwrap_or_else(|| format!("limit-{}", timestamp)),
            currency_pair: symbol.to_string(),
            order_type: GateOrderType::Limit,
            account: account.unwrap_or("spot").to_string(),
            side,
            amount: quantity.to_string(),
            price: price.to_string(),
            time_in_force: Some(time_in_force.unwrap_or(GateTimeInForce::Gtc)),
            iceberg: None,
            auto_borrow: None,
            auto_repay: None,
            stp_act: None,
            action_mode: Some(GateActionMode::Full),
        };

        let payload = GateApiPayload {
            req_id: req_id.clone(),
            req_param: order_request,
            req_header: None,
        };

        let request = GateApiRequest {
            time: timestamp,
            channel: "spot.order_place".to_string(),
            event: "api".to_string(),
            payload,
            id: Some(req_id),
        };

        self.send_api_request(request).await
    }

    /// 下市价单
    pub async fn place_market_order(
        &self,
        symbol: &str,
        side: GateOrderSide,
        quantity: &str,
        client_order_id: Option<String>,
        account: Option<&str>,
    ) -> Result<GateWebSocketResponse<GateOrderResponse>> {
        // 检查登录状态
        if !*self.login_status.read().await {
            return Err(anyhow!("Not logged in"));
        }

        let timestamp = chrono::Local::now().timestamp() as u64;
        let req_id = format!("{}-market-order", chrono::Local::now().timestamp_millis());
        
        let order_request = GateMarketOrderRequest {
            text: client_order_id.unwrap_or_else(|| format!("market-{}", timestamp)),
            currency_pair: symbol.to_string(),
            order_type: GateOrderType::Market,
            account: account.unwrap_or("spot").to_string(),
            side,
            amount: quantity.to_string(),
            time_in_force: Some(GateTimeInForce::Ioc), // 市价单通常使用IOC
            auto_borrow: None,
            auto_repay: None,
            stp_act: None,
            action_mode: Some(GateActionMode::Full),
        };

        let payload = GateApiPayload {
            req_id: req_id.clone(),
            req_param: order_request,
            req_header: None,
        };

        let request = GateApiRequest {
            time: timestamp,
            channel: "spot.order_place".to_string(),
            event: "api".to_string(),
            payload,
            id: Some(req_id),
        };

        self.send_api_request(request).await
    }

    /// 取消订单
    pub async fn cancel_order(
        &self,
        symbol: &str,
        order_id: &str,
        account: Option<&str>,
    ) -> Result<GateWebSocketResponse<GateOrderResponse>> {
        // 检查登录状态
        if !*self.login_status.read().await {
            return Err(anyhow!("Not logged in"));
        }

        let timestamp = chrono::Local::now().timestamp() as u64;
        let req_id = format!("{}-cancel-order", chrono::Local::now().timestamp_millis());
        
        let cancel_request = serde_json::json!({
            "order_id": order_id,
            "currency_pair": symbol,
            "account": account.unwrap_or("spot")
        });

        let payload = GateApiPayload {
            req_id: req_id.clone(),
            req_param: cancel_request,
            req_header: None,
        };

        let request = GateApiRequest {
            time: timestamp,
            channel: "spot.order_cancel".to_string(),
            event: "api".to_string(),
            payload,
            id: Some(req_id),
        };

        self.send_api_request(request).await
    }

    /// 获取连接状态
    pub async fn get_connection_status(&self) -> ConnectionStatus {
        *self.connection_status.read().await
    }

    /// 检查是否已登录
    pub async fn is_logged_in(&self) -> bool {
        *self.login_status.read().await
    }

    /// 手动重连
    pub async fn reconnect(&self) -> Result<()> {
        info!("Manually triggering reconnection");
        *self.connection_status.write().await = ConnectionStatus::Disconnected;
        *self.login_status.write().await = false;
        self.start().await
    }

    /// 查询订单状态
    pub async fn get_order_status(
        &self,
        symbol: &str,
        order_id: &str,
        account: Option<&str>,
    ) -> Result<GateWebSocketResponse<GateOrderResponse>> {
        // 检查登录状态
        if !*self.login_status.read().await {
            return Err(anyhow!("Not logged in"));
        }

        let timestamp = chrono::Local::now().timestamp() as u64;
        let req_id = format!("{}-order-status", chrono::Local::now().timestamp_millis());
        
        let status_request = serde_json::json!({
            "order_id": order_id,
            "currency_pair": symbol,
            "account": account.unwrap_or("spot")
        });

        let payload = GateApiPayload {
            req_id: req_id.clone(),
            req_param: status_request,
            req_header: None,
        };

        let request = GateApiRequest {
            time: timestamp,
            channel: "spot.order_status".to_string(),
            event: "api".to_string(),
            payload,
            id: Some(req_id),
        };

        self.send_api_request(request).await
    }

    /// 发送心跳ping
    pub async fn ping(&self) -> Result<()> {
        let timestamp = chrono::Local::now().timestamp() as u64;
        
        let request = GateApiRequest {
            time: timestamp,
            channel: "spot.ping".to_string(),
            event: "subscribe".to_string(),
            payload: serde_json::Value::Null,
            id: None,
        };

        let request_json = sonic_rs::to_string(&request)?;
        
        let sender_guard = self.message_sender.lock().await;
        if let Some(sender) = sender_guard.as_ref() {
            sender.send(Message::Text(request_json.into()))?;
            debug!("Sent ping request");
        } else {
            return Err(anyhow!("WebSocket sender not available"));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_signature_generation() {
        let client = GateWsTradeClient::new(
            "test_api_key".to_string(),
            "test_secret_key".to_string(),
            true,
        );

        let timestamp = 1681984544u64;
        let signature = client.generate_login_signature(timestamp).unwrap();
        println!("Generated signature: {}", signature);
        // 验证签名不为空
        assert!(!signature.is_empty());
        assert_eq!(signature.len(), 128); // HMAC-SHA512 produces 128 character hex string
    }

    #[tokio::test]
    async fn test_client_creation() {
        let client = GateWsTradeClient::new(
            "test_api_key".to_string(),
            "test_secret_key".to_string(),
            true,
        );

        assert_eq!(client.get_connection_status().await, ConnectionStatus::Disconnected);
        assert!(!client.is_logged_in().await);
    }

    #[test]
    fn test_order_side_conversion() {
        assert_eq!(GateOrderSide::Buy.as_str(), "buy");
        assert_eq!(GateOrderSide::Sell.as_str(), "sell");
    }

    #[test]
    fn test_time_in_force_conversion() {
        assert_eq!(GateTimeInForce::Gtc.as_str(), "gtc");
        assert_eq!(GateTimeInForce::Ioc.as_str(), "ioc");
        assert_eq!(GateTimeInForce::Fok.as_str(), "fok");
        assert_eq!(GateTimeInForce::Poc.as_str(), "poc");
    }
}
