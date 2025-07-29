/********************************************************************************************************
 *
 * FIX (Financial Information eXchange) 协议报文结构
 *
 * @author: 刘本强
 * @date: 2025-07-04
 *
 * 描述:
 * FIX 报文是一个由 ASCII 字符组成的序列。它由三个部分组成：标准报头 (Standard Header)、
 * 报文主体 (Body) 和标准报尾 (Standard Trailer)。
 *
 * 每个字段都是一个 "Tag=Value" 格式的键值对，字段之间由 SOH (Start of Header, ASCII 0x01)
 * 字符分隔。在本文档中，我们用 <SOH> 来代表这个不可见的字符。
 *
 * 报文通用格式: 8=...<SOH>9=...<SOH>35=...<SOH>...<SOH>10=...<SOH>
 *
 ********************************************************************************************************/

/*
+------------------------+-----------+---------------------+----------+---------------------------------------------------------------------------------+
|          段            |  标签 (Tag) |     字段名称        | 是否必须? |                                       说 明                                         |
+------------------------+-----------+---------------------+----------+---------------------------------------------------------------------------------+
|                        |           |                     |          |                                                                                 |
|       标准报头         |     8     |     BeginString     |    Y     | FIX协议版本。例如：FIX.4.2, FIX.4.4, FIXT.1.1。总是报文的第一个字段。               |
|   (Standard Header)    +-----------+---------------------+----------+---------------------------------------------------------------------------------+
|                        |     9     |     BodyLength      |    Y     | 报文主体的长度。从 Tag 35 开始计算，直到 Tag 10 之前的所有字符数。                |
|                        +-----------+---------------------+----------+---------------------------------------------------------------------------------+
|                        |    35     |       MsgType       |    Y     | 报文类型。定义了报文的业务目的。例如 'D' 代表新订单, '8' 代表执行报告。       |
|                        +-----------+---------------------+----------+---------------------------------------------------------------------------------+
|                        |    49     |     SenderCompID    |    Y     | 发送方的唯一标识符。                                                            |
|                        +-----------+---------------------+----------+---------------------------------------------------------------------------------+
|                        |    56     |     TargetCompID    |    Y     | 接收方的唯一标识符。                                                            |
|                        +-----------+---------------------+----------+---------------------------------------------------------------------------------+
|                        |    34     |       MsgSeqNum     |    Y     | 消息序列号。由发送方维护，每个会话中单调递增。                                    |
|                        +-----------+---------------------+----------+---------------------------------------------------------------------------------+
|                        |    52     |      SendingTime    |    Y     | 报文发送时间 (UTC)。格式: YYYYMMDD-HH:MM:SS.sss                                 |
|                        |           |                     |          |                                                                                 |
+------------------------+-----------+---------------------+----------+---------------------------------------------------------------------------------+
|                        |           |                     |          |                                                                                 |
|                        |           |                     |          | 报文的主体内容完全由报头中的 MsgType (Tag 35) 决定。                            |
|       报文主体         |           |                     |          | 以下以一个 "新订单" (New Order - Single, MsgType='D') 报文为例。                |
|         (Body)         +-----------+---------------------+----------+---------------------------------------------------------------------------------+
|    (示例: MsgType=D)   |    11     |       ClOrdID       |    Y     | 客户端指定的订单唯一ID。                                                        |
|                        +-----------+---------------------+----------+---------------------------------------------------------------------------------+
|                        |    55     |        Symbol       |    Y     | 交易品种的标识。例如：AAPL, GOOG。                                              |
|                        +-----------+---------------------+----------+---------------------------------------------------------------------------------+
|                        |    54     |         Side        |    Y     | 交易方向。1 = 买入 (Buy), 2 = 卖出 (Sell)。                                     |
|                        +-----------+---------------------+----------+---------------------------------------------------------------------------------+
|                        |    60     |     TransactTime    |    Y     | 交易发生时间 (UTC)。格式: YYYYMMDD-HH:MM:SS.sss                                 |
|                        +-----------+---------------------+----------+---------------------------------------------------------------------------------+
|                        |    38     |       OrderQty      |    Y     | 订单数量。                                                                      |
|                        +-----------+---------------------+----------+---------------------------------------------------------------------------------+
|                        |    40     |        OrdType      |    Y     | 订单类型。1 = 市价 (Market), 2 = 限价 (Limit)。                                   |
|                        +-----------+---------------------+----------+---------------------------------------------------------------------------------+
|                        |    44     |         Price       |    C     | 价格。当 OrdType=2 (限价单) 时必须提供。                                        |
|                        |           |                     |          |                                                                                 |
+------------------------+-----------+---------------------+----------+---------------------------------------------------------------------------------+
|                        |           |                     |          |                                                                                 |
|       标准报尾         |    10     |       CheckSum      |    Y     | 校验和。将报文中从 Tag 8 开始到 Tag 10 之前的所有字节的 ASCII 值相加，     |
|   (Standard Trailer)   |           |                     |          | 对 256 取模。结果是一个 3 位数，不足 3 位前面补零。总是报文的最后一个字段。  |
|                        |           |                     |          |                                                                                 |
+------------------------+-----------+---------------------+----------+---------------------------------------------------------------------------------+
*/

