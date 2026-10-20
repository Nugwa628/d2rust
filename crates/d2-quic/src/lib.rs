//! The QUIC stream layer: how every online client reaches a D2RUST server.
//!
//! One QUIC connection carries one player session. Inside it, each
//! bidirectional stream is a [`FramedStream`]: [`d2_net::wire`] frames in
//! order, both ways. The client opens the first stream (the control
//! stream: handshake, account, lobby) when it connects, and a second one
//! (the game stream) when it joins a game. This crate only moves frame
//! bodies; it holds no handshake and reads no message. The handshake and
//! the messages are `d2_net`'s, driven by the client library and the
//! server.
//!
//! What QUIC adds over a plain stream:
//!
//! - **Encryption and the server's identity.** Every connection is TLS 1.3.
//!   The server presents its certificate; the client checks it according
//!   to its [`ServerTrust`]: online play pins the SHA-256 [`fingerprint`]
//!   the signed server list names.
//! - **Keepalive and timeouts.** QUIC keeps a quiet connection alive by
//!   itself ([`Timing::keepalive`]) and closes one that has heard nothing
//!   for [`Timing::timeout`]; the streams then read [`NetError::TimedOut`].
//! - **Connection migration.** A client whose address changes (Wi-Fi to
//!   mobile data, a NAT that rebinds) keeps its connection: the server
//!   follows it to the new address and the game notices nothing.
//!
//! The async QUIC machinery runs on tokio: a client on a small runtime of
//! its own, a server on the runtime it is given. The stream ends are
//! channel-backed, so the synchronous game side never waits: `send` queues
//! and returns, `recv` returns what has arrived.

use std::net::{SocketAddr, UdpSocket};
use std::sync::mpsc as std_mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use quinn::crypto::rustls::{QuicClientConfig, QuicServerConfig};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use sha2::{Digest, Sha256};
use tokio::runtime::{Handle, Runtime};
use tokio::sync::mpsc;

use d2_net::wire::{
    body_len, frame, kind, packet_body, parse_body, Body, ConnectError, Timing, MAX_FRAME,
};
use d2_net::{NetError, Packet, Transport};

/// The application protocol name both ends offer in the TLS handshake.
/// It names the stream layout (control stream, then game streams), so a
/// client of another layout fails the TLS handshake instead of misreading.
pub const ALPN: &[u8] = b"d2rust/4";

/// The server name a client asks for. Servers are reached by address, so
/// the name only has to agree with the server's own certificate.
pub const SERVER_NAME: &str = "d2rust";

/// How long a closing end waits for its goodbye to be acknowledged.
const CLOSE_WAIT: Duration = Duration::from_millis(500);

/// Batched game-stream bytes past this go out before the frame ends, so a
/// huge frame never sits in memory whole.
const BATCH_MAX: usize = 1 << 18;

/// How a client decides whether to trust a server's certificate.
///
/// There is no default: every caller names its trust, so server
/// authentication is never turned off by accident.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServerTrust {
    /// Any certificate: the connection is encrypted but the server is not
    /// authenticated. Tests and the dev console only.
    Any,
    /// Only the certificate with this SHA-256 [`fingerprint`].
    Pinned([u8; 32]),
}

/// A certificate's fingerprint: the SHA-256 of its DER bytes.
pub fn fingerprint(cert_der: &[u8]) -> [u8; 32] {
    Sha256::digest(cert_der).into()
}

/// Reads a fingerprint written as 64 hex digits (either case).
pub fn parse_fingerprint(hex: &str) -> Option<[u8; 32]> {
    let hex = hex.trim().as_bytes();
    if hex.len() != 64 {
        return None;
    }
    let digit = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
    let mut out = [0; 32];
    for (o, pair) in out.iter_mut().zip(hex.chunks(2)) {
        *o = digit(pair[0])? << 4 | digit(pair[1])?;
    }
    Some(out)
}

/// A fingerprint as 64 lowercase hex digits, as the server list writes it.
pub fn fingerprint_hex(fp: &[u8; 32]) -> String {
    fp.iter().map(|b| format!("{b:02x}")).collect()
}

/// The certificate and private key a server presents.
#[derive(Clone, Debug)]
pub struct Identity {
    /// The certificate, DER.
    pub cert: Vec<u8>,
    /// Its PKCS#8 private key, DER.
    pub key: Vec<u8>,
}

