//! The control server itself: a Windows named pipe, ACL'd to the current
//! user, gated by the per-session token, speaking newline-delimited JSON.
//!
//! Never a TCP or remote-debugging port - see `SECURITY.md`. A local
//! process still has to (a) know the pipe name, which is fixed and public,
//! and (b) read the token file, which is locked to this Windows account, so
//! only something already running as Daniel can talk to it.

use std::ffi::c_void;
use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::windows::named_pipe::{NamedPipeServer, PipeMode, ServerOptions};

use super::super::dispatch::ToolDispatcher;
use super::super::protocol::{Handshake, Request, Response};
use super::super::token::SessionToken;
use super::acl::{current_user_only_sddl, SecurityDescriptorGuard};

pub const PIPE_NAME: &str = r"\\.\pipe\blueflame-control";

/// The longest a single line (handshake or request) we'll buffer from an
/// unauthenticated caller before giving up. Generous for a JSON tool call
/// with a `type` payload, small enough that a caller can't use it to make
/// us allocate without bound.
const MAX_LINE_BYTES: usize = 4 * 1024 * 1024;

/// Accept connections forever, one `NamedPipeServer` instance per client.
/// Each connection gets its own task so a slow or hung caller (there should
/// only ever be one - the local MCP bridge - but nothing stops Daniel from
/// running two) can't block the others.
pub async fn serve(token: SessionToken, dispatcher: Arc<ToolDispatcher>) -> anyhow::Result<()> {
    let sddl = current_user_only_sddl()?;
    let mut first = true;
    loop {
        let guard = SecurityDescriptorGuard::from_sddl(&sddl)?;
        let mut options = ServerOptions::new();
        options
            .pipe_mode(PipeMode::Byte)
            .reject_remote_clients(true)
            .first_pipe_instance(first);
        first = false;

        // SAFETY: `guard` owns a valid, fully-initialized SECURITY_ATTRIBUTES
        // for the lifetime of this call; CreateNamedPipeW (which this wraps)
        // only reads it synchronously before returning.
        let server: NamedPipeServer = unsafe {
            options.create_with_security_attributes_raw(PIPE_NAME, guard.as_raw() as *mut c_void)?
        };
        drop(guard);

        server.connect().await?;

        let dispatcher = dispatcher.clone();
        let token = token.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_connection(server, token, dispatcher).await {
                tracing::warn!(error = %e, "control pipe connection ended with an error");
            }
        });
    }
}

/// The outcome of checking a caller's first line against the session token.
/// Kept separate from the actual I/O so the token/JSON logic is unit
/// testable without a real pipe (see the `tests` module below).
#[derive(Debug, PartialEq, Eq)]
enum HandshakeOutcome {
    Accepted,
    NotJson,
    WrongToken,
}

fn check_handshake(line: &str, token: &SessionToken) -> HandshakeOutcome {
    match serde_json::from_str::<Handshake>(line.trim_end()) {
        Err(_) => HandshakeOutcome::NotJson,
        Ok(h) if token.matches(&h.token) => HandshakeOutcome::Accepted,
        Ok(_) => HandshakeOutcome::WrongToken,
    }
}

