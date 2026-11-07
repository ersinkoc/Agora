//! WebSocket transport for the framed protocol (ADR-0011), shared by the hub and the node's uplink. One binary WebSocket message carries exactly one
//! encoded [`Frame`]; the framing layer itself does not depend on WebSocket (so QUIC and SSH stdio can carry the
//! same frames later). TLS wraps the stream before it gets here: any `AsyncRead + AsyncWrite` works.

use std::fmt;

use crate::frame::DEFAULT_MAX_PAYLOAD;
use crate::{Decoder, Frame};
use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;

/// Headroom over the payload limit for the frame header.
const HEADER_ROOM: usize = 32;

#[derive(Debug)]
pub enum TransportError {
    /// The WebSocket layer failed (I/O, handshake, oversized message).
    Ws(String),
    /// The peer sent something that is not exactly one valid frame.
    BadFrame,
}

impl fmt::Display for TransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ws(e) => write!(f, "websocket: {e}"),
            Self::BadFrame => write!(f, "peer sent a malformed frame"),
        }
    }
}

impl std::error::Error for TransportError {}

fn ws_err(e: impl fmt::Display) -> TransportError {
    TransportError::Ws(e.to_string())
}

fn config() -> WebSocketConfig {
    WebSocketConfig {
        max_message_size: Some(DEFAULT_MAX_PAYLOAD + HEADER_ROOM),
        max_frame_size: Some(DEFAULT_MAX_PAYLOAD + HEADER_ROOM),
        ..WebSocketConfig::default()
    }
}

/// A connection carrying frames.
pub struct Conn<S> {
    ws: WebSocketStream<S>,
}

/// Server side: perform the WebSocket upgrade on an accepted (and, in production, TLS-wrapped) stream.
pub async fn accept<S>(stream: S) -> Result<Conn<S>, TransportError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let ws = tokio_tungstenite::accept_async_with_config(stream, Some(config()))
        .await
        .map_err(ws_err)?;
    Ok(Conn { ws })
}

/// Server side that also returns the bearer token the peer presented in `Authorization: Bearer <token>`, if any
/// (clients authenticate this way; nodes authenticate with their TLS certificate).
#[allow(clippy::result_large_err)] // the error type of the callback is dictated by tungstenite
pub async fn accept_with_token<S>(stream: S) -> Result<(Conn<S>, Option<String>), TransportError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
    let mut token = None;
    let ws = tokio_tungstenite::accept_hdr_async_with_config(
        stream,
        |req: &Request, resp: Response| {
            token = req
                .headers()
                .get("authorization")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.strip_prefix("Bearer "))
                .map(str::to_string);
            Ok(resp)
        },
        Some(config()),
    )
    .await
    .map_err(ws_err)?;
    Ok((Conn { ws }, token))
}

/// Client side presenting a bearer token (what a client of the hub does after signing in).
pub async fn connect_with_token<S>(
    stream: S,
    url: &str,
    token: &str,
) -> Result<Conn<S>, TransportError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let mut req = url.into_client_request().map_err(ws_err)?;
    req.headers_mut().insert(
        "authorization",
        format!("Bearer {token}").parse().map_err(ws_err)?,
    );
    let (ws, _) = tokio_tungstenite::client_async_with_config(req, stream, Some(config()))
        .await
        .map_err(ws_err)?;
    Ok(Conn { ws })
}

/// Client side (used by tests here; the node uplink has its own dialer).
pub async fn connect<S>(stream: S, url: &str) -> Result<Conn<S>, TransportError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let (ws, _) = tokio_tungstenite::client_async_with_config(url, stream, Some(config()))
        .await
        .map_err(ws_err)?;
    Ok(Conn { ws })
}