use std::{cell::RefCell, sync::atomic::AtomicU32};

use base64::{engine::general_purpose, Engine};
use bytes::BytesMut;
use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256, Sha512};

type HmacSha512 = Hmac<Sha512>;
pub const SOH: u8 = 0x01;

// SAFETY: 这个结构体是线程安全的，多线程下，只有sender_comp_id 会被修改，其他字段都是只读的。
// 且对 sender_comp_id 的修改是串行的，只会初始化一次，只有发生变化的时候才会设置。
unsafe impl Send for FixMessageBuilder {}
unsafe impl Sync for FixMessageBuilder {}

pub struct FixMessageBuilder {
    pub market: String,         // 市场标识符
    pub fix_version: String,    // FIX 协议版本
    pub sender_comp_id: RefCell<String>, // 发送方公司标识符
    pub target_comp_id: String, // 接收方公司标识符
    pub msg_seq_num: AtomicU32, // 消息序列号
}

impl FixMessageBuilder {
    pub fn new(market: &str, fix_version: &str, sender_comp_id: &str, target_comp_id: &str) -> Self {
        FixMessageBuilder {
            market: market.to_string(),
            fix_version: fix_version.to_string(),
            sender_comp_id: RefCell::new(sender_comp_id.to_string()),
            target_comp_id: target_comp_id.to_string(),
            msg_seq_num: AtomicU32::new(1), // 初始序列号为 1
        }
    }

    pub fn set_sender_comp_id(&self, sender_comp_id: String) {
        if sender_comp_id == self.sender_comp_id.borrow().as_str() {
            return; // 如果没有变化，就不需要设置
        }

        self.sender_comp_id.replace(sender_comp_id);
    }

