use bytes::Bytes;
use futures::{
    stream::{SplitSink, SplitStream},
    SinkExt, StreamExt,
};
use std::{
    cell::RefCell,
    collections::HashMap,
    error::Error,
    iter,
    sync::atomic::{AtomicBool, AtomicU16, AtomicU64, AtomicU8, Ordering},
};
use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use tungstenite::Message;

use crate::web_socket::{utils::SingleThreadSharedData, OpCode};

use super::utils::wait_util_ms;

pub const MAX_CONN_COUNT: usize = 30; // 最大连接数量

async fn connect_tcp_tls(host: &str, port: u16) -> Result<TlsStream<TcpStream>, Box<dyn Error + 'static>> {
    // 2. 连接到 TCP
    let tcp_stream = TcpStream::connect((host, port)).await?;
    tcp_stream.set_nodelay(true)?;

    // 3. 升级到 TLS
    let mut root_store = rustls::RootCertStore::empty();
    // 添加系统根证书
    root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let config = rustls::ClientConfig::builder().with_root_certificates(root_store).with_no_client_auth();
    let connector = tokio_rustls::TlsConnector::from(std::sync::Arc::new(config));
    let server_name = rustls::pki_types::ServerName::try_from(host.to_string())?;
    // 将 TCP stream 升级为 TLS stream
    connector.connect(server_name, tcp_stream).await.map_err(|e| Box::new(e) as Box<dyn Error + 'static>)
}

async fn connect_tcp(host: &str, port: u16) -> Result<TcpStream, Box<dyn Error>> {
    let tcp_stream = TcpStream::connect((host, port)).await?;
    tcp_stream.set_nodelay(true)?;
    Ok(tcp_stream)
}

async fn connect_ws(url: &str, headers: Vec<(String, String)>, is_deflate: bool) -> Result<WebSocketStream<MaybeTlsStream<TcpStream>>, Box<dyn Error>> {
    let url_parsed = url::Url::parse(url).unwrap();
    let host = url_parsed.host_str().ok_or("URL does not have a host")?;
    let port = url_parsed.port_or_known_default().unwrap_or(443);

    // 连接 TCP 并升级到 TLS
    let maybe_tls_stream = if url.starts_with("wss") {
        let tls_stream = connect_tcp_tls(host, port).await?;
        MaybeTlsStream::Rustls(tls_stream)
    } else {
        MaybeTlsStream::Plain(connect_tcp(host, port).await?)
    };

    let mut request = tungstenite::ClientRequestBuilder::new(url.parse().unwrap());
    for (header_name, header_value) in headers {
        request = request.with_header(header_name, header_value);
    }

    let mut config = tungstenite::protocol::WebSocketConfig::default();
    if is_deflate {
        config.compression = Some(tungstenite::extensions::DeflateConfig::default());
    }
    let (ws, _) = tokio_tungstenite::client_async_with_config(request, maybe_tls_stream, Some(config)).await?;
    Ok(ws)
}

pub enum WebSocketWriter {
    None,
    Writer(SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>),
}

impl WebSocketWriter {
    /// 向WebSocket连接写入数据
    pub async fn write_frame(&mut self, data: String, op_code: OpCode) -> Result<(), Box<dyn Error>> {
        match self {
            WebSocketWriter::None => Err("WebSocketWriter is None".into()),
            WebSocketWriter::Writer(writer) => {
                match op_code {
                    OpCode::Text => {
                        writer.send(Message::text(data)).await?;
                    }
                    OpCode::Binary => {
                        writer.send(Message::binary(data)).await?;
                    }
                    OpCode::Close => {
                        writer.send(Message::Close(None)).await?;
                    }
                    OpCode::Ping => {
                        writer.send(Message::Ping(Bytes::from(data))).await?;
                    }
                    OpCode::Pong => {
                        writer.send(Message::Pong(Bytes::from(data))).await?;
                    }
                }
                Ok(())
            }
        }
    }
}

pub type TungWebSocketReader = SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>;