impl<S> Conn<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    /// Next frame, or `None` when the peer closed the connection.
    pub async fn recv(&mut self) -> Result<Option<Frame>, TransportError> {
        loop {
            match self.ws.next().await {
                None | Some(Ok(Message::Close(_))) => return Ok(None),
                Some(Err(e)) => return Err(ws_err(e)),
                Some(Ok(Message::Binary(bytes))) => {
                    let mut dec = Decoder::new(DEFAULT_MAX_PAYLOAD);
                    dec.push(&bytes);
                    let frame = dec
                        .next_frame()
                        .map_err(|_| TransportError::BadFrame)?
                        .ok_or(TransportError::BadFrame)?;
                    if dec.buffered() != 0 {
                        return Err(TransportError::BadFrame); // trailing bytes: not exactly one frame
                    }
                    return Ok(Some(frame));
                }
                // Text is not part of the protocol; ping/pong are answered by the library.
                Some(Ok(Message::Text(_))) => return Err(TransportError::BadFrame),
                Some(Ok(_)) => {}
            }
        }
    }

    pub async fn send(&mut self, frame: &Frame) -> Result<(), TransportError> {
        let bytes = frame
            .encode(DEFAULT_MAX_PAYLOAD)
            .map_err(|_| TransportError::BadFrame)?;
        self.ws.send(Message::Binary(bytes)).await.map_err(ws_err)
    }

    pub async fn close(&mut self) {
        let _ = self.ws.close(None).await;
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use tokio::net::{TcpListener, TcpStream};

    async fn pair() -> (Conn<TcpStream>, Conn<TcpStream>) {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (s, _) = l.accept().await.unwrap();
            accept(s).await.unwrap()
        });
        let c = TcpStream::connect(addr).await.unwrap();
        let client = connect(c, &format!("ws://{addr}/")).await.unwrap();
        (server.await.unwrap(), client)
    }

    fn frame(stream_id: u32, payload: &[u8]) -> Frame {
        Frame {
            stream_id,
            kind: 0,
            seq: Some(9),
            payload: payload.to_vec(),
        }
    }

    #[tokio::test]
    async fn frames_round_trip_in_both_directions() {
        let (mut s, mut c) = pair().await;
        c.send(&frame(2, b"up")).await.unwrap();
        assert_eq!(s.recv().await.unwrap().unwrap(), frame(2, b"up"));
        s.send(&frame(0, b"down")).await.unwrap();
        assert_eq!(c.recv().await.unwrap().unwrap(), frame(0, b"down"));
    }

    #[tokio::test]
    async fn order_is_preserved() {
        let (mut s, mut c) = pair().await;
        for i in 0..50u8 {
            c.send(&frame(2, &[i])).await.unwrap();
        }
        for i in 0..50u8 {
            assert_eq!(s.recv().await.unwrap().unwrap().payload, [i]);
        }
    }

    #[tokio::test]
    async fn close_ends_the_stream() {
        let (mut s, mut c) = pair().await;
        c.close().await;
        assert!(s.recv().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn a_message_with_two_frames_or_garbage_is_rejected() {
        let (mut s, mut c) = pair().await;
        let mut two = frame(1, b"a").encode(1024).unwrap();
        two.extend(frame(1, b"b").encode(1024).unwrap());
        c.ws.send(Message::Binary(two)).await.unwrap();
        assert!(matches!(s.recv().await, Err(TransportError::BadFrame)));
        c.ws.send(Message::Binary(vec![0xff; 4])).await.unwrap();
        assert!(matches!(s.recv().await, Err(TransportError::BadFrame)));
        c.ws.send(Message::Text("hi".into())).await.unwrap();
        assert!(matches!(s.recv().await, Err(TransportError::BadFrame)));
    }

    #[tokio::test]
    async fn oversized_message_is_refused_before_buffering_it() {
        let (mut s, mut c) = pair().await;
        // The sender and the refusing receiver must run together: a receiver that stops reading while the sender
        // is still writing a megabyte blocks the sender once the socket buffers fill (small on macOS). Dropping
        // the receiver after the error is what a real server does when it closes the connection.
        let send = async {
            let _ =
                c.ws.send(Message::Binary(vec![0; DEFAULT_MAX_PAYLOAD + 1024]))
                    .await;
        };
        let recv = async {
            let r = s.recv().await;
            drop(s);
            r
        };
        let (_, r) = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            tokio::join!(send, recv)
        })
        .await
        .expect("must not hang");
        assert!(r.is_err());
    }
}