impl Identity {
    /// A fresh self-signed certificate for [`SERVER_NAME`].
    pub fn self_signed() -> Result<Self, ConnectError> {
        let made = rcgen::generate_simple_self_signed(vec![SERVER_NAME.to_string()])
            .map_err(|e| io_err(format!("making a certificate: {e}")))?;
        Ok(Self {
            cert: made.cert.der().to_vec(),
            key: made.key_pair.serialize_der(),
        })
    }
}

fn io_err(msg: impl Into<String>) -> ConnectError {
    ConnectError::Io(std::io::Error::other(msg.into()))
}

fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

fn transport_config(timing: Timing) -> Result<Arc<quinn::TransportConfig>, ConnectError> {
    let mut t = quinn::TransportConfig::default();
    let idle = quinn::IdleTimeout::try_from(timing.timeout)
        .map_err(|_| io_err("timeout too long for QUIC"))?;
    t.max_idle_timeout(Some(idle));
    t.keep_alive_interval(Some(timing.keepalive));
    Ok(Arc::new(t))
}

fn client_runtime() -> Result<Runtime, ConnectError> {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .thread_name("d2-net-quic")
        .enable_all()
        .build()
        .map_err(ConnectError::Io)
}

/// Accepts the certificates its [`ServerTrust`] allows; signatures are
/// always checked, so the server must hold the certificate's key.
#[derive(Debug)]
struct TrustCheck {
    trust: ServerTrust,
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for TrustCheck {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        match &self.trust {
            ServerTrust::Any => Ok(ServerCertVerified::assertion()),
            ServerTrust::Pinned(fp) if *fp == fingerprint(end_entity.as_ref()) => {
                Ok(ServerCertVerified::assertion())
            }
            ServerTrust::Pinned(_) => Err(rustls::Error::InvalidCertificate(
                rustls::CertificateError::UnknownIssuer,
            )),
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

// ---------------------------------------------------------- the streams

/// What a stream's reader hands its owner.
enum In {
    Body(Vec<u8>),
    Ended(NetError),
}

/// What a stream's owner hands its writer.
enum Out {
    /// Whole frames, already framed.
    Bytes(Vec<u8>),
    /// Say goodbye on this stream and finish it; signal when done.
    Close(Option<std_mpsc::Sender<()>>),
}

/// The error a failed stream read means for the game.
fn read_err(e: quinn::ReadExactError) -> NetError {
    use quinn::{ConnectionError, ReadError, ReadExactError};
    match e {
        ReadExactError::ReadError(ReadError::ConnectionLost(ConnectionError::TimedOut)) => {
            NetError::TimedOut
        }
        _ => NetError::Closed,
    }
}

async fn read_frame(recv: &mut quinn::RecvStream) -> Result<Vec<u8>, NetError> {
    let mut header = [0; 4];
    recv.read_exact(&mut header).await.map_err(read_err)?;
    let mut body = vec![0; body_len(header)?];
    recv.read_exact(&mut body).await.map_err(read_err)?;
    Ok(body)
}

/// Reads frame bodies off `recv` until the stream ends. Pings are eaten;
/// a goodbye ends the stream cleanly; a broken frame closes the whole
/// connection.
async fn read_loop(
    conn: quinn::Connection,
    mut recv: quinn::RecvStream,
    to: mpsc::UnboundedSender<In>,
) {
    let ended = loop {
        match read_frame(&mut recv).await {
            Ok(b) if b == [kind::PING] => {}
            Ok(b) if b == [kind::BYE] => break NetError::Closed,
            Ok(b) => {
                if to.send(In::Body(b)).is_err() {
                    return;
                }
            }
            Err(NetError::Malformed) => {
                conn.close(1u32.into(), b"malformed");
                break NetError::Malformed;
            }
            Err(e) => break e,
        }
    };
    let _ = to.send(In::Ended(ended));
}

/// Writes what the owner sends until it closes (or drops) its end, then
/// says goodbye on this stream. The connection stays open.
async fn write_loop(mut send: quinn::SendStream, mut from: mpsc::UnboundedReceiver<Out>) {
    let mut done = None;
    while let Some(o) = from.recv().await {
        match o {
            Out::Bytes(b) => {
                if send.write_all(&b).await.is_err() {
                    return;
                }
            }
            Out::Close(d) => {
                done = d;
                break;
            }
        }
    }
    if send.write_all(&frame(&[kind::BYE])).await.is_ok() && send.finish().is_ok() {
        let _ = tokio::time::timeout(CLOSE_WAIT, send.stopped()).await;
    }
    if let Some(d) = done {
        let _ = d.send(());
    }
}

fn start_stream(
    rt: &Handle,
    conn: quinn::Connection,
    send: quinn::SendStream,
    recv: quinn::RecvStream,
    wait_on_close: bool,
) -> FramedStream {
    let (out_tx, out_rx) = mpsc::unbounded_channel();
    let (in_tx, in_rx) = mpsc::unbounded_channel();
    rt.spawn(read_loop(conn, recv, in_tx));
    rt.spawn(write_loop(send, out_rx));
    FramedStream {
        out: out_tx,
        incoming: in_rx,
        ended: None,
        wait_on_close,
    }
}

/// One bidirectional stream of a connection, carrying frame bodies in
/// order both ways. Neither call ever blocks.
pub struct FramedStream {
    out: mpsc::UnboundedSender<Out>,
    incoming: mpsc::UnboundedReceiver<In>,
    /// Set once the stream is over: every later call returns it.
    ended: Option<NetError>,
    /// A client's [`FramedStream::close`] waits (briefly) for its goodbye
    /// to leave; a server's, running on its runtime, never waits.
    wait_on_close: bool,
}

impl std::fmt::Debug for FramedStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FramedStream")
            .field("ended", &self.ended)
            .finish()
    }
}

impl FramedStream {
    /// Queues one frame body. A body must be 1..=[`MAX_FRAME`] bytes.
    pub fn send(&mut self, body: Vec<u8>) -> Result<(), NetError> {
        if body.is_empty() || body.len() > MAX_FRAME {
            return Err(NetError::Malformed);
        }
        self.send_bytes(frame(&body))
    }