/// WebSocket连接管理器，支持多连接和自动分配
///
/// 管理多个WebSocket连接，支持订阅/取消订阅交易对，并自动分配连接
///
/// 对链接的所有管理操作都是串行操作，不应该存在并发
pub struct TungWsConnection {
    market_code: String,
    /// WebSocket服务器URL
    url: RefCell<String>,
    /// 是否启用压缩
    is_deflate: bool,
    /// 每个连接允许的最大交易对数量
    max_pairs_per_conn: u8,
    /// 最小链接数量
    min_start_conn_count: AtomicU8,
    /// 交易对与连接索引的映射
    pair_conn_map: SingleThreadSharedData<HashMap<String, u8>>,
    /// 读取任务处理函数
    do_read: std::sync::Arc<dyn Fn(TungWebSocketReader, u8) -> tokio::task::JoinHandle<()>>,
    /// 用于构建订阅参数的函数
    build_sub_param: Box<dyn Fn(&str) -> String>,
    /// 用于清空深度数据
    clear_depth: Box<dyn Fn(&str)>,
    /// 每条连接所订阅的交易对数量（当交易对为0时，需要关闭当前连接，并释放JoinHandle）
    conn_status: Vec<AtomicBool>, // true表示未连接，false表示连接已建立
    /// 用于管理链接句柄
    writers: SingleThreadSharedData<Vec<WebSocketWriter>>,
    /// 连接的状态
    link_state: AtomicBool,
    /// 用于管理读取任务句柄
    handles: SingleThreadSharedData<Vec<tokio::task::JoinHandle<()>>>,
    /// 连接的请求头
    headers: SingleThreadSharedData<Vec<(String, String)>>,
    /// 建立websocket最小等待时间间隔（单位：毫秒），某些交易所限制连接频率
    connect_wait_time: AtomicU16,
    _last_connect_time: AtomicU64,
    /// websocket写入的最小等待时间间隔（单位：毫秒），某些交易所限制写入频率
    write_wait_time: AtomicU16,
    last_write_time: Vec<AtomicU64>,
}

unsafe impl Send for TungWsConnection {}
unsafe impl Sync for TungWsConnection {}

impl Drop for TungWsConnection {
    fn drop(&mut self) {
        // 关闭所有连接
        let _ = self.close();
    }
}

impl TungWsConnection {
    /// 创建新的WebSocket连接管理器
    ///
    /// # 参数
    /// * `url` - WebSocket服务器URL
    /// * `is_deflate` - 是否启用压缩
    /// * `conn_count` - 连接数量
    /// * `do_read` - 一个处理读取任务的函数，接收读取组件和连接索引，返回任务句柄, example:
    /// ```rust
    /// // writer_channel_sender: 如要向websocket发送消息，需要使用这个sender
    /// // reconnect_channel_sender: 用于重新连接的sender
    /// async fn do_read(reader: NbWebSocketPartsReader, idx: u8) -> tokio::task::JoinHandle<()> {
    ///    tokio::spawn(async move {
    ///       loop {
    ///          let frame = reader.read_frame().await.unwrap();
    ///         // do something with the frame
    ///      }
    ///  })
    /// }
    /// ```
    /// # 返回
    /// * `Self` - 新创建的连接管理器实例
    pub async fn new(
        market_code: &str,
        url: &str,
        is_deflate: bool,
        max_pairs_per_conn: u8,
        do_read: std::sync::Arc<dyn Fn(TungWebSocketReader, u8) -> tokio::task::JoinHandle<()> + Send + Sync>,
        build_sub_param: Box<dyn Fn(&str) -> String + Send + Sync>,
        clear_depth: Box<dyn Fn(&str) + Send + Sync>,
    ) -> std::sync::Arc<Self> {
        let res: TungWsConnection = Self {
            market_code: market_code.to_string(),
            url: RefCell::new(url.to_string()),
            is_deflate,
            max_pairs_per_conn,
            min_start_conn_count: AtomicU8::new(0),
            pair_conn_map: SingleThreadSharedData::new(HashMap::new()),
            do_read,
            build_sub_param,
            clear_depth,
            conn_status: iter::repeat_with(|| AtomicBool::new(false)).take(MAX_CONN_COUNT).collect(),
            link_state: AtomicBool::new(false),
            writers: SingleThreadSharedData::new(Vec::with_capacity(MAX_CONN_COUNT)),
            handles: SingleThreadSharedData::new(Vec::with_capacity(MAX_CONN_COUNT)),
            headers: SingleThreadSharedData::new(vec![]),
            connect_wait_time: AtomicU16::new(0),
            _last_connect_time: AtomicU64::new(0),
            write_wait_time: AtomicU16::new(0),
            last_write_time: iter::repeat_with(|| AtomicU64::new(0)).take(MAX_CONN_COUNT).collect(),
        };
        let result = std::sync::Arc::new(res);
        return result;
    }
    /// 设置建立连接的最小等待时间
    /// # 参数
    /// * `time` - 等待时间（单位：毫秒）
    pub fn set_connect_wait_time(&self, time: u16) {
        self.connect_wait_time.store(time, Ordering::SeqCst);
    }
    /// 设置订阅交易对的最小等待时间
    /// # 参数
    /// * `time` - 等待时间（单位：毫秒）
    pub fn set_write_wait_time(&self, time: u16) {
        self.write_wait_time.store(time, Ordering::SeqCst);
    }