    pub fn reset_seq_num(&self) {
        self.msg_seq_num.store(1, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn checksum(&self, bytes: &[u8]) -> u8 {
        let sum: u32 = bytes.iter().map(|&b| b as u32).sum();
        (sum % 256) as u8
    }

    /// 构造完整 FIX 消息（自动填 8=、9=、10=）
    /// # Arguments
    /// * `fields` - 包含所有字段的向量，**只需要Body，不需要Head**，每个字段是一个元组 (tag, value)，其中 tag 是字段的标签，value 是字段的值。
    /// * `msg_type` - 消息类型，例如 'D' 代表新订单，'8' 代表执行报告等。
    /// # Returns
    /// 返回一个包含完整 FIX 消息的字节向量。
    pub fn build_fix_message(&self, fields: Vec<(u32, String)>, msg_type: &str, seq_num: Option<u32>, curr_time: Option<DateTime<Utc>>) -> Vec<u8> {
        let msg_seq_num = seq_num.unwrap_or_else(|| self.msg_seq_num.fetch_add(1, std::sync::atomic::Ordering::Relaxed));

        let msg_timestamp = curr_time
            .map(|ts| ts.format("%Y%m%d-%H:%M:%S.%3f").to_string())
            .unwrap_or_else(|| chrono::Utc::now().format("%Y%m%d-%H:%M:%S.%3f").to_string());

        let mut body = Vec::<u8>::new();
        body.push(SOH);
        body.extend_from_slice(format!("35={}", msg_type).as_bytes());
        body.push(SOH);
        body.extend_from_slice(format!("34={}", msg_seq_num).as_bytes());
        body.push(SOH);
        body.extend_from_slice(format!("49={}", self.sender_comp_id.borrow()).as_bytes());
        body.push(SOH);
        body.extend_from_slice(format!("52={}", msg_timestamp).as_bytes());
        body.push(SOH);
        body.extend_from_slice(format!("56={}", self.target_comp_id).as_bytes());
        body.push(SOH);

        for (tag, val) in fields {
            body.extend_from_slice(format!("{tag}={val}").as_bytes());
            body.push(SOH);
        }

        let mut msg = Vec::<u8>::new();
        msg.extend_from_slice(format!("8={}", self.fix_version).as_bytes());
        msg.push(SOH);
        msg.extend_from_slice(format!("9={}", body.len() - 1).as_bytes());
        msg.extend_from_slice(&body);
        let cksm = self.checksum(&msg);
        msg.extend_from_slice(format!("10={:0>3}", cksm).as_bytes());
        msg.push(SOH); // 添加结尾的 SOH
        msg
    }
    /// 从缓冲区中提取完整的 FIX 消息，如果缓冲区中没有完整的消息，则返回 None，等待再次调用提取。
    pub fn extract_fix_message(&self, buf: &mut BytesMut) -> Option<BytesMut> {
        // 至少要能容纳 "8=FIX*<SOH>9=0<SOH>10=000<SOH>"
        if buf.len() < 18 {
            return None;
        }

        // 1. 校验开始必须是 8=
        if !buf.starts_with(b"8=") {
            log::warn!(
                "Invalid FIX message start in extract_fix_message: buffer={:?}, market={}",
                buf,
                self.market
            );
            buf.clear(); // 清空缓冲区
            return None; // 不是有效的 FIX 消息
        }

        // 2. 找到第一个 SOH → tag 8 的结尾
        let begin_end = match memchr::memchr(SOH, &buf[..]) {
            Some(i) => i,
            None => return None, // 未收到完整 tag8
        };

        // 3. 紧接着必须是 "9="
        let after_begin = begin_end + 1;
        if buf.len() < after_begin + 2 || &buf[after_begin..after_begin + 2] != b"9=" {
            log::warn!(
                "Invalid FIX message format in extract_fix_message: buffer={:?}, market={}",
                buf,
                self.market
            );
            buf.clear(); // 清空缓冲区
            return None; // 不是有效的 FIX 消息
        }

        // 4. 找到 tag 9 的 SOH，解析 BodyLength
        let body_len_start = after_begin + 2;
        let body_len_end = match memchr::memchr(SOH, &buf[body_len_start..]) {
            Some(rel) => body_len_start + rel,
            None => return None, // tag9 未完
        };

        let body_len: Option<usize> = std::str::from_utf8(&buf[body_len_start..body_len_end]).ok().and_then(|s| s.parse().ok());
        if body_len.is_none() {
            log::warn!(
                "Invalid BodyLength in FIX message in extract_fix_message: buffer={:?}, market={}",
                buf,
                self.market
            );
            buf.clear(); // 清空缓冲区
            return None; // 不是有效的 FIX 消息
        }
        let body_len = body_len.unwrap();

        // 5. 已知完整报文总长度
        //    = header_len (到 tag9 末尾) + body_len + checksum(7B)
        let header_len = body_len_end + 1; // +1 把 tag9 的 SOH 算进去
        let total_len = header_len + body_len + 7; // 7 = "10=000<SOH>" 的长度

        if buf.len() < total_len {
            // 还没收完
            return None;
        }

        // 6. 把完整帧切出来
        let frame = buf.split_to(total_len);
        Some(frame)
    }

    /// 构建 Kraken 的登录消息
    /// 构建登陆消息的时候，需要重置消息序列号为 2，因为 Kraken 的登录消息是第一个消息。
    /// # Arguments
    /// * `api_key` - API Key
    /// * `api_secret` - API Secret
    ///
    pub fn build_kraken_logon_message(&self, api_key: &str, api_secret: &str) -> Vec<u8> {
        self.msg_seq_num.store(2, std::sync::atomic::Ordering::Relaxed);
        let curr_time = chrono::Utc::now();
        let nonce: String = curr_time.timestamp_millis().to_string();
        let message_input = format!(
            "35=A{soh}34=1{soh}49={sender}{soh}56={target}{soh}553={api_key}{soh}",
            soh = '\x01',
            sender = self.sender_comp_id.borrow(),
            target = self.target_comp_id,
            api_key = api_key
        );
        let password = Self::get_password(&message_input, api_secret, &nonce).unwrap();

        let body = vec![
            (98, String::from("0")),
            (108, String::from("60")),
            (141, String::from("N")),   // Heartbeat Interval
            (553, api_key.to_string()), // API Key
            (554, password),            // Password
            (5025, nonce),              // Timestamp
        ];

        return self.build_fix_message(body, "A", Some(1), Some(curr_time));
    }

    pub fn get_password(message_input: &str, api_secret_b64: &str, nonce: &str) -> Result<String, Box<dyn std::error::Error>> {
        // 1) sha256(message_input + nonce)
        let mut sha256 = Sha256::new();
        sha256.update(message_input.as_bytes());
        sha256.update(nonce.as_bytes());
        let sha256_digest = sha256.finalize(); // 32 bytes

        // 2) hmac_sha512(secret, sha256_digest)
        let secret_bytes = general_purpose::STANDARD.decode(api_secret_b64)?;
        let mut mac = HmacSha512::new_from_slice(&secret_bytes)?;
        mac.update(&sha256_digest);
        let hmac_result = mac.finalize().into_bytes();

        // 3) base64(hmac_result)
        let password_b64 = general_purpose::STANDARD.encode(hmac_result);

        Ok(password_b64)
    }

    /// 从FIX消息中提取某个字段的值
    /// # Arguments
    /// * `frame` - FIX 消息的字节切片
    /// * `key` - 要提取的字段的标签 (Tag)，例如 11 表示 ClOrdID，55 表示 Symbol 等。
    /// * `first_stop` - 是否是第一个停止点，如果是，则只提取第一个匹配的字段
    /// # Returns
    /// 返回一个包含字段值的字符串向量。如果没有找到对应的字段，则返回空向量。
    pub fn get_field_value<'a>(&self, frame: &'a [u8], key: u32, first_stop: bool) -> Vec<&'a str> {
        let mut values = Vec::new();

        let mut idx = 0;
        let len = frame.len();

        while idx < len {
            // 找本字段结尾（下一个 SOH）
            let end = memchr::memchr(SOH, &frame[idx..]).map_or(len, |rel| idx + rel);
            let field = &frame[idx..end];

            // 找 '=' 分隔符
            if let Some(eq_pos) = memchr::memchr(b'=', field) {
                // 解析 tag（bytes -> u32，避免生成 String）
                let mut tag_num: u32 = 0;
                let mut valid_tag = true;
                for &b in &field[..eq_pos] {
                    if !(b'0'..=b'9').contains(&b) {
                        valid_tag = false;
                        break;
                    }
                    tag_num = tag_num * 10 + (b - b'0') as u32;
                }

                if valid_tag && tag_num == key {
                    // 取得 value 部分
                    if let Ok(val_str) = std::str::from_utf8(&field[eq_pos + 1..]) {
                        values.push(val_str);
                        if first_stop {
                            break;
                        }
                    }
                }
            }

            // 跳到下一个字段（越过当前 SOH）
            idx = end + 1;
        }

        values
    }
}
