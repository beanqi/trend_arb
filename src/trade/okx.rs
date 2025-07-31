use anyhow::{anyhow, Result};
use base64::{engine::general_purpose, Engine as _};
use futures_util::{SinkExt, StreamExt};
use hmac::{Hmac, Mac};
use log::{debug, error, info, warn};
use sha2::Sha256;
use sonic_rs::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::{mpsc, Mutex, RwLock};
use tokio::time::{sleep, timeout};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;

/// OKX WebSocket API URLs
const WS_PRIVATE_URL: &str = "wss://ws.okx.com:8443/ws/v5/private";
const WS_PRIVATE_TESTNET_URL: &str = "wss://wspap.okx.com:8443/ws/v5/private";

/// 订单类型
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OkxOrderType {
    Market,
    Limit,
    #[serde(rename = "post_only")]
    PostOnly,
    Fok,
    Ioc,
}

impl OkxOrderType {
    pub fn as_str(&self) -> &str {
        match self {
            OkxOrderType::Market => "market",
            OkxOrderType::Limit => "limit",
            OkxOrderType::PostOnly => "post_only",
            OkxOrderType::Fok => "fok",
            OkxOrderType::Ioc => "ioc",
        }
    }
}

/// 订单方向
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OkxOrderSide {
    Buy,
    Sell,
}

impl OkxOrderSide {
    pub fn as_str(&self) -> &str {
        match self {
            OkxOrderSide::Buy => "buy",
            OkxOrderSide::Sell => "sell",
        }
    }
}

/// 交易模式
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OkxTradeMode {
    Cash,      // 现货模式
    Cross,     // 全仓
    Isolated,  // 逐仓
}

impl OkxTradeMode {
    pub fn as_str(&self) -> &str {
        match self {
            OkxTradeMode::Cash => "cash",
            OkxTradeMode::Cross => "cross",
            OkxTradeMode::Isolated => "isolated",
        }
    }
}

/// 持仓方向
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OkxPositionSide {
    Net,   // 买卖模式
    Long,  // 开平仓模式开多
    Short, // 开平仓模式开空
}

impl OkxPositionSide {
    pub fn as_str(&self) -> &str {
        match self {
            OkxPositionSide::Net => "net",
            OkxPositionSide::Long => "long",
            OkxPositionSide::Short => "short",
        }
    }
}

/// 登录请求参数
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OkxLoginRequest {
    pub api_key: String,
    pub passphrase: String,
    pub timestamp: String,
    pub sign: String,
}

/// 下单请求参数
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OkxPlaceOrderRequest {
    pub inst_id: String,
    pub td_mode: String,
    pub side: String,
    #[serde(rename = "ordType")]
    pub ord_type: String,
    pub sz: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub px: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pos_side: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cl_ord_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reduce_only: Option<bool>,
}

/// 撤单请求参数
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OkxCancelOrderRequest {
    pub inst_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ord_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cl_ord_id: Option<String>,
}

/// 修改订单请求参数
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OkxAmendOrderRequest {
    pub inst_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ord_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cl_ord_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_sz: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_px: Option<String>,
}

/// WebSocket请求格式
#[derive(Debug, Clone, Serialize)]
pub struct OkxWebSocketRequest<T> {
    pub id: String,
    pub op: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub args: Option<T>,
}

/// 登录WebSocket请求格式
#[derive(Debug, Clone, Serialize)]
pub struct OkxLoginWebSocketRequest {
    pub op: String,
    pub args: Vec<OkxLoginRequest>,
}

/// 订单响应
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OkxOrderResponse {
    pub ord_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cl_ord_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    pub s_code: String,
    pub s_msg: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ts: Option<String>,
}

/// WebSocket响应格式
#[derive(Debug, Clone, Deserialize)]
pub struct OkxWebSocketResponse<T> {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub op: String,
    pub code: String,
    pub msg: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Vec<T>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conn_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub in_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub out_time: Option<String>,
}