    pub fn set_min_conn_count(&self, count: u8) {
        self.min_start_conn_count.store(count, Ordering::SeqCst);
    }

    pub fn set_url(&self, url: &str) {
        self.url.replace(url.to_string());
    }
    /// 获取当前连接所有订阅的交易对
    /// # 参数
    /// * `idx` - 连接索引
    /// # 返回
    /// * `Vec<String>` - 订阅的交易对列表
    pub fn get_sub_pairs(&self, idx: u8) -> Vec<String> {
        let mut pairs = vec![];
        let pair_conn_map = self.pair_conn_map.get();
        for (k, v) in pair_conn_map.iter() {
            if *v == idx {
                pairs.push(k.clone());
            }
        }
        pairs
    }

    /// 设置状态
    pub fn set_link_sate(&self, state: bool) {
        self.link_state.store(state, Ordering::SeqCst)
    }

    /// 检查状态
    pub fn get_state(&self) -> bool {
        self.link_state.load(Ordering::SeqCst)
    }

    pub fn set_headers(&self, headers: Vec<(String, String)>) {
        let headers_guard = self.headers.get_mut();
        headers_guard.clear();
        for (k, v) in headers {
            headers_guard.push((k, v));
        }
    }

    /// 初始化所有WebSocket连接
    /// # 返回
    /// * `Vec<tokio::task::JoinHandle<()>>` - 所有任务的句柄集合，包括读取任务和写入任务
    pub async fn init(&self) {
        // i. 初始化Writer数组， handles数组
        for _ in 0..MAX_CONN_COUNT {
            self.writers.get_mut().push(WebSocketWriter::None);
            self.handles.get_mut().push(tokio::spawn(async move {
                // Empty task that will be replaced when actual connection is established
            }));
        }
        // ii. 设置连接状态
        Self::set_link_sate(&self, true);
        // iii. 初始化最小链接数量
        for i in 0..self.min_start_conn_count.load(Ordering::Acquire) {
            match self.init_connection(i).await {
                Ok(_) => {
                    // 连接成功
                    log::warn!("{}-WS连接成功-idx={}", self.market_code, i);
                }
                Err(e) => {
                    // 连接失败
                    log::warn!("{}-WS连接失败-idx={},Error:{}", self.market_code, i, e);
                    return;
                }
            }
        }
    }

    /// 初始化指定索引的WebSocket连接
    async fn init_connection(&self, index: u8) -> Result<(), Box<dyn Error>> {
        if !self.get_state() {
            return Err("NBWsConnection 未初始化".into());
        }
        // i. 检查连接是否已经创建
        if self.conn_status[index as usize].load(Ordering::Acquire) {
            // 连接已经创建，直接返回
            return Ok(());
        }
        log::warn!("{}-WS连接开始建立-idx={}", self.market_code, index);
        // ii. 创建新连接
        let header_vec = self.build_header();
        let url = self.url.borrow().clone();
        let ws = connect_ws(&url, header_vec, self.is_deflate).await?;
        let (writer, reader) = ws.split();

        // iii. 更新writers
        self.writers.get_mut()[index as usize] = WebSocketWriter::Writer(writer);
        // iv. 创建异步读取任务，并更新handles
        let do_read = self.do_read.clone();
        let handle = do_read(reader, index);
        self.handles.get_mut()[index as usize] = handle;

        // v. 标记为已创建
        self.conn_status[index as usize].store(true, Ordering::Release);
        log::warn!("{}-WS连接建立成功-idx={}", self.market_code, index);
        Ok(())
    }