    fn send_bytes(&mut self, bytes: Vec<u8>) -> Result<(), NetError> {
        if let Some(e) = &self.ended {
            return Err(e.clone());
        }
        self.out
            .send(Out::Bytes(bytes))
            .map_err(|_| self.end(NetError::Closed))
    }

    fn end(&mut self, e: NetError) -> NetError {
        self.ended.get_or_insert(e).clone()
    }

    fn take(&mut self, got: In) -> Result<Vec<u8>, NetError> {
        match got {
            In::Body(b) => Ok(b),
            In::Ended(e) => Err(self.end(e)),
        }
    }

    /// The next body that has arrived, or `None`. [`NetError::Closed`]
    /// once the other end said goodbye and everything before it was read.
    pub fn recv(&mut self) -> Result<Option<Vec<u8>>, NetError> {
        if let Some(e) = &self.ended {
            return Err(e.clone());
        }
        match self.incoming.try_recv() {
            Ok(got) => self.take(got).map(Some),
            Err(mpsc::error::TryRecvError::Empty) => Ok(None),
            Err(mpsc::error::TryRecvError::Disconnected) => Err(self.end(NetError::Closed)),
        }
    }

    /// [`FramedStream::recv`], waiting at most `wait` for a body (for
    /// handshakes on the synchronous side; never on a runtime thread).
    pub fn recv_timeout(&mut self, wait: Duration) -> Result<Option<Vec<u8>>, NetError> {
        let deadline = Instant::now() + wait;
        loop {
            if let Some(b) = self.recv()? {
                return Ok(Some(b));
            }
            let now = Instant::now();
            if now >= deadline {
                return Ok(None);
            }
            std::thread::sleep((deadline - now).min(Duration::from_millis(1)));
        }
    }

    /// The next body, waiting for it (for the server's tokio loop).
    pub async fn recv_async(&mut self) -> Result<Vec<u8>, NetError> {
        if let Some(e) = &self.ended {
            return Err(e.clone());
        }
        match self.incoming.recv().await {
            Some(got) => self.take(got),
            None => Err(self.end(NetError::Closed)),
        }
    }