/// 登录响应格式
#[derive(Debug, Clone, Deserialize)]
pub struct OkxLoginResponse {
    pub event: String,
    pub code: String,
    pub msg: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conn_id: Option<String>,
}

/// 连接状态
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ConnectionStatus {
    Disconnected,
    Connecting,
    Connected,
    LoggingIn,
    LoggedIn,
    Reconnecting,
}

/// OKX WebSocket交易客户端
#[derive(Clone)]
pub struct OkxWsTradeClient {
    api_key: String,
    secret_key: String,
    passphrase: String,
    testnet: bool,
    connection_status: Arc<RwLock<ConnectionStatus>>,
    pending_requests: Arc<Mutex<std::collections::HashMap<String, mpsc::UnboundedSender<String>>>>,
    reconnect_attempts: Arc<Mutex<u32>>,
    max_reconnect_attempts: u32,
    reconnect_delay: Duration,
    message_sender: Arc<Mutex<Option<mpsc::UnboundedSender<Message>>>>,
}

impl OkxWsTradeClient {
    /// 创建新的OKX WebSocket交易客户端
    pub fn new(api_key: String, secret_key: String, passphrase: String, testnet: bool) -> Self {
        Self {
            api_key,
            secret_key,
            passphrase,
            testnet,
            connection_status: Arc::new(RwLock::new(ConnectionStatus::Disconnected)),
            pending_requests: Arc::new(Mutex::new(std::collections::HashMap::new())),
            reconnect_attempts: Arc::new(Mutex::new(0)),
            max_reconnect_attempts: 6000,
            reconnect_delay: Duration::from_secs(1),
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
            WS_PRIVATE_TESTNET_URL
        } else {
            WS_PRIVATE_URL
        };

        info!("Connecting to OKX WebSocket API: {}", url);
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
                        
                        // 检查是否是登录响应
                        if let Ok(login_response) = sonic_rs::from_str::<OkxLoginResponse>(&text) {
                            if login_response.event == "login" {
                                if login_response.code == "0" {
                                    info!("Login successful");
                                    *connection_status.write().await = ConnectionStatus::LoggedIn;
                                } else {
                                    error!("Login failed: {}", login_response.msg);
                                    *connection_status.write().await = ConnectionStatus::Connected;
                                }
                                continue;
                            }
                        }
                        
                        // 解析普通响应并分发到对应的请求处理器
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

        // 登录
        if let Err(e) = self.login().await {
            error!("Failed to login: {}", e);
            return Err(e);
        }

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

    /// 登录到WebSocket
    async fn login(&self) -> Result<()> {
        info!("Logging in to OKX WebSocket");
        *self.connection_status.write().await = ConnectionStatus::LoggingIn;

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            .to_string();

        let sign_string = format!("{}GET/users/self/verify", timestamp);
        let signature = self.generate_signature(&sign_string)?;

        let login_request = OkxLoginWebSocketRequest {
            op: "login".to_string(),
            args: vec![OkxLoginRequest {
                api_key: self.api_key.clone(),
                passphrase: self.passphrase.clone(),
                timestamp,
                sign: signature,
            }],
        };

        let login_json = sonic_rs::to_string(&login_request)?;
        debug!("Sending login request: {}", login_json);

        // 发送登录请求
        {
            let sender_guard = self.message_sender.lock().await;
            if let Some(sender) = sender_guard.as_ref() {
                sender.send(Message::Text(login_json.into()))?;
            } else {
                return Err(anyhow!("WebSocket sender not available"));
            }
        }

        // 等待登录完成
        let mut attempts = 0;
        while attempts < 50 {  // 最多等待5秒
            tokio::time::sleep(Duration::from_millis(100)).await;
            let status = *self.connection_status.read().await;
            match status {
                ConnectionStatus::LoggedIn => return Ok(()),
                ConnectionStatus::Connected => return Err(anyhow!("Login failed")),
                _ => {}
            }
            attempts += 1;
        }

        Err(anyhow!("Login timeout"))
    }

    /// 发送WebSocket请求并等待响应（登录后的请求不需要签名）
    async fn send_request<T, R>(&self, op: &str, args: T) -> Result<OkxWebSocketResponse<R>>
    where
        T: Serialize,
        R: for<'de> Deserialize<'de>,
    {
        // 检查连接状态
        if *self.connection_status.read().await != ConnectionStatus::LoggedIn {
            return Err(anyhow!("WebSocket not logged in"));
        }

        let request_id = Uuid::new_v4().to_string();
        let request = OkxWebSocketRequest {
            id: request_id.clone(),
            op: op.to_string(),
            args: Some(vec![args]),
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

        let response: OkxWebSocketResponse<R> = sonic_rs::from_str(&response_text)?;
        
        if response.code != "0" {
            return Err(anyhow!("API Error {}: {}", response.code, response.msg));
        }

        Ok(response)
    }

    /// 生成签名
    fn generate_signature(&self, message: &str) -> Result<String> {
        let mut mac = HmacSha256::new_from_slice(self.secret_key.as_bytes())?;
        mac.update(message.as_bytes());
        let result = mac.finalize();
        Ok(general_purpose::STANDARD.encode(result.into_bytes()))
    }

    /// 下限价单
    pub async fn place_limit_order(
        &self,
        inst_id: &str,
        side: OkxOrderSide,
        td_mode: OkxTradeMode,
        price: &str,
        size: &str,
        pos_side: Option<OkxPositionSide>,
        client_order_id: Option<String>,
        tag: Option<String>,
        reduce_only: Option<bool>,
    ) -> Result<OkxWebSocketResponse<OkxOrderResponse>> {
        let request = OkxPlaceOrderRequest {
            inst_id: inst_id.to_string(),
            td_mode: td_mode.as_str().to_string(),
            side: side.as_str().to_string(),
            ord_type: OkxOrderType::Limit.as_str().to_string(),
            sz: size.to_string(),
            px: Some(price.to_string()),
            pos_side: pos_side.map(|ps| ps.as_str().to_string()),
            cl_ord_id: client_order_id,
            tag,
            reduce_only,
        };

        self.send_request("order", request).await
    }

    /// 下市价单
    pub async fn place_market_order(
        &self,
        inst_id: &str,
        side: OkxOrderSide,
        td_mode: OkxTradeMode,
        size: &str,
        pos_side: Option<OkxPositionSide>,
        client_order_id: Option<String>,
        tag: Option<String>,
        reduce_only: Option<bool>,
    ) -> Result<OkxWebSocketResponse<OkxOrderResponse>> {
        let request = OkxPlaceOrderRequest {
            inst_id: inst_id.to_string(),
            td_mode: td_mode.as_str().to_string(),
            side: side.as_str().to_string(),
            ord_type: OkxOrderType::Market.as_str().to_string(),
            sz: size.to_string(),
            px: None,
            pos_side: pos_side.map(|ps| ps.as_str().to_string()),
            cl_ord_id: client_order_id,
            tag,
            reduce_only,
        };

        self.send_request("order", request).await
    }

    /// 下POST_ONLY单
    pub async fn place_post_only_order(
        &self,
        inst_id: &str,
        side: OkxOrderSide,
        td_mode: OkxTradeMode,
        price: &str,
        size: &str,
        pos_side: Option<OkxPositionSide>,
        client_order_id: Option<String>,
        tag: Option<String>,
        reduce_only: Option<bool>,
    ) -> Result<OkxWebSocketResponse<OkxOrderResponse>> {
        let request = OkxPlaceOrderRequest {
            inst_id: inst_id.to_string(),
            td_mode: td_mode.as_str().to_string(),
            side: side.as_str().to_string(),
            ord_type: OkxOrderType::PostOnly.as_str().to_string(),
            sz: size.to_string(),
            px: Some(price.to_string()),
            pos_side: pos_side.map(|ps| ps.as_str().to_string()),
            cl_ord_id: client_order_id,
            tag,
            reduce_only,
        };

        self.send_request("order", request).await
    }

    /// 批量下单
    pub async fn place_multiple_orders(
        &self,
        orders: Vec<OkxPlaceOrderRequest>,
    ) -> Result<OkxWebSocketResponse<OkxOrderResponse>> {
        self.send_request("batch-orders", orders).await
    }

    /// 撤单
    pub async fn cancel_order(
        &self,
        inst_id: &str,
        order_id: Option<String>,
        client_order_id: Option<String>,
    ) -> Result<OkxWebSocketResponse<OkxOrderResponse>> {
        if order_id.is_none() && client_order_id.is_none() {
            return Err(anyhow!("Either order_id or client_order_id must be provided"));
        }

        let request = OkxCancelOrderRequest {
            inst_id: inst_id.to_string(),
            ord_id: order_id,
            cl_ord_id: client_order_id,
        };

        self.send_request("cancel-order", request).await
    }

    /// 批量撤单
    pub async fn cancel_multiple_orders(
        &self,
        orders: Vec<OkxCancelOrderRequest>,
    ) -> Result<OkxWebSocketResponse<OkxOrderResponse>> {
        self.send_request("cancel-batch-orders", orders).await
    }

    /// 修改订单
    pub async fn amend_order(
        &self,
        inst_id: &str,
        order_id: Option<String>,
        client_order_id: Option<String>,
        new_size: Option<String>,
        new_price: Option<String>,
    ) -> Result<OkxWebSocketResponse<OkxOrderResponse>> {
        if order_id.is_none() && client_order_id.is_none() {
            return Err(anyhow!("Either order_id or client_order_id must be provided"));
        }

        if new_size.is_none() && new_price.is_none() {
            return Err(anyhow!("Either new_size or new_price must be provided"));
        }

        let request = OkxAmendOrderRequest {
            inst_id: inst_id.to_string(),
            ord_id: order_id,
            cl_ord_id: client_order_id,
            new_sz: new_size,
            new_px: new_price,
        };

        self.send_request("amend-order", request).await
    }

    /// 批量修改订单
    pub async fn amend_multiple_orders(
        &self,
        orders: Vec<OkxAmendOrderRequest>,
    ) -> Result<OkxWebSocketResponse<OkxOrderResponse>> {
        self.send_request("amend-batch-orders", orders).await
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
        let client = OkxWsTradeClient::new(
            "test_api_key".to_string(),
            "test_secret_key".to_string(),
            "test_passphrase".to_string(),
            true,
        );

        let message = "1640995200GET/users/self/verify";
        let signature = client.generate_signature(message).unwrap();
        println!("Generated signature: {}", signature);
        // 验证签名不为空且是有效的base64
        assert!(!signature.is_empty());
        assert!(general_purpose::STANDARD.decode(&signature).is_ok());
    }

    #[tokio::test]
    async fn test_client_creation() {
        let client = OkxWsTradeClient::new(
            "test_api_key".to_string(),
            "test_secret_key".to_string(),
            "test_passphrase".to_string(),
            true,
        );

        assert_eq!(client.get_connection_status().await, ConnectionStatus::Disconnected);
    }

    #[test]
    fn test_order_side_serialization() {
        assert_eq!(OkxOrderSide::Buy.as_str(), "buy");
        assert_eq!(OkxOrderSide::Sell.as_str(), "sell");
    }

    #[test]
    fn test_order_type_serialization() {
        assert_eq!(OkxOrderType::Market.as_str(), "market");
        assert_eq!(OkxOrderType::Limit.as_str(), "limit");
        assert_eq!(OkxOrderType::PostOnly.as_str(), "post_only");
    }

    #[test]
    fn test_trade_mode_serialization() {
        assert_eq!(OkxTradeMode::Cash.as_str(), "cash");
        assert_eq!(OkxTradeMode::Cross.as_str(), "cross");
        assert_eq!(OkxTradeMode::Isolated.as_str(), "isolated");
    }
}