    /// 订阅交易对
    ///
    /// # 参数
    /// * `pair` - 交易对标识符
    /// * `param` - 订阅参数
    /// * `idx` - 可选的连接索引，如果不指定则自动分配
    ///
    /// # 返回
    /// * `Result<(), Box<dyn std::error::Error>>` - 成功返回Ok(())，失败返回错误
    pub async fn subscribe(&self, pair: &str, param: Option<&str>, idx: Option<u8>) -> Result<(), Box<dyn std::error::Error>> {
        if self.pair_conn_map.get().contains_key(pair) {
            // 已订阅，直接返回
            return Ok(());
        }
        // 分配到某个连接上
        let id = match idx {
            Some(i) => i,
            None => self.get_next_idx().0,
        };

        // 确保连接已初始化
        self.init_connection(id).await?;
        let param = match param {
            Some(p) => p,
            None => &(self.build_sub_param)(pair),
        };
        if self.write(param, id, OpCode::Text).await {
            self.pair_conn_map.get_mut().insert(pair.to_string(), id);
        }
        return Ok(());
    }

    /**
     * 订阅一个数组
     */
    pub async fn subscribe_vec(
        &self,
        pairs: Vec<String>,
        param: &str,
        max_count: Option<u16>, // 批量订阅时，交易所每次发送的交易对最大数量
        subscribe_func: impl Fn(&Vec<String>, &str) -> String + Send + 'static,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // i. 去掉已经订阅过的交易对
        let mut to_sub_pairs: Vec<String> = pairs.into_iter().filter(|pair| !self.pair_conn_map.get().contains_key(pair)).collect();
        while !to_sub_pairs.is_empty() {
            // ii. 查找下一个连接索引
            let (idx, remaining) = self.get_next_idx();
            // iii. 确定这次订阅的最大批次，取决于 max_count 和当前连接剩余未订阅的交易对数量
            let max_sub_count = if let Some(max) = max_count { remaining.min(max as usize) } else { remaining };
            let max_sub_count = max_sub_count.min(to_sub_pairs.len());
            // iv. 按照订阅数量分割交易对
            let curr_sub_pairs = to_sub_pairs.drain(0..max_sub_count).collect();
            // v. 订阅交易对
            let sub_msg = subscribe_func(&curr_sub_pairs, param);
            self.init_connection(idx).await?;
            if self.write(&sub_msg, idx, OpCode::Text).await {
                // vi. 更新pair_conn_map
                self.pair_conn_map.get_mut().extend(curr_sub_pairs.into_iter().map(|pair| (pair, idx)));
            }
        }
        return Ok(());
    }

    /// 取消订阅交易对
    ///
    /// # 参数
    /// * `pair` - 交易对标识符
    /// * `param` - 取消订阅参数
    ///
    /// # 返回
    /// * `Result<u8, Box<dyn std::error::Error>>` - 成功返回使用的连接索引，失败返回错误
    pub async fn unsubscribe(&self, pair: &str, param: &str) -> Result<u8, Box<dyn std::error::Error>> {
        let mut result = u8::MAX;
        if let Some(idx) = self.pair_conn_map.get_mut().remove(pair) {
            self.write(param, idx, OpCode::Text).await;
            result = idx;
        }
        (self.clear_depth)(pair); // 清空深度数据

        // 更新连接的订阅对数
        if result != u8::MAX {
            let pair_count = self.get_sub_pairs(result).len();
            if pair_count == 0 {
                // 如果没有订阅的交易对了，关闭连接
                self.close_connection(result).await;
            }
        }
        Ok(result)
    }