    /// Ends this stream cleanly: a goodbye, then the stream finishes. The
    /// connection and its other streams stay open. Later calls return
    /// [`NetError::Closed`].
    pub fn close(&mut self) {
        if self.ended.is_some() {
            return;
        }
        self.ended = Some(NetError::Closed);
        if self.wait_on_close {
            let (done_tx, done_rx) = std_mpsc::channel();
            if self.out.send(Out::Close(Some(done_tx))).is_ok() {
                let _ = done_rx.recv_timeout(CLOSE_WAIT * 2);
            }
        } else {
            let _ = self.out.send(Out::Close(None));
        }
    }
}

impl Drop for FramedStream {
    fn drop(&mut self) {
        // Dropping the sender also ends the writer with a goodbye; never wait here.
        if self.ended.is_none() {
            let _ = self.out.send(Out::Close(None));
        }
    }
}

/// A game stream as a [`Transport`]: [`d2_net::wire`] game packets. The
/// server's end collects one frame's packets and writes them in one go on
/// [`Transport::flush`], closed by `FRAME_END`, so the client can apply a
/// server frame as a whole; the client's end sends each packet at once.
#[derive(Debug)]
pub struct PacketStream {
    stream: FramedStream,
    /// The server end's packets of the frame not flushed yet.
    batch: Option<Vec<u8>>,
}

impl PacketStream {
    /// The server's end of a game stream.
    pub fn server(stream: FramedStream) -> Self {
        Self {
            stream,
            batch: Some(Vec::new()),
        }
    }

    /// A client's end of a game stream.
    pub fn client(stream: FramedStream) -> Self {
        Self {
            stream,
            batch: None,
        }
    }

    /// The stream underneath.
    pub fn stream_mut(&mut self) -> &mut FramedStream {
        &mut self.stream
    }

    /// Ends the game stream cleanly (see [`FramedStream::close`]).
    pub fn close(&mut self) {
        let _ = self.flush();
        self.stream.close();
    }
}

impl Transport for PacketStream {
    fn send(&mut self, packet: Packet) -> Result<(), NetError> {
        let body = packet_body(&packet);
        let Some(batch) = &mut self.batch else {
            return self.stream.send(body);
        };
        if body.len() > MAX_FRAME {
            return Err(NetError::Malformed);
        }
        batch.extend_from_slice(&frame(&body));
        if batch.len() >= BATCH_MAX {
            let bytes = std::mem::take(batch);
            return self.stream.send_bytes(bytes);
        }
        Ok(())
    }

    fn recv(&mut self) -> Result<Option<Packet>, NetError> {
        loop {
            let Some(body) = self.stream.recv()? else {
                return Ok(None);
            };
            // A frame's end only matters to a client that applies whole
            // frames; a packet reader skips it.
            if body == [kind::FRAME_END] {
                continue;
            }
            return match parse_body(&body)? {
                Body::Packet(p) => Ok(Some(p)),
                Body::Ping => continue,
                Body::Bye => Err(self.stream.end(NetError::Closed)),
            };
        }
    }

    fn flush(&mut self) -> Result<(), NetError> {
        let Some(batch) = &mut self.batch else {
            return Ok(());
        };
        batch.extend_from_slice(&frame(&[kind::FRAME_END]));
        let bytes = std::mem::take(batch);
        self.stream.send_bytes(bytes)
    }
}

// ------------------------------------------------------------ the client

/// A client's connection: its endpoint, the small runtime it runs on and
/// the connection. Dropping it closes the connection.
pub struct QuicClient {
    conn: quinn::Connection,
    endpoint: quinn::Endpoint,
    timing: Timing,
    /// Dropped last: the endpoint and the streams' tasks run on it.
    rt: Option<Runtime>,
}

impl std::fmt::Debug for QuicClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QuicClient")
            .field("peer", &self.conn.remote_address())
            .finish()
    }
}

