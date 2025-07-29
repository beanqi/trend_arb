
use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream;

pub async fn connect_fix(host: &str, port: u16) -> Result<(tokio::io::ReadHalf<TlsStream<TcpStream>>, tokio::io::WriteHalf<TlsStream<TcpStream>>), Box<dyn std::error::Error + Send + Sync>> {
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
    let tls = connector.connect(server_name, tcp_stream)
        .await
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync + 'static>)?;

    let (reader, writer) = tokio::io::split(tls);
    return Ok((reader, writer));
}