    /**
    * 重新订阅 单个交易对
    * 参数：
          pair: 要重新订阅的交易对,
          sub_str: 订阅的字符串,
          unsub_str: 取消订阅的字符串
    */
    pub async fn restart_sub_one(&self, pair: &str, sub_str: &str, unsub_str: &str) {
        // 获取到 这个交易对所在的连接索引
        let index;
        if let Some(idx) = self.pair_conn_map.get().get(pair) {
            index = *idx;
        } else {
            // 未获取到，记录一下
            log::warn!("重新订阅时,未获取到所在的连接：{}-{}", self.market_code, pair);
            // 先随机分配一个连接
            let (new, _) = self.get_next_idx();
            self.pair_conn_map.get_mut().insert(pair.to_string(), new);
            index = new;
        }
        // 取消之前的订阅
        self.write(&unsub_str, index, OpCode::Text).await;
        (self.clear_depth)(pair); // 清空深度数据
                                  // 重新订阅
        self.write(&sub_str, index, OpCode::Text).await;
    }

    /**
    * 重新订阅一个数组
    * 传入参数：
           symbols: 要重新订阅的交易对数组,
           sub_param: 订阅参数,
           unsub_param: 取消订阅参数,
           max_count: 批量订阅时，交易所每次允许订阅的交易对最大数量,没限制传入None,
           subscribe_func: 订阅函数，用于构建订阅消息
    */
    pub async fn restart_subscribe_vec(
        &self,
        symbols: Vec<String>,
        sub_param: &str,
        unsub_param: &str,
        max_count: Option<u16>,
        subscribe_func: impl Fn(&Vec<String>, &str) -> String + Send + 'static,
    ) {
        let mut mapping: Vec<Vec<String>> = Vec::with_capacity(MAX_CONN_COUNT as usize);
        for _ in 0..MAX_CONN_COUNT {
            mapping.push(Vec::new());
        }
        // 获取到之前订阅的交易对所在的 连接索引
        for symbol in symbols.iter() {
            (self.clear_depth)(&symbol); // 清理深度数据
            if let Some(idx) = self.pair_conn_map.get_mut().remove(symbol) {
                mapping[idx as usize].push(symbol.clone());
            }
        }
        // 取消之前的订阅
        for (idx, vec) in mapping.iter().enumerate() {
            if vec.is_empty() {
                continue;
            }
            if let Some(max_count) = max_count {
                // 有上限
                let group_data: Vec<Vec<String>> = vec.chunks(max_count as usize).map(|chunk| chunk.to_vec()).collect();
                for group in group_data {
                    let unsub_msg = subscribe_func(&group, unsub_param);
                    self.write(&unsub_msg, idx as u8, OpCode::Text).await;
                }
            } else {
                // 无上限
                let unsub_msg = subscribe_func(vec, unsub_param);
                self.write(&unsub_msg, idx as u8, OpCode::Text).await;
            }
        }
        // 重新订阅
        let res = self.subscribe_vec(symbols, sub_param, max_count, subscribe_func).await;
        if let Err(e) = res {
            log::warn!("{}-重启交易对时，重新订阅交易对失败,Error:{}，订阅的交易对：{}", self.market_code, e, sub_param);
        }
    }