impl QuicClient {
    /// Connects to the server at `addr`, checks its certificate against
    /// `trust` and opens the control stream. Waits at most
    /// `timing.timeout`. The server sees the control stream once the
    /// client sends on it (the handshake's first frame).
    pub fn connect(
        addr: SocketAddr,
        trust: ServerTrust,
        timing: Timing,
    ) -> Result<(Self, FramedStream), ConnectError> {
        let rt = client_runtime()?;
        let mut crypto = rustls::ClientConfig::builder_with_provider(provider())
            .with_protocol_versions(&[&rustls::version::TLS13])
            .map_err(|e| io_err(e.to_string()))?
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(TrustCheck {
                trust,
                provider: provider(),
            }))
            .with_no_client_auth();
        crypto.alpn_protocols = vec![ALPN.to_vec()];
        let quic = QuicClientConfig::try_from(crypto).map_err(|e| io_err(e.to_string()))?;
        let mut config = quinn::ClientConfig::new(Arc::new(quic));
        config.transport_config(transport_config(timing)?);
        let local: SocketAddr = if addr.is_ipv6() {
            "[::]:0".parse().expect("an address")
        } else {
            "0.0.0.0:0".parse().expect("an address")
        };
        let mut endpoint = {
            let _guard = rt.enter();
            quinn::Endpoint::client(local)?
        };
        endpoint.set_default_client_config(config);
        let opened = rt.block_on(async {
            tokio::time::timeout(timing.timeout, async {
                let conn = endpoint
                    .connect(addr, SERVER_NAME)
                    .map_err(|e| io_err(format!("connect: {e}")))?
                    .await
                    .map_err(|e| io_err(format!("connect: {e}")))?;
                let (send, recv) = conn
                    .open_bi()
                    .await
                    .map_err(|e| io_err(format!("open stream: {e}")))?;
                Ok::<_, ConnectError>((conn, send, recv))
            })
            .await
        });
        let (conn, send, recv) = match opened {
            Ok(r) => r?,
            Err(_) => return Err(io_err("the server did not answer in time")),
        };
        let control = start_stream(rt.handle(), conn.clone(), send, recv, true);
        Ok((
            Self {
                conn,
                endpoint,
                timing,
                rt: Some(rt),
            },
            control,
        ))
    }

    fn rt(&self) -> &Runtime {
        self.rt.as_ref().expect("the runtime lives until drop")
    }

    /// Opens another stream (the game stream). The server sees it once the
    /// client sends on it.
    pub fn open_stream(&self) -> Result<FramedStream, ConnectError> {
        let conn = self.conn.clone();
        let opened = self
            .rt()
            .block_on(async { tokio::time::timeout(self.timing.timeout, conn.open_bi()).await });
        let (send, recv) = match opened {
            Ok(r) => r.map_err(|e| io_err(format!("open stream: {e}")))?,
            Err(_) => return Err(io_err("the server did not take the stream in time")),
        };
        Ok(start_stream(
            self.rt().handle(),
            self.conn.clone(),
            send,
            recv,
            true,
        ))
    }

    /// The server's address.
    pub fn peer(&self) -> SocketAddr {
        self.conn.remote_address()
    }

    /// Why the connection ended, once it has (`None` while it is open).
    pub fn closed_reason(&self) -> Option<NetError> {
        self.conn.close_reason().map(|r| match r {
            quinn::ConnectionError::TimedOut => NetError::TimedOut,
            _ => NetError::Closed,
        })
    }

    /// Moves the connection to a new local socket bound at `local`, as a
    /// change of network does (Wi-Fi to mobile data). The connection
    /// carries on; the server follows it.
    pub fn rebind(&self, local: SocketAddr) -> Result<(), ConnectError> {
        let socket = UdpSocket::bind(local)?;
        // The runtime polls the socket: it must not block.
        socket.set_nonblocking(true)?;
        let _guard = self.rt().enter();
        self.endpoint.rebind(socket)?;
        Ok(())
    }

    /// Closes the connection and every stream on it, waiting (briefly)
    /// until the server has been told.
    pub fn close(&mut self) {
        self.conn.close(0u32.into(), b"bye");
        let ep = self.endpoint.clone();
        let _ = self
            .rt()
            .block_on(async { tokio::time::timeout(CLOSE_WAIT, ep.wait_idle()).await });
    }

    /// Drops the connection without a word, as a machine that loses the
    /// network does (tests of timeouts and reconnects).
    #[doc(hidden)]
    pub fn vanish(mut self) {
        if let Some(rt) = self.rt.take() {
            rt.shutdown_background();
        }
    }
}

impl Drop for QuicClient {
    fn drop(&mut self) {
        if self.rt.is_some() {
            self.close();
        }
    }
}

// ------------------------------------------------------------ the server

/// A listening server endpoint that hands out connections with their
/// control streams. It runs on the server's own tokio runtime.
pub struct QuicServer {
    endpoint: quinn::Endpoint,
    arrivals: tokio::sync::Mutex<mpsc::UnboundedReceiver<(QuicConn, FramedStream)>>,
    identity: Identity,
}

impl std::fmt::Debug for QuicServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QuicServer")
            .field("addr", &self.endpoint.local_addr().ok())
            .finish()
    }
}

/// One accepted connection.
#[derive(Clone)]
pub struct QuicConn {
    conn: quinn::Connection,
    rt: Handle,
}