async fn handle_connection(
    server: NamedPipeServer,
    token: SessionToken,
    dispatcher: Arc<ToolDispatcher>,
) -> anyhow::Result<()> {
    let (read_half, mut write_half) = tokio::io::split(server);
    let mut reader = BufReader::new(read_half);

    let mut first_line = String::new();
    let n = read_line_capped(&mut reader, &mut first_line).await?;
    if n == 0 {
        return Ok(()); // client disconnected before sending anything
    }
    match check_handshake(&first_line, &token) {
        HandshakeOutcome::NotJson => {
            write_line(
                &mut write_half,
                br#"{"ok":false,"error":"expected a token handshake first"}"#,
            )
            .await?;
            return Ok(());
        }
        HandshakeOutcome::WrongToken => {
            tracing::warn!("control pipe: rejected connection with an invalid token");
            write_line(&mut write_half, br#"{"ok":false,"error":"invalid token"}"#).await?;
            return Ok(());
        }
        HandshakeOutcome::Accepted => {}
    }
    write_line(&mut write_half, br#"{"ok":true,"result":"ready"}"#).await?;

    loop {
        let mut line = String::new();
        let n = read_line_capped(&mut reader, &mut line).await?;
        if n == 0 {
            return Ok(()); // clean disconnect
        }
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Request>(trimmed) {
            Ok(req) => {
                let result = dispatcher.dispatch(&req.tool, req.args).await;
                match result {
                    Ok(value) => Response::ok(req.id, value),
                    Err(e) => Response::err(req.id, e),
                }
            }
            Err(e) => Response::err(0, format!("invalid request: {e}")),
        };
        let body = serde_json::to_string(&response).unwrap_or_else(|_| {
            r#"{"id":0,"ok":false,"error":"failed to encode response"}"#.to_string()
        });
        write_half.write_all(body.as_bytes()).await?;
        write_half.write_all(b"\n").await?;
        write_half.flush().await?;
    }
}

async fn read_line_capped<R: tokio::io::AsyncBufRead + Unpin>(
    reader: &mut R,
    out: &mut String,
) -> std::io::Result<usize> {
    let n = reader.read_line(out).await?;
    if out.len() > MAX_LINE_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "line too long",
        ));
    }
    Ok(n)
}

async fn write_line<W: tokio::io::AsyncWrite + Unpin>(
    writer: &mut W,
    body: &[u8],
) -> std::io::Result<()> {
    writer.write_all(body).await?;
    writer.write_all(b"\n").await?;
    writer.flush().await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token() -> SessionToken {
        SessionToken::from_str_for_test("abc123")
    }

    #[test]
    fn accepts_matching_token() {
        let line = r#"{"token":"abc123"}"#;
        assert_eq!(check_handshake(line, &token()), HandshakeOutcome::Accepted);
    }

    #[test]
    fn rejects_wrong_token() {
        let line = r#"{"token":"wrong"}"#;
        assert_eq!(
            check_handshake(line, &token()),
            HandshakeOutcome::WrongToken
        );
    }

    #[test]
    fn rejects_empty_token() {
        let line = r#"{"token":""}"#;
        assert_eq!(
            check_handshake(line, &token()),
            HandshakeOutcome::WrongToken
        );
    }

    #[test]
    fn rejects_malformed_json() {
        assert_eq!(
            check_handshake("not json", &token()),
            HandshakeOutcome::NotJson
        );
        assert_eq!(check_handshake("{}", &token()), HandshakeOutcome::NotJson);
        assert_eq!(
            check_handshake(r#"{"nottoken":"abc123"}"#, &token()),
            HandshakeOutcome::NotJson
        );
    }

    #[test]
    fn tolerates_trailing_newline_from_read_line() {
        // read_line_capped leaves the trailing '\n' in the buffer; check_handshake
        // trims it itself so callers don't have to remember to.
        let line = "{\"token\":\"abc123\"}\n";
        assert_eq!(check_handshake(line, &token()), HandshakeOutcome::Accepted);
    }

    #[tokio::test]
    async fn read_line_capped_reads_a_full_line() {
        let mock = tokio_test::io::Builder::new()
            .read(b"{\"token\":\"abc123\"}\n")
            .build();
        let mut reader = BufReader::new(mock);
        let mut out = String::new();
        let n = read_line_capped(&mut reader, &mut out).await.unwrap();
        assert_eq!(n, out.len());
        assert_eq!(out, "{\"token\":\"abc123\"}\n");
    }

    #[tokio::test]
    async fn read_line_capped_returns_zero_on_immediate_eof() {
        let mock = tokio_test::io::Builder::new().build();
        let mut reader = BufReader::new(mock);
        let mut out = String::new();
        let n = read_line_capped(&mut reader, &mut out).await.unwrap();
        assert_eq!(n, 0);
    }

    #[tokio::test]
    async fn read_line_capped_rejects_a_line_over_the_cap() {
        let too_long = vec![b'a'; MAX_LINE_BYTES + 1];
        let mock = tokio_test::io::Builder::new().read(&too_long).build();
        let mut reader = BufReader::new(mock);
        let mut out = String::new();
        let err = read_line_capped(&mut reader, &mut out).await.unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    }
}