    /// 批量取消订阅交易对
    /// 针对的是不再需要的交易对
    ///
    /// # 参数
    /// * `pair` - 交易对数组
    /// * `param` - 取消订阅参数
    /// * `subscribe_func` - 取消订阅的订阅函数，用于构建订阅消息
    /// * `max_count` - 批量订阅时，交易所每次允许订阅的交易对最大数量,没限制传入None,
    pub async fn unsubscribe_vec(
        &self,
        pairs: Vec<String>,
        param: &str,
        max_count: Option<u16>,
        subscribe_func: impl Fn(&Vec<String>, &str) -> String + Send + 'static,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut mapping: Vec<Vec<String>> = Vec::with_capacity(MAX_CONN_COUNT);
        for _ in 0..MAX_CONN_COUNT {
            mapping.push(Vec::new());
        }
        // 获取到之前订阅的交易对所在的 连接索引
        for pair in pairs {
            (self.clear_depth)(&pair); // 清空深度数据
            if let Some(idx) = self.pair_conn_map.get_mut().remove(&pair) {
                mapping[idx as usize].push(pair);
            }
        }

        for (idx, vec) in mapping.iter().enumerate() {
            if vec.is_empty() {
                continue;
            }
            if let Some(max_count) = max_count {
                // 有上限
                let group_data: Vec<Vec<String>> = vec.chunks(max_count as usize).map(|chunk| chunk.to_vec()).collect();
                for group in group_data {
                    let unsub_msg = subscribe_func(&group, param);
                    self.write(&unsub_msg, idx as u8, OpCode::Text).await;
                }
            } else {
                let unsub_msg = subscribe_func(vec, param);
                self.write(&unsub_msg, idx as u8, OpCode::Text).await;
            }
        }

        for (idx, vec) in mapping.iter().enumerate() {
            if vec.is_empty() {
                continue;
            }
            let pair_count = self.get_sub_pairs(idx as u8).len();
            if pair_count == 0 {
                // 如果没有订阅的交易对了，关闭连接
                self.close_connection(idx as u8).await;
            }
        }

        Ok(())
    }

    /// 向指定连接发送数据
    ///
    /// # 参数
    /// * `data` - 要发送的字符串数据
    /// * `idx` - 连接索引
    /// * `op_code` - WebSocket操作码
    ///
    /// # 返回
    /// * `Result<(), Box<dyn std::error::Error>>` - 成功返回Ok(())，失败返回错误
    pub async fn write(&self, data: &str, idx: u8, op_code: OpCode) -> bool {
        // 检查写入是否已经限速
        let last_write_time = self.last_write_time[idx as usize].load(Ordering::Acquire);
        wait_util_ms(self.write_wait_time.load(Ordering::SeqCst), last_write_time).await;

        let res = self.writers.get_mut()[idx as usize].write_frame(data.to_string(), op_code).await;
        if let Err(e) = res {
            log::warn!("{}-WS写入失败-idx={},Error:{},写入内容：{}", self.market_code, idx, e, data);
            return false;
        }

        self.last_write_time[idx as usize].store(tokio::time::Instant::now().elapsed().as_millis() as u64, Ordering::Release);
        return true;
    }

    /// 关闭所有WebSocket连接
    ///
    /// 发送关闭帧到所有连接并释放资源
    pub async fn close(&self) {
        // 关闭所有连接
        for idx in 0..MAX_CONN_COUNT {
            // 判断当前链接是否已经是关闭状态
            if !self.conn_status[idx as usize].load(Ordering::Acquire) {
                continue;
            }
            self.close_connection(idx as u8).await;
        }
        // abort 写入handle以及重连handle
        Self::set_link_sate(&self, false);
    }

    /// 关闭指定连接
    pub async fn close_connection(&self, idx: u8) -> Vec<String> {
        log::warn!("{}-WS关闭连接-idx={}", self.market_code, idx);
        // Close the connection
        self.write("", idx, OpCode::Close).await;
        self.clear_conn_status(idx).await
    }
    /// 清空连接状态，包括读取任务、深度数据、连接状态、pair_conn_map
    async fn clear_conn_status(&self, idx: u8) -> Vec<String> {
        // Abort the read task
        self.handles.get_mut()[idx as usize].abort();
        // 清空深度数据
        let pairs = self.get_sub_pairs(idx);
        for pair in pairs {
            (self.clear_depth)(&pair);
        }
        // 设置连接状态
        self.conn_status[idx as usize].store(false, Ordering::Release);
        // 清空pair_conn_map
        let pair_conn_map = self.pair_conn_map.get_mut();
        let curr_conn_pairs = pair_conn_map.iter().filter(|&(_, &v)| v == idx).map(|(k, _)| k.clone()).collect::<Vec<String>>();
        pair_conn_map.retain(|_, &mut v| v != idx);
        return curr_conn_pairs;
    }