impl std::fmt::Debug for QuicConn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QuicConn")
            .field("peer", &self.conn.remote_address())
            .finish()
    }
}

/// Waits for a connection's control stream, then hands both to the server.
async fn arrive(
    incoming: quinn::Incoming,
    timing: Timing,
    rt: Handle,
    to: mpsc::UnboundedSender<(QuicConn, FramedStream)>,
) {
    let opened = tokio::time::timeout(timing.timeout, async {
        let conn = incoming.await.ok()?;
        let (send, recv) = conn.accept_bi().await.ok()?;
        Some((conn, send, recv))
    })
    .await;
    if let Ok(Some((conn, send, recv))) = opened {
        let control = start_stream(&rt, conn.clone(), send, recv, false);
        let _ = to.send((QuicConn { conn, rt }, control));
    }
}

impl QuicServer {
    /// Listens on `addr` (port 0 picks a free port), presenting
    /// `identity`, on the runtime `rt`.
    pub fn bind(
        addr: SocketAddr,
        identity: Identity,
        timing: Timing,
        rt: Handle,
    ) -> Result<Self, ConnectError> {
        let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(identity.key.clone()));
        let mut crypto = rustls::ServerConfig::builder_with_provider(provider())
            .with_protocol_versions(&[&rustls::version::TLS13])
            .map_err(|e| io_err(e.to_string()))?
            .with_no_client_auth()
            .with_single_cert(vec![CertificateDer::from(identity.cert.clone())], key)
            .map_err(|e| io_err(format!("certificate: {e}")))?;
        crypto.alpn_protocols = vec![ALPN.to_vec()];
        let quic = QuicServerConfig::try_from(crypto).map_err(|e| io_err(e.to_string()))?;
        let mut config = quinn::ServerConfig::with_crypto(Arc::new(quic));
        config.transport_config(transport_config(timing)?);
        config.migration(true);
        let endpoint = {
            let _guard = rt.enter();
            quinn::Endpoint::server(config, addr)?
        };
        let (arr_tx, arr_rx) = mpsc::unbounded_channel();
        let ep = endpoint.clone();
        let handle = rt.clone();
        rt.spawn(async move {
            while let Some(incoming) = ep.accept().await {
                // Address validation: a first attempt proves its address
                // before any handshake work (`server::answer`).
                let validated = incoming.remote_address_validated();
                if server::answer(validated, incoming.may_retry()) == server::Answer::Retry {
                    let _ = incoming.retry();
                    continue;
                }
                tokio::spawn(arrive(incoming, timing, handle.clone(), arr_tx.clone()));
            }
        });
        Ok(Self {
            endpoint,
            arrivals: tokio::sync::Mutex::new(arr_rx),
            identity,
        })
    }

    /// The address it listens on.
    pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.endpoint.local_addr()
    }

    /// The certificate it presents (DER); clients pin its [`fingerprint`].
    pub fn cert(&self) -> &[u8] {
        &self.identity.cert
    }

    /// The next connection whose control stream has opened. `None` once
    /// the server is closed.
    pub async fn accept(&self) -> Option<(QuicConn, FramedStream)> {
        self.arrivals.lock().await.recv().await
    }

    /// [`QuicServer::accept`] without waiting: `None` when nobody is waiting.
    pub fn try_accept(&self) -> Option<(QuicConn, FramedStream)> {
        self.arrivals.try_lock().ok()?.try_recv().ok()
    }

    /// Stops listening and closes every connection.
    pub fn close(&self) {
        self.endpoint.close(0u32.into(), b"server closed");
    }
}

impl Drop for QuicServer {
    fn drop(&mut self) {
        self.close();
    }
}

impl QuicConn {
    /// The client's address as the server sees it now (it changes when a
    /// client migrates).
    pub fn peer(&self) -> SocketAddr {
        self.conn.remote_address()
    }

    /// The next stream the client opens (its game stream). `None` once the
    /// connection is gone.
    pub async fn accept_stream(&self) -> Option<FramedStream> {
        let (send, recv) = self.conn.accept_bi().await.ok()?;
        Some(start_stream(&self.rt, self.conn.clone(), send, recv, false))
    }

    /// Closes the connection with an application `code` and a short reason.
    pub fn close(&self, code: u32, reason: &str) {
        self.conn.close(code.into(), reason.as_bytes());
    }
}

pub mod server;

#[cfg(test)]
mod tests;