    /// 尝试重新连接WebSocket
    pub async fn reconnect(&self, idx: u8) {
        log::warn!("{}-WS链接开始重连-idx={}，当前连接订阅的交易对：{}", self.market_code, idx, self.get_sub_pairs(idx).join(","));
        // 清空连接状态，这里不需要关闭连接，只需要清空状态，因为触发重连的地方连接已经为关闭状态。
        let pairs_in_conn = self.clear_conn_status(idx).await;
        // Reconnect
        match self.init_connection(idx).await {
            Ok(_) => {
                // 重新连接成功
                log::warn!("{}-WS重新连接成功-idx={}", self.market_code, idx);
            }
            Err(e) => {
                // 重新连接失败
                log::warn!("{}-WS重新连接失败-idx={},Error:{}", self.market_code, idx, e);
                // **重要**:重新连接失败，可能是网络问题或者服务器问题，这里需要将连接状态置为true，从而下次心跳还会检测活跃时间，发起第二次重连
                self.conn_status[idx as usize].store(true, Ordering::Release);
                // 重新设置pairmap
                for pair in pairs_in_conn {
                    self.pair_conn_map.get_mut().insert(pair, idx);
                }
                return;
            }
        };
        // Subscribe pairs
        for pair in pairs_in_conn {
            self.subscribe(&pair, None, Some(idx)).await.unwrap();
        }
    }

    /// 分割函数
    /// # 参数
    /// * `vec` -是需要分割的数组
    /// * `count` -是需要分割为几个
    /// # 返回
    /// 返回值是分割后的结果 如果超过count则会有10个vec
    ///  如果不超过10个count则会有 vec.len()个vec
    pub fn split_vec(vec: Vec<String>, count: u8) -> Vec<Vec<String>> {
        let mut vec_ret = vec![];
        for _ in 0..count {
            vec_ret.push(vec![]);
        }
        let mut index = 0;
        for item in vec {
            vec_ret[index].push(item);
            index = (index + 1) % count as usize;
        }
        let mut real_ret_vec = vec![];
        for vec in vec_ret {
            if vec.len() != 0 {
                real_ret_vec.push(vec);
            }
        }
        real_ret_vec
    }
    /// 获取下一个连接索引
    /// 返回值是一个元组，包含连接索引和当前索引剩余的订阅数量
    fn get_next_idx(&self) -> (u8, usize) {
        // 寻找当前还没满足订阅数量的连接索引，如果没有，则使用一个新连接
        let mut next_to_init = MAX_CONN_COUNT;
        let mut idx = MAX_CONN_COUNT;
        // 余量
        let mut remaining = 0;
        for i in 0..MAX_CONN_COUNT {
            let pair_count = self.get_sub_pairs(i as u8).len();
            if pair_count == 0 {
                // 保存id最小的未初始化的连接
                if next_to_init == MAX_CONN_COUNT {
                    next_to_init = i;
                }
            } else if pair_count < self.max_pairs_per_conn as usize {
                // 找到一个可以使用的连接
                idx = i;
                remaining = self.max_pairs_per_conn as usize - pair_count;
                break;
            }
        }
        if idx == MAX_CONN_COUNT {
            // 没有找到合适的索引，使用next_to_init
            idx = next_to_init;
            remaining = self.max_pairs_per_conn as usize;
        }
        return (idx as u8, remaining);
    }
    /// 获取当前还存在交易对订阅的连接
    pub async fn get_connected_conn(&self) -> Vec<u8> {
        let mut result = vec![];
        for i in 0..MAX_CONN_COUNT {
            let pair_status = self.conn_status[i as usize].load(Ordering::Acquire);
            if pair_status {
                result.push(i as u8);
            }
        }
        return result;
    }

    /// 组装WebSocket Header
    fn build_header(&self) -> Vec<(String, String)> {
        let headers_guard = self.headers.get();
        if headers_guard.is_empty() {
            return vec![];
        }
        let mut headers = vec![];
        for (k, v) in headers_guard.iter() {
            headers.push((k.clone(), v.clone()));
        }
        // 添加时间戳，需要按照不同交易所的要求来添加
        // BtcTurk HFT市场
        headers.push(("X-Stamp".to_string(), format!("{}", chrono::Utc::now().timestamp_millis())));
        headers
    }
}
